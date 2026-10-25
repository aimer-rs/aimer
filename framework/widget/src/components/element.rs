use std::any::TypeId;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU64;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_cupid::damage_region::{DamageRect, DamageSet};
use aimer_cupid::draw_cmd_v2::{RenderNodeId, RenderPaintSource, RenderTree};
use aimer_cupid::utilities::{IdBuildHasher, Rect};
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
#[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
use aimer_events::window::request_animation_frame;
use aimer_focus::{FocusCandidate, FocusCandidates, FocusManager, FocusNode, FocusTrapId};
use aimer_rubick::{ErasedFrom, UiAllocator};
use hashbrown::{HashMap, HashSet};
use smallvec::SmallVec;

use crate::base::*;
use crate::components::event_element::{
    CaptureRequest, EventElement, EventResult, EventTreeRole, FollowUp, PointerKey,
};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
pub(crate) use crate::components::visitor_element::VisitorElement;
use crate::element_invalidation::{
    ElementChangeKind, ElementInvalidationBatch, ElementInvalidationBounds,
    ElementInvalidationRevisions,
};
use crate::components::drawable::{
    CompositorAnimationDecision, CompositorAnimationFrame,
};
#[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
use crate::components::drawable::CompositorTransform;
use crate::pointer_claim;
use crate::{Drawable, Key};

mod event;
mod live_children;
mod scoped_rebuild;
#[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
mod animation_only;
#[cfg(any(target_arch = "wasm32", feature = "portable-guest"))]
#[path = "element/animation_only_stub.rs"]
mod animation_only;
mod compositor_applied;
mod node;
#[cfg(test)]
mod paint_invalidation_tests;
#[cfg(test)]
mod keep_reachable_tests;
#[cfg(test)]
mod frame_instant_tests;
#[cfg(test)]
mod render_context_tests;
#[cfg(test)]
mod window_paint_state_tests;
mod retained_window;

pub use animation_only::{
    animation_only_targets, is_animation_only_frame, request_isolated_animation_frame,
    request_isolated_subtree_animation_frame, run_animation_only_pass, set_frame_rebuilt_roots, set_animation_only_frame, traversal_counts,
    with_v2_animation_only_context,
};
use animation_only::{
    begin_full_traversal, capture_rebuilt_root, enter_subtree_walk, finish_full_traversal, is_animation_only_pass,
    pass_skips_subtree, register_animating_element,
};
use compositor_applied::{
    compositor_animation_may_be_applied, forget_unmapped_compositor_animations,
    note_compositor_animation_applied, note_compositor_animation_cleared,
};
pub use live_children::{live_children_generation, notify_live_children_changed};
pub use scoped_rebuild::{
    ScopedCursor, has_scoped_rebuilds_since, scoped_rebuild_cursor, scoped_rebuilds_since,
    unscoped_element_tree_generation,
};
use scoped_rebuild::{note_scoped_rebuild, note_unscoped_tree_change};
pub(crate) use scoped_rebuild::rename_scoped_rebuilds_since;
use node::ElementNode;

mod any_element;
pub use any_element::AnyElement;
pub use retained_window::{
    may_escape_ancestor_clips,
    VisibleWindow, may_skip_settled_offscreen, retained_element_settled, retained_visible_window,
    retained_visible_window_of_current, set_scroll_only_frame, set_settled_offscreen_skip,
};
pub use event::{
    ElementPath, EventDispatchContext, EventDispatcher, broadcast_event, dispatch_event,
    dispatch_focused_event, focus_is_settling_between_events,
};

static NEXT_ELEMENT_ID: AtomicU64 = AtomicU64::new(1);
static ELEMENT_TREE_GENERATION: AtomicU64 = AtomicU64::new(0);
static RETAINED_RENDER_STRUCTURE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Local paint invalidations kept for elements that have not been drawn yet.
const MAX_PENDING_LOCAL_PAINT_INVALIDATIONS: usize = 1 << 16;

static REBUILD_INVALIDATION_GENERATION: AtomicU64 = AtomicU64::new(0);
static LAYOUT_INVALIDATION_GENERATION: AtomicU64 = AtomicU64::new(0);
/// Advances of the rebuild generation that only kept an element reachable.
static REBUILD_KEEPALIVE_COUNT: AtomicU64 = AtomicU64::new(0);
/// Advances of the rebuild generation made by `set_state` marking an element
/// dirty.
static REBUILD_STATE_MARK_COUNT: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_ELEMENT_TREE_GENERATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// Active per-window v2 recording context. It is installed only while the
    /// UI thread traverses the retained element tree for one frame.
    static V2_RENDER_CONTEXT_STACK: RefCell<Vec<V2RenderContext>> = const { RefCell::new(Vec::new()) };

    /// Elements that ran `ElementNode::draw` under retained presentation without
    /// a render node. A frame loop drains this after its final structure sync:
    /// an element still unmapped then painted into nothing, because the render
    /// tree never learned of it (typically its parent draws it without
    /// exposing it through `visit_children`).
    static UNMAPPED_DRAWS: RefCell<Vec<(ElementId, &'static str)>> = const { RefCell::new(Vec::new()) };
    #[cfg(debug_assertions)]
    static DECLINED_PAINT_ELEMENTS: RefCell<Vec<(ElementId, &'static str)>> = const { RefCell::new(Vec::new()) };

    /// Nested `mark_needs_rebuild` calls share one invalidation generation.
    /// Marking a large tree is already recursive; it must not also perform one
    /// atomic increment per descendant.
    static REBUILD_INVALIDATION_DEPTH: Cell<usize> = const { Cell::new(0) };

    /// Revisions are kept outside [`ElementNode`] so adding stable-layout
    /// tracking does not change the inline-erased element size. Only elements
    /// that opt into generation-independent sizing create entries here.
    static STABLE_SUBTREE_GENERATIONS: RefCell<HashMap<ElementId, u64, IdBuildHasher>> =
        RefCell::new(HashMap::default());

    /// Which elements had their stable generation set, in order, so a container
    /// can learn which of its children changed without asking each of them.
    static STABLE_GENERATION_LOG: RefCell<StableGenerationLog> =
        const { RefCell::new(StableGenerationLog { first: 0, ids: Vec::new() }) };

    /// Number of dirty rebuild sources whose root-relative path crosses each
    /// retained element. Counts keep shared ancestors indexed until every
    /// dirty source below them has been rebuilt.
    static DIRTY_SUBTREE_COUNTS: RefCell<HashMap<ElementId, usize, IdBuildHasher>> =
        RefCell::new(HashMap::default());

    /// For each element, the children that dirty paths run through, so a
    /// container can reach them without entering its other children.
    static DIRTY_EDGES: RefCell<HashMap<ElementId, SmallVec<[DirtyEdge; 2]>, IdBuildHasher>> =
        RefCell::new(HashMap::default());

    /// Advances whenever a dirty path is added, so a pass that rebuilt a
    /// snapshot of the edges can tell that the index grew underneath it.
    static DIRTY_EDGE_VERSION: Cell<u64> = const { Cell::new(0) };

    /// Event dispatchers use this UI-thread epoch to coalesce generation checks
    /// across all nested dispatchers until the next completed frame.
    static EVENT_FRAME_EPOCH: Cell<Option<u64>> = const { Cell::new(None) };

    /// The path index is usable only after one complete walk of the retained
    /// tree without a structural replacement or an untracked invalidation.
    static DIRTY_PATHS_READY: Cell<bool> = const { Cell::new(false) };
    static DIRTY_INDEXED_ROOTS: RefCell<HashSet<ElementId>> = RefCell::new(HashSet::new());
    static DIRTY_PATHS_INVALIDATED_DURING_TRAVERSAL: Cell<bool> = const { Cell::new(false) };
    static REBUILD_TRAVERSAL_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// The root of the outermost rebuild traversal in progress, if any.
    static REBUILD_TRAVERSAL_ROOT: Cell<Option<ElementId>> = const { Cell::new(None) };
    static REBUILD_DESCENTS: Cell<u64> = const { Cell::new(0) };
    static REBUILD_VISITS: Cell<u64> = const { Cell::new(0) };
    static STRUCTURAL_SEARCHES: Cell<u64> = const { Cell::new(0) };
    /// Non-zero while a caller that rebases the paths of the subtree it
    /// replaces is committing that replacement.
    static PRESERVE_DIRTY_PATHS_DEPTH: Cell<usize> = const { Cell::new(0) };
    static REBUILD_PATH: RefCell<Vec<ElementId>> = const { RefCell::new(Vec::new()) };
    static REBUILD_FORCE_DESCEND_DEPTH: Cell<usize> = const { Cell::new(0) };
    static REBUILD_MARK_DEPTH: Cell<usize> = const { Cell::new(0) };
    static PLACEMENT_MARK_DEPTH: Cell<usize> = const { Cell::new(0) };
    static PLACEMENT_SKIP_SUSPENDED: Cell<usize> = const { Cell::new(0) };
    static PLACEMENT_MARKS_SUSPENDED: Cell<usize> = const { Cell::new(0) };
    static DRAW_DEPTH: Cell<usize> = const { Cell::new(0) };
    static DRAW_INVALIDATION_OWNER: Cell<Option<ElementId>> = const { Cell::new(None) };
    /// Paint invalidation metadata is retained for the current pair of
    /// rebuild/tree generations so a retained scroll tile can distinguish a
    /// local dirty element from an unrelated branch's rebuild.
    static PAINT_INVALIDATED_LOCAL_ELEMENTS: RefCell<HashSet<ElementId>> = RefCell::new(HashSet::new());
    /// Visual event changes invalidate one element's retained list for the
    /// next collection, then are consumed without dirtying later frames.
    static EVENT_PAINT_INVALIDATIONS: RefCell<HashSet<ElementId>> = RefCell::new(HashSet::new());
    /// Damage accumulated by retained paint owners during the current frame.
    /// The target is thread-local because widget trees and their canvases are
    /// single-threaded; the finished [`DamageSet`] is copied into the frame
    /// packet before presentation moves to another thread.
    static FRAME_PAINT_DAMAGE: RefCell<Option<DamageSet>> = const { RefCell::new(None) };
    /// Stateful paint owners that contributed a precise footprint this frame.
    /// An owner rectangle can cover an otherwise unbounded event-tree
    /// invalidation somewhere inside its bounded paint subtree.
    static FRAME_PAINT_DAMAGE_OWNERS: RefCell<HashSet<ElementId>> = RefCell::new(HashSet::new());
    /// A platform event can invalidate paint before the next frame opens its
    /// damage collector. Keep that request until [`begin_paint_frame`] consumes
    /// it instead of silently dropping the invalidation.
    static PENDING_PAINT_DAMAGE_FULL: Cell<bool> = const { Cell::new(false) };
    static FRAME_PAINT_TARGET: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// When the frame being drawn began; see [`frame_instant`].
    static FRAME_INSTANT: Cell<Option<aimer_utils::AnimInstant>> = const { Cell::new(None) };
    #[cfg(feature = "frame-stats")]
    static DRAW_TRAVERSAL_COUNT: Cell<u64> = const { Cell::new(0) };
    #[cfg(feature = "frame-stats")]
    static ROUTED_EVENT_VISIT_COUNT: Cell<u64> = const { Cell::new(0) };
    #[cfg(test)]
    static HOVER_MEMBERSHIP_CHECK_COUNT: Cell<u64> = const { Cell::new(0) };
}

/// Maps each element to its render node in the window's retained render tree.
///
/// It is consulted for every element on every draw, so it uses `hashbrown`'s
/// default hasher: the keys are process-local identifiers, and the standard
/// library's DoS-resistant SipHash costs far more than the lookup itself,
/// especially in unoptimized builds.
pub type ElementNodeMap = HashMap<ElementId, RenderNodeId>;

/// The retained render tree a traversal paints into, and how elements map to
/// its nodes.
///
/// Every element's `update` takes a copy of this, so a copy is one counted
/// reference to shared data, not one per field.
#[derive(Clone)]
pub(crate) struct V2RenderContext(Rc<V2RenderScope>);

pub(crate) struct V2RenderScope {
    pub(crate) tree: RenderTree,
    element_nodes: Rc<ElementNodeMap>,
    prepared_root: Cell<Option<ElementId>>,
}

impl std::ops::Deref for V2RenderContext {
    type Target = V2RenderScope;

    #[inline]
    fn deref(&self) -> &V2RenderScope {
        &self.0
    }
}

impl V2RenderScope {
    #[inline]
    pub(crate) fn node_for_element(&self, element: ElementId) -> Option<RenderNodeId> {
        self.element_nodes.get(&element).copied()
    }

    #[inline]
    pub(crate) fn take_prepared_root(&self, element: ElementId) -> bool {
        self.prepared_root.get() == Some(element) && self.prepared_root.take().is_some()
    }
}

struct V2RenderContextGuard;

impl Drop for V2RenderContextGuard {
    fn drop(&mut self) {
        V2_RENDER_CONTEXT_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Runs the element traversal with the retained tree as the paint source.
///
/// Elements that have a render node record their paint into it, and the
/// traversal drops whatever they also draw through the canvas while it still
/// visits children and updates state.
#[doc(hidden)]
pub fn with_v2_render_tree_context<R>(
    tree: RenderTree,
    element_nodes: Rc<ElementNodeMap>,
    prepared_root: Option<ElementId>,
    callback: impl FnOnce() -> R,
) -> R {
    forget_unmapped_compositor_animations(&element_nodes);
    begin_full_traversal();
    V2_RENDER_CONTEXT_STACK.with(|stack| {
        stack.borrow_mut().push(V2RenderContext(Rc::new(V2RenderScope {
            tree,
            element_nodes,
            prepared_root: Cell::new(prepared_root),
        })));
    });
    let _guard = V2RenderContextGuard;
    let result = callback();
    finish_full_traversal();
    result
}

pub(crate) fn current_v2_render_context() -> Option<V2RenderContext> {
    V2_RENDER_CONTEXT_STACK.with(|stack| stack.borrow().last().cloned())
}

/// Returns whether the current draw is using the window's retained v2 tree.
#[doc(hidden)]
pub fn has_active_v2_render_tree() -> bool {
    // Asked once per element per frame: looking is enough, copying is not.
    V2_RENDER_CONTEXT_STACK.with(|stack| !stack.borrow().is_empty())
}

/// Returns whether retained v2 paint is the current presentation source, which
/// it is whenever a render tree is active.
#[doc(hidden)]
pub fn has_active_v2_render_presentation() -> bool {
    has_active_v2_render_tree()
}

/// Updates one mapped element's local bounds and clip during retained drawing.
///
/// Viewports use this for scroll offsets that change independently of layout.
/// The tree damages the old and new clipped footprints as one geometry change.
#[doc(hidden)]
pub fn update_v2_render_node_geometry(
    element: ElementId,
    bounds: Rect,
    clip: Option<Rect>,
) -> bool {
    let Some(context) = current_v2_render_context() else {
        return false;
    };
    let Some(node) = context.node_for_element(element) else {
        return false;
    };
    context.tree.set_geometry(node, bounds, clip).is_ok()
}

/// Starts a new event-dispatch frame on the current UI thread.
///
/// [`EventDispatcher`] instances use the shared epoch to check their root's
/// subtree generation once and defer a path-index rebuild until the first
/// dispatch that needs it. The application frame loop should call this after a
/// frame finishes so platform and nested widget events share the next epoch.
pub fn begin_event_frame() {
    EVENT_FRAME_EPOCH.with(|epoch| {
        let next = epoch.get().unwrap_or(0).wrapping_add(1);
        epoch.set(Some(next));
    });
}

/// The paint state that waits between an event and the frame that answers it,
/// for one window.
///
/// A window records what changed — a full repaint, a hover that has to be drawn
/// — and its next frame acts on it. Those records live in thread-locals,
/// because a frame and the events before it run on one thread; with several
/// windows on that thread they would be one shared set, and the frame of one
/// window would spend what was recorded for another. A window therefore keeps
/// its own state here and swaps it onto the thread while it is the active one
/// (see [`Self::swap_with_thread`]).
///
/// The state also remembers the size of the target the window last drew, which
/// is how the first frame for a target is told from the ones after it.
#[doc(hidden)]
#[derive(Default)]
pub struct WindowPaintState {
    frame_damage: Option<DamageSet>,
    damage_owners: HashSet<ElementId>,
    pending_full: bool,
    target: Option<(u32, u32)>,
    event_invalidations: HashSet<ElementId>,
    queue: crate::element_invalidation::PendingRecords,
}

impl WindowPaintState {
    /// Exchanges this state with the one on the thread.
    ///
    /// Calling it twice puts both back where they were, so one call makes a
    /// window's state the thread's and the second hands the thread's back.
    pub fn swap_with_thread(&mut self) {
        FRAME_PAINT_DAMAGE
            .with(|damage| std::mem::swap(&mut *damage.borrow_mut(), &mut self.frame_damage));
        FRAME_PAINT_DAMAGE_OWNERS
            .with(|owners| std::mem::swap(&mut *owners.borrow_mut(), &mut self.damage_owners));
        PENDING_PAINT_DAMAGE_FULL.with(|pending| self.pending_full = pending.replace(self.pending_full));
        FRAME_PAINT_TARGET.with(|target| {
            let theirs = target.replace(self.target);
            self.target = theirs;
        });
        EVENT_PAINT_INVALIDATIONS.with(|invalidations| {
            std::mem::swap(&mut *invalidations.borrow_mut(), &mut self.event_invalidations)
        });
        crate::element_invalidation::swap_pending(&mut self.queue);
    }

    /// Moves what this state recorded into the thread's, leaving this empty.
    ///
    /// Whatever was recorded while no window was active has no window to wait
    /// for, so the window that becomes active next takes it: it is handed to
    /// the thread right after that window's own state is installed. A record
    /// that cannot say what it touched becomes a full repaint.
    pub fn absorb_into_thread(&mut self) {
        let mut full = std::mem::take(&mut self.pending_full);
        EVENT_PAINT_INVALIDATIONS.with(|invalidations| {
            invalidations
                .borrow_mut()
                .extend(self.event_invalidations.drain());
        });
        full |= crate::element_invalidation::absorb_pending(&mut self.queue);
        self.frame_damage = None;
        self.damage_owners.clear();
        if full {
            PENDING_PAINT_DAMAGE_FULL.with(|pending| pending.set(true));
        }
    }
}

/// Starts collecting damage for one device-sized frame target.
///
/// The first frame for a target is conservatively full. Subsequent frames can
/// remain empty when every retained paint owner replays unchanged content;
/// invalidation requested between frames is carried into the next collection
/// interval.
#[doc(hidden)]
pub fn begin_paint_frame(width: u32, height: u32) {
    FRAME_INSTANT.with(|instant| instant.set(Some(aimer_utils::AnimInstant::now())));
    FRAME_PAINT_DAMAGE_OWNERS.with(|owners| owners.borrow_mut().clear());
    let target_changed = FRAME_PAINT_TARGET.with(|target| {
        let changed = target.get() != Some((width, height));
        target.set(Some((width, height)));
        changed
    });
    let pending_full = PENDING_PAINT_DAMAGE_FULL.with(|pending| pending.replace(false));
    FRAME_PAINT_DAMAGE.with(|damage| {
        let mut next = DamageSet::new(width, height);
        if target_changed || pending_full {
            next.mark_full();
        }
        *damage.borrow_mut() = Some(next);
    });
}

/// The instant the frame being drawn began, or now between frames.
///
/// An element that samples a clock when it rebuilds is rebuilt more than once in
/// a frame whenever a pass enters it from several places, and each visit saw a
/// later time than the one before: a value that is a pure function of time moved
/// a little on every visit, so the element replaced its subtree each time and the
/// walk after it was the cost of all of them. Sampling this instead gives every
/// visit of one frame the same answer, which is what an animation controller's
/// `tick` asks for: once per frame.
#[doc(hidden)]
#[inline]
pub fn frame_instant() -> aimer_utils::AnimInstant {
    FRAME_INSTANT
        .with(Cell::get)
        .unwrap_or_else(aimer_utils::AnimInstant::now)
}

/// Takes the damage collected since [`begin_paint_frame`].
#[doc(hidden)]
pub fn take_paint_frame_damage(width: u32, height: u32) -> DamageSet {
    FRAME_INSTANT.with(|instant| instant.set(None));
    let damage = FRAME_PAINT_DAMAGE
        .with(|damage| damage.borrow_mut().take())
        .unwrap_or_else(|| DamageSet::full(width, height));
    EVENT_PAINT_INVALIDATIONS.with(|invalidations| invalidations.borrow_mut().clear());
    damage
}

/// Adds one conservative device-pixel footprint to the current frame.
#[doc(hidden)]
pub fn mark_paint_damage(rectangle: DamageRect) {
    if let Some(owner) = DRAW_INVALIDATION_OWNER.with(Cell::get) {
        FRAME_PAINT_DAMAGE_OWNERS.with(|owners| {
            owners.borrow_mut().insert(owner);
        });
        queue_element_invalidation_for(
            Some(owner),
            None,
            ElementChangeKind::Paint,
            Some(rectangle),
        );
    }
    FRAME_PAINT_DAMAGE.with(|damage| {
        if let Some(damage) = damage.borrow_mut().as_mut() {
            damage.add(rectangle);
        }
    });
}

/// Forces the current frame to repaint its complete target.
///
/// When called between frame collections, the request is retained for the next
/// call to [`begin_paint_frame`].
#[doc(hidden)]
pub fn mark_paint_damage_full() {
    if let Some(owner) = DRAW_INVALIDATION_OWNER.with(Cell::get) {
        queue_element_invalidation_for(
            Some(owner),
            None,
            ElementChangeKind::Unknown,
            None,
        );
    } else if FRAME_PAINT_DAMAGE.with(|damage| damage.borrow().is_none()) {
        // External readiness and platform changes can request a full repaint
        // without an element owner. Preserve that fact for the next frame's
        // invalidation batch as an explicit unknown record.
        queue_element_invalidation(None, ElementChangeKind::Unknown);
    }
    FRAME_PAINT_DAMAGE.with(|damage| {
        if let Some(damage) = damage.borrow_mut().as_mut() {
            damage.mark_full();
        } else {
            PENDING_PAINT_DAMAGE_FULL.with(|pending| pending.set(true));
        }
    });
}

fn current_event_frame() -> Option<u64> {
    EVENT_FRAME_EPOCH.with(Cell::get)
}

fn record_local_paint_invalidation_path(path: &[ElementId]) {
    if let Some(element) = path.last().copied() {
        PAINT_INVALIDATED_LOCAL_ELEMENTS.with(|elements| {
            elements.borrow_mut().insert(element);
        });
    }
}

fn queue_element_invalidation(
    path: Option<Rc<[ElementId]>>,
    change: ElementChangeKind,
) {
    let element_id = path.as_deref().and_then(|path| path.last().copied());
    queue_element_invalidation_for(element_id, path, change, None);
}

fn queue_element_invalidation_for(
    element_id: Option<ElementId>,
    path: Option<Rc<[ElementId]>>,
    change: ElementChangeKind,
    damage_hint: Option<DamageRect>,
) {
    let revisions = current_element_invalidation_revisions();
    let queued_during_frame = FRAME_PAINT_DAMAGE.with(|damage| damage.borrow().is_some());
    crate::element_invalidation::enqueue(
        element_id,
        path,
        change,
        revisions,
        queued_during_frame,
        damage_hint,
    );
}

fn record_current_paint_invalidation(element: ElementId, owns_paint: bool) {
    // Only an element that owns its paint has a local list to refresh; a
    // descendant's rebuild reaches its own paint owner through the dirty path.
    if !owns_paint {
        return;
    }
    REBUILD_PATH.with(|path| {
        let path = path.borrow();
        if path.is_empty() {
            record_local_paint_invalidation_path(&[element]);
        } else {
            record_local_paint_invalidation_path(&path);
        }
    });
}

fn current_element_invalidation_revisions() -> ElementInvalidationRevisions {
    ElementInvalidationRevisions {
        tree: element_tree_generation(),
        rebuild: rebuild_invalidation_generation(),
        layout: layout_invalidation_generation(),
        resource: None,
    }
}

fn element_invalidation_bounds(element: &dyn Element) -> Option<ElementInvalidationBounds> {
    let (start, end) = element.event_tree_bounds()?;
    if !start.x.is_finite()
        || !start.y.is_finite()
        || !end.x.is_finite()
        || !end.y.is_finite()
        || end.x < start.x
        || end.y < start.y
    {
        return None;
    }
    Some(ElementInvalidationBounds { start, end })
}

fn mark_paint_invalidations_unknown() {
    queue_element_invalidation(None, ElementChangeKind::Unknown);
}

/// Returns whether this element's own local retained paint was invalidated
/// since it last asked, and forgets the invalidation.
///
/// An invalidation lives until the element's paint is resolved, not until the
/// next rebuild. A rebuild pass reconciles many elements before any of them is
/// drawn, and each rebuild that marks a descendant advances the rebuild
/// generation; forgetting on that advance left only the last-reconciled
/// element's invalidation standing, so every other rebuilt element kept
/// painting its previous content.
#[doc(hidden)]
pub fn take_local_paint_invalidated(element: ElementId) -> bool {
    PAINT_INVALIDATED_LOCAL_ELEMENTS.with(|elements| {
        let mut elements = elements.borrow_mut();
        !elements.is_empty() && elements.remove(&element)
    })
}

/// Marks one reconciled element's retained paint as stale until it is resolved.
#[doc(hidden)]
pub(crate) fn mark_paint_element_invalidated(element: ElementId) {
    PAINT_INVALIDATED_LOCAL_ELEMENTS.with(|elements| {
        let mut elements = elements.borrow_mut();
        // An element that is removed before it is drawn never asks, so the set
        // is bounded rather than trusted to drain.
        if elements.len() >= MAX_PENDING_LOCAL_PAINT_INVALIDATIONS {
            elements.clear();
        }
        elements.insert(element);
    });
}

#[inline]
pub(crate) fn mark_event_paint_invalidated(element: ElementId) {
    EVENT_PAINT_INVALIDATIONS.with(|invalidations| {
        invalidations.borrow_mut().insert(element);
    });
}

#[inline]
pub(crate) fn take_event_paint_invalidated(element: ElementId) -> bool {
    EVENT_PAINT_INVALIDATIONS.with(|invalidations| {
        let mut invalidations = invalidations.borrow_mut();
        // Asked of every element every frame, and almost always empty: hashing
        // the element only to find that out is the whole cost.
        !invalidations.is_empty() && invalidations.remove(&element)
    })
}

/// Resets the draw traversal counter for the next measured frame.
#[cfg(feature = "frame-stats")]
pub fn reset_draw_traversal_count() {
    DRAW_TRAVERSAL_COUNT.with(|count| count.set(0));
}

/// Takes the number of retained element draw calls observed since the last
/// reset. A draw call is counted for every element reached by the drawable
/// traversal, including a scroll container whose children may be culled.
#[cfg(feature = "frame-stats")]
pub fn take_draw_traversal_count() -> u64 {
    DRAW_TRAVERSAL_COUNT.with(Cell::get)
}

/// Resets the routed-event visit counter for the next measured input sample.
#[cfg(feature = "frame-stats")]
#[doc(hidden)]
pub fn reset_routed_event_visit_count() {
    ROUTED_EVENT_VISIT_COUNT.with(|count| count.set(0));
}

/// Takes the number of elements reached by routed pointer dispatch since the
/// last reset. Cached hit-chain replay contributes one visit per delivered
/// element, so this counter measures the work visible to the event path rather
/// than only calls into the uncached recursive walker.
#[cfg(feature = "frame-stats")]
#[doc(hidden)]
pub fn take_routed_event_visit_count() -> u64 {
    ROUTED_EVENT_VISIT_COUNT.with(Cell::get)
}

#[inline]
fn record_routed_event_visit() {
    crate::frame_work_stats::record_hit_test_visit();
    #[cfg(feature = "frame-stats")]
    ROUTED_EVENT_VISIT_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[inline]
fn record_hover_membership_check() {
    #[cfg(test)]
    HOVER_MEMBERSHIP_CHECK_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn reset_hover_membership_check_count() {
    HOVER_MEMBERSHIP_CHECK_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn take_hover_membership_check_count() -> u64 {
    HOVER_MEMBERSHIP_CHECK_COUNT.with(Cell::get)
}

#[cfg(feature = "frame-stats")]
#[inline]
fn record_draw_traversal() {
    DRAW_TRAVERSAL_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

struct RebuildInvalidationGuard;

impl Drop for RebuildInvalidationGuard {
    fn drop(&mut self) {
        REBUILD_INVALIDATION_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

pub(crate) struct RebuildMarkGuard;

impl Drop for RebuildMarkGuard {
    fn drop(&mut self) {
        REBUILD_MARK_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

pub(crate) fn begin_rebuild_mark() -> (RebuildMarkGuard, bool) {
    let outermost = REBUILD_MARK_DEPTH.with(|depth| {
        let outermost = depth.get() == 0;
        depth.set(depth.get() + 1);
        outermost
    });
    (RebuildMarkGuard, outermost)
}

struct PlacementMarkGuard;

impl Drop for PlacementMarkGuard {
    fn drop(&mut self) {
        PLACEMENT_MARK_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// Runs `mark` as the mark a *placement* of a retained subtree makes.
///
/// A parent that rebuilds itself places its retained child again, and marks it
/// so that builds which observed inherited context run again. Only a build that
/// read such context can differ from its last run, so while this scope is open
/// a self-rebuilding element whose last build read none
/// ([`DirtySource::reads_context`]) leaves itself clean and only forwards the
/// mark to its child. Every other mark — a window resize, a theme change, an
/// explicit invalidation — is unaffected and stays conservative.
pub(crate) fn with_placement_mark<R>(mark: impl FnOnce() -> R) -> R {
    PLACEMENT_MARK_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = PlacementMarkGuard;
    mark()
}

/// Runs `build` with the marks of retained-child placements switched off: a
/// retained child placed again inside it is not marked at all.
///
/// A placement marks the child it places so that builds which observed inherited
/// context run again, because a parent that rebuilds itself is usually a parent
/// whose context changed. That does not hold for a parent that rebuilds only
/// because a value it animates moved: nothing it publishes has changed, and a
/// context that does change marks its dependents on its own. For such a parent
/// the mark would make every unchanged child run its builds on every frame, the
/// work handing it over as a retained child was meant to avoid.
#[doc(hidden)]
pub fn without_placement_marks<R>(build: impl FnOnce() -> R) -> R {
    struct Resume;
    impl Drop for Resume {
        fn drop(&mut self) {
            PLACEMENT_MARKS_SUSPENDED.with(|depth| depth.set(depth.get() - 1));
        }
    }
    PLACEMENT_MARKS_SUSPENDED.with(|depth| depth.set(depth.get() + 1));
    let _resume = Resume;
    build()
}

/// Whether [`without_placement_marks`] is in effect.
#[inline]
pub(crate) fn placement_marks_suspended() -> bool {
    PLACEMENT_MARKS_SUSPENDED.with(|depth| depth.get() != 0)
}

/// A number that names the chain of elements the rebuild pass is inside of.
fn rebuild_path_signature() -> u64 {
    REBUILD_PATH.with(|path| {
        path.borrow().iter().fold(0x9e37_79b9_7f4a_7c15u64, |hash, id| {
            (hash.rotate_left(5) ^ id.get()).wrapping_mul(0xf135_7aea_2e62_a9c5)
        })
    })
}

/// Rebuilds a retained `child` the pass has reached.
///
/// Inside a replaced subtree the pass descends everything, because nothing in it
/// has been visited and every element has to register its path. A retained child
/// was not replaced: it is the same element as last frame, and if the chain of
/// elements above it is the same too, the paths it registered then are still
/// right, so the descent has nothing to add and would only cost as much as the
/// child is large. `placed_at` remembers the chain the child was last descended
/// under.
pub(crate) fn rebuild_retained_child(child: &dyn Rebuildable, placed_at: &Cell<u64>, ctx: &BuildContext) {
    let forced = REBUILD_FORCE_DESCEND_DEPTH.with(Cell::get);
    if forced == 0 {
        child.rebuild_if_dirty(ctx);
        return;
    }
    let signature = rebuild_path_signature();
    if placed_at.get() == signature {
        REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(0));
        child.rebuild_if_dirty(ctx);
        REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(forced));
    } else {
        child.rebuild_if_dirty(ctx);
        placed_at.set(signature);
    }
}

/// Runs `build` with placement skipping suspended: every mark it makes is a full
/// one.
///
/// A placement may leave a stateful element clean because its last build read no
/// inherited context, which assumes the element is the one that built last time.
/// That does not hold while a fresh element adopts a live element's state: the
/// child it places then is new content, built at construction and never since,
/// and it has to be rebuilt to show the adopted state. Reconciliation wraps its
/// adoption-time rebuild in this.
pub(crate) fn without_placement_skip<R>(build: impl FnOnce() -> R) -> R {
    struct Resume;
    impl Drop for Resume {
        fn drop(&mut self) {
            PLACEMENT_SKIP_SUSPENDED.with(|depth| depth.set(depth.get() - 1));
        }
    }
    PLACEMENT_SKIP_SUSPENDED.with(|depth| depth.set(depth.get() + 1));
    let _resume = Resume;
    build()
}

/// Returns whether the mark in progress is the one a placement makes.
#[inline]
pub(crate) fn placement_mark_active() -> bool {
    PLACEMENT_SKIP_SUSPENDED.with(|depth| depth.get() == 0)
        && PLACEMENT_MARK_DEPTH.with(|depth| depth.get() != 0)
}

struct DrawGuard;

impl Drop for DrawGuard {
    fn drop(&mut self) {
        DRAW_DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

struct DrawInvalidationOwnerGuard {
    previous: Option<ElementId>,
}

impl DrawInvalidationOwnerGuard {
    #[inline]
    fn enter(owner: ElementId) -> Self {
        Self {
            previous: DRAW_INVALIDATION_OWNER.with(|current| current.replace(Some(owner))),
        }
    }
}

impl Drop for DrawInvalidationOwnerGuard {
    #[inline]
    fn drop(&mut self) {
        DRAW_INVALIDATION_OWNER.with(|current| current.set(self.previous));
    }
}

fn begin_draw() -> (DrawGuard, bool) {
    let outermost = DRAW_DEPTH.with(|depth| {
        let outermost = depth.get() == 0;
        depth.set(depth.get() + 1);
        outermost
    });
    (DrawGuard, outermost)
}

/// Identifies one logical element for as long as it remains in the element tree.
///
/// IDs are assigned monotonically and are never reused. Reconciliation transfers
/// an ID to a compatible newly generated element, while a genuine replacement
/// keeps its newly assigned ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ElementId(NonZeroU64);

impl ElementId {
    fn next() -> Self {
        let value = NEXT_ELEMENT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .expect("exhausted all element identities");
        Self(NonZeroU64::new(value).expect("element identity counter starts at one"))
    }

    /// Returns the non-zero integer representation of this identity.
    #[inline]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Tracks one built-in self-rebuilding element's dirty flag and current path.
/// The source is shared with the build consumer or state updater that can mark
/// the element outside the retained-tree walk.
pub(crate) struct DirtySource {
    dirty: Rc<Cell<bool>>,
    path: RefCell<Option<Rc<[ElementId]>>>,
    indexed: Cell<bool>,
    /// Whether the last build may have observed inherited context.
    ///
    /// Starts `true` and is cleared only when a build runs under a
    /// [`BuildConsumer`](crate::components::context::BuildConsumer), which sets
    /// it again on every untracked read. A source that never saw a tracked
    /// build therefore stays conservative.
    reads_context: Cell<bool>,
}

impl DirtySource {
    pub(crate) fn new(dirty: Rc<Cell<bool>>) -> Rc<Self> {
        Rc::new(Self {
            dirty,
            path: RefCell::new(None),
            indexed: Cell::new(false),
            reads_context: Cell::new(true),
        })
    }

    /// Starts a build that records whether it reads inherited context.
    #[inline]
    pub(crate) fn begin_build(&self) {
        self.reads_context.set(false);
    }

    /// Records that the build in progress read inherited context.
    #[inline]
    pub(crate) fn note_context_read(&self) {
        self.reads_context.set(true);
    }

    /// Returns whether the last build may have observed inherited context.
    #[inline]
    pub(crate) fn reads_context(&self) -> bool {
        self.reads_context.get()
    }

    /// Marks the source dirty and returns whether it was clean before this
    /// call.
    #[inline]
    pub(crate) fn mark(&self) -> bool {
        if self.dirty.get() {
            if let Some(path) = self.path.borrow().clone() {
                queue_element_invalidation(Some(path), ElementChangeKind::Unknown);
            } else {
                mark_paint_invalidations_unknown();
            }
            if !self.indexed.get() {
                invalidate_dirty_paths();
            }
            return false;
        }
        let mut first = false;
        with_rebuild_invalidation(|| {
            first = !self.dirty.replace(true);
            if first {
                let path = self.path.borrow().clone();
                if let Some(path) = path {
                    queue_element_invalidation(Some(path.clone()), ElementChangeKind::Unknown);
                    record_local_paint_invalidation_path(&path);
                    add_dirty_path(&path);
                    self.indexed.set(true);
                } else {
                    mark_paint_invalidations_unknown();
                    invalidate_dirty_paths();
                }
            }
        });
        first
    }

    #[inline]
    pub(crate) fn clear(&self) {
        self.dirty.set(false);
        let path = self.path.borrow().clone();
        if self.indexed.replace(false)
            && let Some(path) = path.as_deref()
        {
            remove_dirty_path(path);
        }
    }

    /// Associates the source with the current root-relative path. Stable paths
    /// are left in place so clean frames do not allocate.
    pub(crate) fn set_path(&self, path: &[ElementId]) {
        let old_path = self.path.borrow().clone();
        if old_path.as_deref() == Some(path) {
            return;
        }
        // A subtree walked on its own — an animation rebuilding the child it
        // replaced every frame — starts below the root, so the path it sees is
        // the tail of the one the root's walk registered, not a new place for the
        // element. Replacing the path with the tail, and discarding the index
        // because the roots differ, made two walks of the same element undo each
        // other every frame: the pass then descended the whole page each time.
        if old_path
            .as_deref()
            .is_some_and(|old| old.len() > path.len() && old.ends_with(path))
        {
            return;
        }

        let old_root = old_path.as_deref().and_then(|path| path.first()).copied();
        let new_root = path.first().copied();
        // The path registered before was the tail of this one: a walk that began
        // below the root saw the source first, and the walk from the root sees
        // all of it. While the pass prunes by the index it has not necessarily
        // reached every source it skipped, so each of those would stay on a path
        // the pass cannot follow. A pass that prunes nothing reaches them all and
        // upgrades each as it goes, which is what completes the index; discarding
        // it at the first upgrade only made the next pass walk everything again.
        let completes_a_tail = old_path
            .as_deref()
            .is_some_and(|old| path.len() > old.len() && path.ends_with(old))
            && !dirty_paths_cover_current_traversal();
        if old_root.is_some() && old_root != new_root && !completes_a_tail {
            // A source path is relative to the traversal root. If a subtree
            // was walked independently, its metadata cannot safely answer a
            // later walk from the retained application root.
            invalidate_dirty_paths();
        }

        if self.indexed.replace(false)
            && let Some(old_path) = old_path.as_deref()
        {
            remove_dirty_path(old_path);
        }

        let new_path: Rc<[ElementId]> = Rc::from(path.to_vec());
        *self.path.borrow_mut() = Some(new_path.clone());

        if self.dirty.get() {
            record_local_paint_invalidation_path(&new_path);
            add_dirty_path(&new_path);
            self.indexed.set(true);
        }
    }

    pub(crate) fn set_path_if_missing(&self, path: &[ElementId]) {
        if self.path.borrow().is_none() {
            self.set_path(path);
        }
    }
}

/// Associates a newly materialized rebuilding element with its structural path.
///
/// Dynamic child sources can use this when a stateful child appears after the
/// ordinary rebuild walk has already indexed its parent. Existing source paths
/// are left intact.
#[doc(hidden)]
pub fn set_rebuild_source_path(element: &dyn Element, path: &[ElementId]) {
    let Some(value) = element.option_any() else {
        return;
    };
    if let Some(stateless) = value.downcast_ref::<crate::widget::stateless::StatelessElement>() {
        stateless.dirty_source.set_path_if_missing(path);
    } else if let Some(stateful) = value.downcast_ref::<crate::widget::stateful::StatefulElement>() {
        stateful.dirty_source.borrow().set_path_if_missing(path);
    }
}

impl Drop for DirtySource {
    fn drop(&mut self) {
        if self.indexed.get()
            && let Some(path) = self.path.get_mut().as_deref()
        {
            remove_dirty_path(path);
        }
    }
}

/// One child a dirty path runs through, and how many paths do.
struct DirtyEdge {
    child: ElementId,
    paths: usize,
    /// Where the parent last found the child, so the next lookup is one probe.
    hint: Cell<usize>,
}

fn add_dirty_path(path: &[ElementId]) {
    DIRTY_SUBTREE_COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        for id in path {
            *counts.entry(*id).or_default() += 1;
        }
    });
    DIRTY_EDGES.with(|edges| {
        let mut edges = edges.borrow_mut();
        for pair in path.windows(2) {
            let siblings = edges.entry(pair[0]).or_default();
            match siblings.iter_mut().find(|edge| edge.child == pair[1]) {
                Some(edge) => edge.paths += 1,
                None => siblings.push(DirtyEdge { child: pair[1], paths: 1, hint: Cell::new(0) }),
            }
        }
    });
    DIRTY_EDGE_VERSION.with(|version| version.set(version.get() + 1));
}

fn remove_dirty_path(path: &[ElementId]) {
    DIRTY_SUBTREE_COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        for id in path {
            let Some(count) = counts.get_mut(id) else {
                continue;
            };
            *count -= 1;
            if *count == 0 {
                counts.remove(id);
            }
        }
    });
    DIRTY_EDGES.with(|edges| {
        let mut edges = edges.borrow_mut();
        for pair in path.windows(2) {
            let Some(siblings) = edges.get_mut(&pair[0]) else {
                continue;
            };
            if let Some(position) = siblings.iter().position(|edge| edge.child == pair[1]) {
                siblings[position].paths -= 1;
                if siblings[position].paths == 0 {
                    siblings.swap_remove(position);
                }
            }
            if siblings.is_empty() {
                edges.remove(&pair[0]);
            }
        }
    });
}

fn dirty_path_contains(id: ElementId) -> bool {
    DIRTY_SUBTREE_COUNTS.with(|counts| counts.borrow().contains_key(&id))
}

/// Whether the dirty-path index is currently usable, for tests.
#[cfg(test)]
pub(crate) fn dirty_paths_are_ready() -> bool {
    DIRTY_PATHS_READY.with(Cell::get)
}

/// Marks the dirty-path index usable, for tests.
#[cfg(test)]
pub(crate) fn mark_dirty_paths_ready() {
    DIRTY_PATHS_READY.with(|ready| ready.set(true));
}

pub(crate) fn invalidate_dirty_paths() {
    DIRTY_PATHS_READY.with(|ready| ready.set(false));
    DIRTY_INDEXED_ROOTS.with(|roots| roots.borrow_mut().clear());
    if REBUILD_TRAVERSAL_DEPTH.with(|depth| depth.get() > 0) {
        DIRTY_PATHS_INVALIDATED_DURING_TRAVERSAL.with(|invalidated| invalidated.set(true));
    }
}

/// The registered path of a holder whose own id changed while its ancestors did
/// not, or `None` when `current` describes a different place.
///
/// A path is relative to the walk that produced it, so one of the two is usually
/// the tail of the other, and what is compared is what lies above the holder. The
/// holder's id differs because whatever placed it again gave it a new one; keeping
/// the longer of the two ancestor chains, with the newest id at the end, is what
/// both walks agree on.
fn path_after_new_id(registered: &[ElementId], current: &[ElementId]) -> Option<Vec<ElementId>> {
    let (registered_last, registered_above) = registered.split_last()?;
    let (current_last, current_above) = current.split_last()?;
    if registered_last == current_last {
        return None;
    }
    if registered_above.len() >= current_above.len() && registered_above.ends_with(current_above) {
        let mut path = registered.to_vec();
        *path.last_mut()? = *current_last;
        Some(path)
    } else if current_above.ends_with(registered_above) {
        Some(current.to_vec())
    } else {
        None
    }
}

struct RebuildTraversalGuard {
    outermost: bool,
    complete: bool,
    root: ElementId,
}

impl Drop for RebuildTraversalGuard {
    fn drop(&mut self) {
        REBUILD_TRAVERSAL_DEPTH.with(|depth| depth.set(depth.get() - 1));
        if self.outermost {
            REBUILD_TRAVERSAL_ROOT.with(|root| root.set(None));
        }
        if self.outermost
            && self.complete
            && !DIRTY_PATHS_INVALIDATED_DURING_TRAVERSAL.with(Cell::get)
        {
            DIRTY_INDEXED_ROOTS.with(|roots| {
                roots.borrow_mut().insert(self.root);
            });
            DIRTY_PATHS_READY.with(|ready| ready.set(true));
        }
    }
}

fn begin_rebuild_traversal(root: ElementId) -> RebuildTraversalGuard {
    let outermost = REBUILD_TRAVERSAL_DEPTH.with(|depth| {
        let outermost = depth.get() == 0;
        if outermost {
            DIRTY_PATHS_INVALIDATED_DURING_TRAVERSAL.with(|invalidated| invalidated.set(false));
            REBUILD_TRAVERSAL_ROOT.with(|current| current.set(Some(root)));
        }
        depth.set(depth.get() + 1);
        outermost
    });
    RebuildTraversalGuard {
        outermost,
        complete: false,
        root,
    }
}

/// Whether the dirty-path index was built from the root of the traversal in
/// progress, and so describes everything that traversal could prune.
///
/// The index is shared, but a clean traversal of a subtree — a scrollable
/// rebuilding its content while it draws — only indexes that subtree. Taking it
/// as ready for every root let such a walk undo the invalidation another
/// element had just asked for, and the pass from the real root then pruned
/// branches that had never been indexed.
#[inline]
pub(crate) fn dirty_paths_cover_current_traversal() -> bool {
    DIRTY_PATHS_READY.with(Cell::get)
        && REBUILD_TRAVERSAL_ROOT.with(Cell::get).is_some_and(|root| {
            DIRTY_INDEXED_ROOTS.with(|roots| roots.borrow().contains(&root))
        })
}

struct RebuildPathGuard;

impl RebuildPathGuard {
    fn push(id: ElementId) -> Self {
        REBUILD_PATH.with(|path| path.borrow_mut().push(id));
        Self
    }
}

impl Drop for RebuildPathGuard {
    fn drop(&mut self) {
        REBUILD_PATH.with(|path| {
            path.borrow_mut().pop();
        });
    }
}

struct RebuildDescendGuard {
    active: bool,
}

impl RebuildDescendGuard {
    fn enter(active: bool) -> Self {
        if active {
            REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(depth.get() + 1));
        }
        Self { active }
    }
}

impl Drop for RebuildDescendGuard {
    fn drop(&mut self) {
        if self.active {
            REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
}


fn bounds_from_element(element: &dyn Element) -> Option<aimer_attribute::Bounds> {
    element.event_tree_bounds().map(|(start, end)| {
        aimer_attribute::Bounds::new(
            start.x,
            start.y,
            end.x - start.x,
            end.y - start.y,
        )
    })
}

impl<T> Element for T where T: VisitorElement + EventElement + LayoutElement + Rebuildable + Drawable
{}

pub trait Element: VisitorElement + EventElement + LayoutElement + Rebuildable + Drawable {
    /// Returns this element's stable logical identity.
    ///
    /// The identity remains unchanged when the owning [`AnyElement`] moves and
    /// is transferred to compatible generated elements during reconciliation.
    #[inline]
    fn id(&self) -> ElementId {
        self.element_id()
            .expect("all erased elements must carry an ElementId")
    }
    /// Erases this element into an inline-or-heap [`AnyElement`].
    ///
    /// Elements fitting `Rubick`'s configured size and alignment are embedded
    /// directly in the returned owner. Larger or over-aligned elements use one
    /// heap allocation. The historical method name is retained for source
    /// familiarity and does not imply that allocation occurred.
    ///
    /// Borrowing the owner provides a `dyn Element` view. Moving an inline
    /// owner also moves its concrete element, so callers must not rely on a
    /// stable payload address without pinning.
    ///
    /// This method requires a sized, `'static` concrete element because stable
    /// Rust does not support general implicit unsizing for custom smart
    /// pointers.
    fn boxed(self) -> AnyElement
    where
        Self: Sized + 'static,
    {
        AnyElement::from_node(ElementNode {
            id: Cell::new(ElementId::next()),
            element: self,
        })
    }

    /// Erases this element using the supplied application UI allocator.
    ///
    /// The resulting retained element keeps its allocation source alive until
    /// the element is dropped, including when it outlives the allocator scope.
    #[inline]
    fn boxed_in(self, allocator: &UiAllocator) -> AnyElement
    where
        Self: Sized + 'static,
    {
        AnyElement::from_node_in(
            ElementNode {
                id: Cell::new(ElementId::next()),
                element: self,
                },
            allocator,
        )
    }
}

// SAFETY: The template is `null::<ElementNode<E>>()` coerced to the target, so
// it carries exactly that node's vtable and a null data address.
/// The most unmapped draws one frame keeps; a runaway tree must not grow it
/// without bound.
const UNMAPPED_DRAW_LIMIT: usize = 1024;

/// Notes that `id` was drawn under retained presentation with no render node.
#[inline]
fn record_unmapped_draw(id: ElementId, name: &'static str) {
    UNMAPPED_DRAWS.with(|draws| {
        let mut draws = draws.borrow_mut();
        if draws.len() < UNMAPPED_DRAW_LIMIT && !draws.iter().any(|(seen, _)| *seen == id) {
            draws.push((id, name));
        }
    });
}

/// Notes that `id` had no retained paint to show this frame (it declined, or
/// its state was not valid enough to paint).
#[cfg(debug_assertions)]
fn record_declined_paint(id: ElementId, name: &'static str) {
    DECLINED_PAINT_ELEMENTS.with(|elements| {
        let mut elements = elements.borrow_mut();
        if elements.len() < UNMAPPED_DRAW_LIMIT && !elements.iter().any(|(seen, _)| *seen == id) {
            elements.push((id, name));
        }
    });
}

/// Takes the elements that had no retained paint to show since the last call.
/// Only debug builds measure this, so release builds see none.
#[doc(hidden)]
pub fn take_declined_paint_elements() -> Vec<(ElementId, &'static str)> {
    #[cfg(debug_assertions)]
    {
        DECLINED_PAINT_ELEMENTS.with(|elements| std::mem::take(&mut *elements.borrow_mut()))
    }
    #[cfg(not(debug_assertions))]
    {
        Vec::new()
    }
}

/// Takes the elements recorded as drawn without a render node since the last
/// call. Frame loops call this to discard stale entries at frame start and
/// again after their final render-tree sync to see which are still unmapped.
#[doc(hidden)]
pub fn take_unmapped_draws() -> Vec<(ElementId, &'static str)> {
    UNMAPPED_DRAWS.with(|draws| std::mem::take(&mut *draws.borrow_mut()))
}

/// Returns the generation of the currently installed element-tree structure.
///
/// The generation advances whenever a generated child subtree is replaced. It
/// can be used to invalidate path indexes without rescanning the tree for every
/// event.
#[inline]
pub fn element_tree_generation() -> u64 {
    ELEMENT_TREE_GENERATION.load(Ordering::Acquire)
}

/// Returns the generation of dynamically exposed children used by retained rendering.
///
/// This is separate from [`element_tree_generation`], which also invalidates
/// event-path indexes. Virtualized sources advance this generation when their
/// live child window changes without replacing the owning element.
#[inline]
pub fn retained_render_structure_generation() -> u64 {
    RETAINED_RENDER_STRUCTURE_GENERATION.load(Ordering::Acquire)
}

/// Notifies retained rendering that an element's live child window changed.
///
/// Use this when `visit_children` starts exposing a different set of live
/// children while the owning element remains in place. Ordinary generated-tree
/// replacements already advance [`element_tree_generation`].
#[doc(hidden)]
#[inline]
pub fn notify_retained_render_structure_changed() {
    RETAINED_RENDER_STRUCTURE_GENERATION
        .try_update(Ordering::Release, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .expect("exhausted all retained render structure generations");
}

/// Makes the next rebuild pass reach every ticking element again.
///
/// A rebuild pass normally visits only the elements that were marked dirty and
/// the paths leading to them, and a stateless ancestor does not descend at all
/// while the rebuild generation is the one it last saw. An element that
/// advances on its own clock, such as a running animation, is never marked, so
/// a tick that publishes nothing new (an eased animation's first steps round to
/// the start value) would leave the pass skipping it, and the animation would
/// stop where it stood. Such an element calls this from `rebuild_if_dirty` for
/// as long as it still has frames to produce. It repaints the target, like any
/// rebuild invalidation, and leaves layout alone.
#[doc(hidden)]
#[inline]
pub fn keep_ticking_elements_reachable() {
    advance_rebuild_invalidation_generation();
}

/// Keeps the calling element reachable by the next rebuild pass, without
/// repainting anything.
///
/// A rebuild pass visits only the elements marked dirty and the paths leading to
/// them. An element that rebuilds its own child on its own clock, such as a
/// running [`AnimatedBuilder`], is never marked, so the pass would skip it and
/// the animation would stand still. Calling this from `rebuild_if_dirty` for as
/// long as the element still has frames to produce makes the next pass descend
/// to it again.
///
/// Unlike [`keep_ticking_elements_reachable`] this does not mark paint
/// invalidations unknown or damage the whole target: the caller's own rebuild
/// reports what it changed. Each call is counted, see
/// [`rebuild_keepalive_count`], so a frame loop can tell a generation that moved
/// only to keep an element reachable from one that moved because something
/// changed.
///
/// [`AnimatedBuilder`]: https://docs.rs/aimer_animation
#[doc(hidden)]
#[inline]
pub fn keep_element_reachable() {
    invalidate_dirty_paths();
    advance_tracked_rebuild_invalidation_generation();
    REBUILD_KEEPALIVE_COUNT.fetch_add(1, Ordering::Release);
}

/// Keeps one element reachable by the rebuild pass for as long as it holds.
///
/// [`keep_element_reachable`] makes the next pass descend to the caller by
/// discarding the index of dirty paths, so every animating frame walks the whole
/// tree to find one element. This registers the path to the element in that
/// index instead: the pass prunes everything off the path and still reaches the
/// element, and the index stays valid for every other consumer.
///
/// The holder owns its registration. Releasing it, or dropping it while it still
/// holds, removes the path again, so an element that is dropped mid-animation
/// cannot leave a path behind that keeps its ancestors descending.
///
/// Nothing is recorded as a paint invalidation: the holder's own rebuild reports
/// what it changed, as with [`keep_element_reachable`].
#[doc(hidden)]
#[derive(Default)]
pub struct KeepReachable {
    /// The registered path, empty while nothing is held.
    path: RefCell<Vec<ElementId>>,
}

impl KeepReachable {
    /// Creates a holder that holds nothing.
    #[inline]
    pub const fn new() -> Self {
        Self { path: RefCell::new(Vec::new()) }
    }

    /// Keeps the element being rebuilt reachable for the next pass.
    ///
    /// Call from `rebuild_if_dirty` on every frame the element still has work
    /// to produce. The registration only changes when the element's path does.
    /// Outside a rebuild pass there is no path to register, so this falls back
    /// to [`keep_element_reachable`].
    pub fn hold(&self) {
        let registered = REBUILD_PATH.with(|current| {
            let current = current.borrow();
            if current.is_empty() {
                return false;
            }
            let mut path = self.path.borrow_mut();
            // A path is relative to the traversal that produced it. A subtree
            // walked on its own — a stateful element rebuilding itself — starts
            // below the root, so the path it sees is the tail of the one the
            // root's walk registered, not a new place for the element. Replacing
            // the full path with the tail would leave the root's ancestors off
            // the index, and every later pass would prune at them and never
            // reach the holder again.
            let is_tail_of_registered = path.len() > current.len() && path.ends_with(&current);
            if let Some(renamed) = path_after_new_id(&path, &current) {
                // The element is where it was, under the same ancestors, but
                // something placed it again and it has another id. That is not a
                // move: only its own entry on the index changes.
                if *path != renamed {
                    remove_dirty_path(&path);
                    path.clear();
                    path.extend_from_slice(&renamed);
                    add_dirty_path(&path);
                }
            } else if *path != *current && !is_tail_of_registered {
                // The index cannot answer a walk from a different root, so the
                // next pass has to be a full one.
                if path.first().is_some_and(|old_root| Some(old_root) != current.first()) {
                    invalidate_dirty_paths();
                }
                if !path.is_empty() {
                    remove_dirty_path(&path);
                }
                path.clear();
                path.extend_from_slice(&current);
                add_dirty_path(&path);
            }
            true
        });
        if registered {
            // Stateless ancestors skip their subtree while this is unchanged.
            advance_tracked_rebuild_invalidation_generation();
            REBUILD_KEEPALIVE_COUNT.fetch_add(1, Ordering::Release);
        } else {
            keep_element_reachable();
        }
    }

    /// Whether a rebuild pass has registered a path for this holder.
    ///
    /// A holder that was never visited by a pass — a row a lazily built list
    /// materialized while drawing, say — has none, and nothing will visit it
    /// until something asks the next pass to look.
    #[inline]
    pub fn is_registered(&self) -> bool {
        !self.path.borrow().is_empty()
    }

    /// Stops keeping the element reachable.
    pub fn release(&self) {
        let mut path = self.path.borrow_mut();
        if !path.is_empty() {
            remove_dirty_path(&path);
            path.clear();
        }
    }
}

impl Drop for KeepReachable {
    fn drop(&mut self) {
        self.release();
    }
}

/// Runs `commit`, a reconciliation of a subtree that the caller then passes to
/// [`rebuild_replaced_subtree`], without discarding the index of dirty paths.
///
/// A replacement normally cannot say which retained paths it left stale, so it
/// discards the index and the next pass walks the whole tree to rebuild it. A
/// caller that replaces the subtree below one element, and re-registers every
/// path in that subtree afterwards, has already done what the walk would.
///
/// Anything outside the replaced subtree is untouched by a scoped
/// reconciliation, so its paths stay valid.
#[doc(hidden)]
pub fn preserving_dirty_paths<R>(commit: impl FnOnce() -> R) -> R {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            PRESERVE_DIRTY_PATHS_DEPTH.with(|depth| depth.set(depth.get() - 1));
        }
    }
    PRESERVE_DIRTY_PATHS_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _restore = Restore;
    commit()
}

/// Runs the rebuild pass over a subtree that [`preserving_dirty_paths`] just
/// replaced.
///
/// Call from the replaced subtree's owner, inside its `rebuild_if_dirty`. The
/// pass descends the whole subtree once instead of pruning it. Nothing in a
/// fresh subtree has been visited, so it cannot be told apart from one with
/// work in it, and the descent is what registers the subtree: every element
/// sets its dirty-source path from the pass's own path, replacing the one a
/// carried state holder brought with it from wherever it used to be. An element
/// in the subtree that keeps itself reachable, such as a nested animator, also
/// gets to do so.
///
/// Falls back to discarding the index when there is no pass to root the paths
/// in.
#[doc(hidden)]
pub fn rebuild_replaced_subtree(root: &dyn Element, ctx: &BuildContext) {
    if REBUILD_PATH.with(|path| path.borrow().is_empty()) {
        invalidate_dirty_paths();
    }
    REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(depth.get() + 1));
    root.rebuild_if_dirty(ctx);
    REBUILD_FORCE_DESCEND_DEPTH.with(|depth| depth.set(depth.get() - 1));
}

/// How many retained boundaries the rebuild walk entered and did not prune.
///
/// A boundary that returns early because its dirty path holds no work is not
/// counted, so the difference across a frame is the size of the subtree the
/// rebuild prepass really descended.
#[doc(hidden)]
#[inline]
pub fn rebuild_descent_count() -> u64 {
    REBUILD_DESCENTS.with(Cell::get)
}

/// Rebuilds the children of the element being rebuilt that dirty paths run
/// through, and only those.
///
/// For [`Rebuildable::rebuild_indexed_children`]. `child_at` is the
/// container's own positional access to its children. Returns `false`, having
/// rebuilt whatever it reached, when the pass cannot rely on the edges: a child
/// is not where the container can find it, nothing leads below this element, or
/// the index grew or was discarded while the children rebuilt. The caller then
/// visits every child, which is always correct because each child decides for
/// itself whether it has work.
#[doc(hidden)]
pub fn rebuild_dirty_children<'a>(
    len: usize,
    child_at: impl Fn(usize) -> Option<&'a dyn Element>,
    ctx: &BuildContext,
) -> bool {
    let Some(parent) = REBUILD_PATH.with(|path| path.borrow().last().copied()) else {
        return false;
    };
    let wanted: SmallVec<[(ElementId, usize); 4]> = DIRTY_EDGES.with(|edges| {
        edges.borrow().get(&parent).map_or_else(SmallVec::new, |siblings| {
            siblings.iter().map(|edge| (edge.child, edge.hint.get())).collect()
        })
    });
    if wanted.is_empty() {
        return false;
    }

    let mut found: SmallVec<[(usize, &'a dyn Element); 4]> = SmallVec::new();
    for (id, hint) in wanted {
        let mut located = child_at(hint).filter(|child| child.id() == id).map(|child| (hint, child));
        if located.is_none() {
            located = (0..len).find_map(|index| {
                child_at(index).filter(|child| child.id() == id).map(|child| (index, child))
            });
        }
        let Some((index, child)) = located else {
            return false;
        };
        if index != hint {
            DIRTY_EDGES.with(|edges| {
                if let Some(edge) = edges
                    .borrow()
                    .get(&parent)
                    .and_then(|siblings| siblings.iter().find(|edge| edge.child == id))
                {
                    edge.hint.set(index);
                }
            });
        }
        found.push((index, child));
    }
    // The order a full visit would have used.
    found.sort_unstable_by_key(|(index, _)| *index);
    found.dedup_by_key(|(index, _)| *index);

    let version = DIRTY_EDGE_VERSION.with(Cell::get);
    for (_, child) in found {
        child.rebuild_if_dirty(ctx);
    }
    // A child that marked a sibling dirty, or a path that was registered or
    // discarded, may have work the snapshot did not know about.
    DIRTY_EDGE_VERSION.with(Cell::get) == version && DIRTY_PATHS_READY.with(Cell::get)
}

/// How many stable-generation changes the log remembers. A reader that falls
/// further behind finds its cursor gone and must check every child.
const MAX_LOGGED_STABLE_GENERATIONS: usize = 1024;

struct StableGenerationLog {
    /// Sequence number of `ids[0]`.
    first: u64,
    ids: Vec<ElementId>,
}

/// Returns the current end of the log of stable-generation changes.
///
/// Take it before looking at any child, then hand it to
/// [`stable_generations_since`] on the next pass.
#[doc(hidden)]
#[inline]
pub fn stable_generation_cursor() -> u64 {
    STABLE_GENERATION_LOG.with(|log| {
        let log = log.borrow();
        log.first + log.ids.len() as u64
    })
}

/// Calls `visit` with every element whose stable generation changed since
/// `cursor`, and returns `true`; returns `false`, visiting nothing, when the
/// log no longer reaches back that far.
///
/// An element that opts into generation-independent sizing changes size only
/// when it is replaced, and replacing it sets its generation. A container that
/// last checked its children at `cursor` therefore need only look at the
/// elements reported here, plus the children that did not opt in.
#[doc(hidden)]
pub fn stable_generations_since(cursor: u64, visit: &mut dyn FnMut(ElementId)) -> bool {
    STABLE_GENERATION_LOG.with(|log| {
        let log = log.borrow();
        let Some(skip) = cursor.checked_sub(log.first) else {
            return false;
        };
        for id in log.ids.iter().skip(skip as usize) {
            visit(*id);
        }
        true
    })
}

fn note_stable_generation_change(id: ElementId) {
    STABLE_GENERATION_LOG.with(|log| {
        let mut log = log.borrow_mut();
        log.ids.push(id);
        if log.ids.len() > MAX_LOGGED_STABLE_GENERATIONS {
            let dropped = log.ids.len() / 2;
            log.ids.drain(..dropped);
            log.first += dropped as u64;
        }
    });
}

/// How many retained boundaries the rebuild walk entered, pruned or not.
///
/// Together with [`rebuild_descent_count`] this separates the cost of
/// deciding to skip a subtree from the cost of walking it.
#[doc(hidden)]
#[inline]
pub fn rebuild_visit_count() -> u64 {
    REBUILD_VISITS.with(Cell::get)
}

/// How many times the whole retained tree was searched for an element the index
/// could not find.
///
/// A frame should need at most a search or two however many elements changed;
/// one per changed element makes a change that touches the whole tree, such as a
/// theme switch, cost the square of its size.
#[doc(hidden)]
#[inline]
pub fn structural_search_count() -> u64 {
    STRUCTURAL_SEARCHES.with(Cell::get)
}

#[inline]
pub(crate) fn note_structural_search() {
    STRUCTURAL_SEARCHES.with(|count| count.set(count.get() + 1));
}

#[inline]
pub(crate) fn note_rebuild_visit() {
    REBUILD_VISITS.with(|count| count.set(count.get() + 1));
}

#[inline]
pub(crate) fn note_rebuild_descent() {
    REBUILD_DESCENTS.with(|count| count.set(count.get() + 1));
}

/// How many times [`keep_element_reachable`] has advanced the rebuild
/// generation.
#[doc(hidden)]
#[inline]
pub fn rebuild_keepalive_count() -> u64 {
    REBUILD_KEEPALIVE_COUNT.load(Ordering::Acquire)
}

/// Records that a state update just marked an element dirty, advancing the
/// rebuild generation.
///
/// The element rebuilds its own subtree, which the log of replaced subtrees
/// describes, so a frame loop may treat the advance as explained, see
/// [`rebuild_state_mark_count`]. A dependency-driven mark is not counted: it
/// reaches every element that read the dependency.
#[doc(hidden)]
#[inline]
pub fn note_state_update_marked() {
    REBUILD_STATE_MARK_COUNT.fetch_add(1, Ordering::Release);
}

/// How many times a state update advanced the rebuild generation.
#[doc(hidden)]
#[inline]
pub fn rebuild_state_mark_count() -> u64 {
    REBUILD_STATE_MARK_COUNT.load(Ordering::Acquire)
}

/// Invalidates event paths and retained render nodes for a hosted child change.
///
/// Unlike [`notify_element_tree_changed`], this does not invalidate layout or
/// force full paint damage. Use it when a hosted subtree is added to or removed
/// from an overlay whose layout bounds remain stable.
#[doc(hidden)]
pub fn notify_hosted_element_tree_changed() {
    advance_element_tree_generation();
    notify_retained_render_structure_changed();
}

/// Returns the generation of the most recent layout invalidation.
///
/// Layout caches include this value in their keys. A layout invalidation can
/// therefore retire cached measurements without recursively visiting every
/// descendant of the retained tree.
#[inline]
pub fn layout_invalidation_generation() -> u64 {
    LAYOUT_INVALIDATION_GENERATION.load(Ordering::Acquire)
}

/// Tells every layout cache that some element's size changed without the
/// element being replaced.
///
/// A container that sizes itself — a panel being dragged wider, a body that
/// opens — cannot be noticed by the parents that measured it: they cache what
/// they measured under the layout generation, and nothing else changes. Call this
/// from the code that changes the size. It retires cached measurements without
/// touching the element tree, so it is cheap enough for every frame of a drag.
#[doc(hidden)]
#[inline]
pub fn notify_layout_changed() {
    advance_layout_invalidation_generation();
}

/// Advances the layout invalidation generation.
#[inline]
pub(crate) fn advance_layout_invalidation_generation() {
    LAYOUT_INVALIDATION_GENERATION
        .try_update(Ordering::Release, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .expect("exhausted all layout invalidation generations");
    queue_element_invalidation(None, ElementChangeKind::Layout);
}

/// Returns the generation of the most recent rebuild invalidation.
///
/// Paint caches include this value so a state, style, text, or other rebuild
/// that can change a subtree's visual output retires its retained commands
/// before they are replayed.
#[inline]
pub fn rebuild_invalidation_generation() -> u64 {
    REBUILD_INVALIDATION_GENERATION.load(Ordering::Acquire)
}

/// Advances the rebuild invalidation generation.
#[inline]
pub(crate) fn advance_rebuild_invalidation_generation() {
    invalidate_dirty_paths();
    advance_tracked_rebuild_invalidation_generation();
    // Async and portable-state producers do not have an element-owned dirty
    // path to record. Retained paint therefore cannot identify a smaller
    // footprint safely; invalidate it conservatively and repaint the target.
    mark_paint_invalidations_unknown();
    mark_paint_damage_full();
}

#[inline]
fn advance_tracked_rebuild_invalidation_generation() {
    REBUILD_INVALIDATION_GENERATION
        .try_update(Ordering::Release, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .expect("exhausted all rebuild invalidation generations");
}

/// Runs one public dirty-marking operation under a single invalidation bump.
pub(crate) fn with_rebuild_invalidation<R>(operation: impl FnOnce() -> R) -> R {
    let outermost = REBUILD_INVALIDATION_DEPTH.with(|depth| depth.get() == 0);
    if outermost {
        advance_tracked_rebuild_invalidation_generation();
    }
    REBUILD_INVALIDATION_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let _guard = RebuildInvalidationGuard;
    operation()
}

fn advance_element_tree_generation() {
    advance_element_tree_generation_counter();
    note_unscoped_tree_change();
}

/// Advances the tree generation for the replacement of the subtree at `root`.
///
/// Every consumer of [`element_tree_generation`] still sees the advance. A
/// consumer that tracks [`unscoped_element_tree_generation`] sees only that a
/// subtree was replaced, and which one.
fn advance_element_tree_generation_scoped(root: Option<ElementId>) {
    advance_element_tree_generation_counter();
    note_scoped_rebuild(root);
}

fn advance_element_tree_generation_counter() {
    #[cfg(test)]
    let _generation_lock = TEST_ELEMENT_TREE_GENERATION_LOCK
        .lock()
        .expect("element-tree generation test lock must not be poisoned");

    if PRESERVE_DIRTY_PATHS_DEPTH.with(|depth| depth.get() == 0) {
        invalidate_dirty_paths();
    }
    ELEMENT_TREE_GENERATION
        .try_update(Ordering::Release, Ordering::Relaxed, |generation| {
            generation.checked_add(1)
        })
        .expect("exhausted all element-tree generations");
}

/// Notifies retained traversal that an element's exposed child tree changed.
///
/// Most widgets should update children through rebuild reconciliation. A
/// widget that installs a child after asynchronous loading can use this hook
/// when its `visit_children` result changes without replacing its element.
/// Layout caches are invalidated alongside retained traversal.
#[doc(hidden)]
pub fn notify_element_tree_changed() {
    advance_element_tree_generation();
    advance_layout_invalidation_generation();
}

#[cfg(test)]
pub(crate) fn test_generation_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_ELEMENT_TREE_GENERATION_LOCK
        .lock()
        .expect("element-tree generation test lock must not be poisoned")
}

pub(crate) fn structural_children(element: &dyn Element) -> SmallVec<[&dyn Element; 8]> {
    let mut children: SmallVec<[&dyn Element; 8]> = SmallVec::new();
    element.structural_children(&mut |child| children.push(child));
    children
}

#[inline]
fn structural_child_at(element: &dyn Element, target: usize) -> Option<&dyn Element> {
    let mut index = 0;
    let mut result = None;
    element.structural_children(&mut |child| {
        if index == target {
            result = Some(child);
        }
        index = index.saturating_add(1);
    });
    result
}

/// Whether two elements describe the same widget in the same role.
///
/// Same concrete element type, same debug name and the same reconciliation key —
/// two keyed elements match only on equal keys, and a keyed one never matches an
/// unkeyed one.
pub(crate) fn identities_are_compatible(old: &dyn Element, new: &dyn Element) -> bool {
    if old.element_type_id() != new.element_type_id() || old.debug_name() != new.debug_name() {
        return false;
    }

    // Two different retained children are two different subtrees. Their
    // elements outlive the build that placed them, so neither is the other's
    // replacement: pairing them by position would carry one's state into the
    // other.
    if different_retained_children(old, new) {
        return false;
    }

    match (old.reconciliation_key(), new.reconciliation_key()) {
        (None, None) => true,
        (Some(old), Some(new)) => old == new,
        _ => false,
    }
}

/// Transfers logical identities from an old subtree to compatible nodes in a
/// newly generated subtree.
///
/// Keyed children match by key regardless of sibling order. Unkeyed children
/// match only in the same sibling position. Incompatible or removed nodes keep
/// their newly allocated identities.
#[cfg(test)]
pub(crate) fn reconcile_element_identities(old: &dyn Element, new: &dyn Element) {
    crate::reconciliation_plan::plan_element_reconciliation(old, new).apply_identities();
}

/// Reconciles identities for a generated subtree and invalidates structural
/// path indexes.
pub(crate) fn reconcile_generated_tree(old: &dyn Element, new: &dyn Element) {
    crate::reconciliation_plan::plan_element_reconciliation(old, new)
        .commit_generated_tree()
        .expect("fresh reconciliation plan must remain valid until commit");
}

/// Carries live state from `old` into the freshly built `new`, then reconciles
/// the two as [`reconcile_generated_tree`] does.
///
/// The state carry reconciles nested pairs of its own. They are logged before
/// `new` takes over the identities of `old`, so the log is corrected when it
/// does.
pub(crate) fn reconcile_carried_tree(old: &dyn Element, new: &dyn Element, ctx: &BuildContext) {
    let carried_from = scoped_rebuild_cursor();
    crate::widget::stateful::carry_child_state(old, new, ctx);
    crate::reconciliation_plan::plan_element_reconciliation(old, new)
        .commit_carried_tree(carried_from)
        .expect("fresh reconciliation plan must remain valid until commit");
}

/// Returns whether `old` and `new` are two placements of one retained child.
///
/// Both then stand for the same subtree, so reconciling them could only
/// compare it with itself.
#[inline]
pub(crate) fn same_retained_child(old: &dyn Element, new: &dyn Element) -> bool {
    matches!(
        (old.retained_identity(), new.retained_identity()),
        (Some(old), Some(new)) if old == new
    )
}

/// Returns whether `old` and `new` are placements of two *different* retained
/// slots.
///
/// A slot is a part its owner declared and keeps under a name, and its element
/// outlives the build that placed it, so neither of two different slots
/// replaces the other: whatever state each holds is its own, and nothing may be
/// carried from one into the other by sharing a position.
///
/// Only slots are isolated this way. A retained child that is not a slot (a
/// route page, say) is routinely replaced by another at the same position, and
/// the state below it is meant to flow across.
#[inline]
pub(crate) fn different_retained_children(old: &dyn Element, new: &dyn Element) -> bool {
    matches!(
        (old.retained_identity(), new.retained_identity()),
        (Some(old_id), Some(new_id)) if old_id != new_id
    ) && (old.is_retained_slot() || new.is_retained_slot())
}

pub(crate) fn complete_generated_tree_reconciliation(old: &dyn Element, new: &dyn Element) {
    clear_removed_focus(old, new);
    // The replacement changes the subtree below `new` and nothing outside it.
    advance_element_tree_generation_scoped(new.element_id());
    new.set_subtree_generation(element_tree_generation());
}

fn event_callback_enabled(element: &dyn Element) -> bool {
    element.event_tree_role() != EventTreeRole::Transparent
}

fn clear_removed_focus(old: &dyn Element, new: &dyn Element) {
    if same_retained_child(old, new) {
        return;
    }
    let Some((focused_element, focused_id, focused_node)) = find_focused_element(old) else {
        return;
    };
    if contains_focus_attachment(new, focused_id, focused_node) {
        return;
    }

    focused_node.set_focused(false);
    if event_callback_enabled(focused_element) {
        let _ = focused_element.on_event(&ElementEvent::FocusLost);
    }
}

fn find_focused_element(element: &dyn Element) -> Option<(&dyn Element, ElementId, &FocusNode)> {
    if let (Some(id), Some(node)) = (element.element_id(), element.focus_node())
        && node.has_focus()
    {
        return Some((element, id, node));
    }
    for child in structural_children(element) {
        if let Some(focused) = find_focused_element(child) {
            return Some(focused);
        }
    }
    None
}

fn contains_focus_attachment(
    element: &dyn Element,
    focused_id: ElementId,
    focused_node: &FocusNode,
) -> bool {
    if element.element_id() == Some(focused_id)
        && element
            .focus_node()
            .is_some_and(|node| node.ptr_eq(focused_node))
    {
        return true;
    }
    structural_children(element)
        .into_iter()
        .any(|child| contains_focus_attachment(child, focused_id, focused_node))
}
