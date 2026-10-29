/// Differential audit of cached interaction bounds against the render tree.
#[doc(hidden)]
pub mod bounds_audit;
#[cfg(test)]
mod base_context_tests;
/// Diagnostic census of render nodes by paint source.
#[doc(hidden)]
pub mod census;
pub mod event_handler;
/// The file drag the window is currently under.
mod accessibility;
pub(crate) mod file_drag;
mod native_input;
mod platform_gesture;
#[cfg(not(target_arch = "wasm32"))]
mod render_worker;
mod scoped_sync;
pub(crate) use aimer_platform::system_back;
pub mod scroll_classifier;
pub mod scroll_utils;
#[cfg(any(target_os = "ios", test))]
mod uikit_frame;
pub(crate) mod user_events;
/// Gesture segmentation for the phase-less browser wheel stream.
///
/// Compiled for the web target only; the `test` predicate keeps the logic
/// unit-testable from a host build, where no browser exists.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) mod web_scroll_phase;

#[cfg(target_os = "android")]
use crate::aimer_app::ANDROID_APP;
use crate::aimer_app::{AimerNativePlatformEvent, FrameRequestKind};
#[cfg(target_os = "android")]
use crate::ffi_utils::android_screen;
#[allow(unused)]
use crate::handler;
use crate::handler::event_handler::WindowEventHandler;
use crate::handler::file_drag::FileDrag;
use crate::handler::scroll_classifier::DualScroller;
use crate::handler::user_events::handle_user_event;
use crate::render_ctx::AimerRenderContext;
use crate::window_attr::WindowAttr;
#[cfg(feature = "wasm-hot-reload")]
use aimer_anteros::{
    ReloadCommit, ReloadCommitError, ReloadCoordinator, ReloadEventDisposition,
    ReloadEventOverflow, ReloadGuest, ReloadSnapshot, ReloadTransactionError, ReloadTransactionId,
};
use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd_v2::{
    DrawCommand as V2DrawCommand, Rect as RenderRect, RenderFrame, RenderNodeId, RenderNodeSpec,
    RenderOp, RenderPaintSource, RenderTree, RenderTreeError,
};
use aimer_cupid::frame::{Frame, FramePacket, FrameRenderMetadata};
use aimer_rubick::{UiAllocator, UiMemory};
use aimer_utils::debug;
use aimer_venus::Venus;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, CALLED, Element, ElementChangeKind, ElementId, ElementInvalidationBatch,
    ElementNodeMap, EventDispatcher, EventResult, Widget, begin_event_frame,
};
#[cfg(feature = "wasm-hot-reload")]
use aimer_widget::{ReconciliationPlanError, plan_element_reconciliation};
use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Handle;
use winit::application::ApplicationHandler;
#[allow(unused)]
use winit::dpi::{LogicalSize, PhysicalSize, Position};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
#[allow(unused)]
use winit::monitor::MonitorHandle;
#[allow(unused)]
use winit::window::{self, Fullscreen, Window, WindowAttributes, WindowId};

pub(crate) type StartupHook = Box<dyn FnOnce() -> Box<dyn Any>>;

/// A fully prepared reload candidate waiting for a Quiver host safe point.
///
/// The command owns the candidate guest, callback snapshot, and disconnected
/// native root as one value. Dropping a superseded command therefore retires
/// the complete candidate without touching the active application.
#[cfg(feature = "wasm-hot-reload")]
pub struct ReloadCommand<G: ReloadGuest, R> {
    transaction: ReloadTransactionId,
    candidate: ReloadSnapshot<G, R>,
}

#[cfg(feature = "wasm-hot-reload")]
impl<G: ReloadGuest, R> ReloadCommand<G, R> {
    /// Creates a command from a coordinator-issued transaction and candidate.
    #[inline]
    pub const fn new(transaction: ReloadTransactionId, candidate: ReloadSnapshot<G, R>) -> Self {
        Self {
            transaction,
            candidate,
        }
    }
}

/// A single-threaded reload command sink for headless Quiver hosts.
///
/// Candidate preparation may queue a command at any time between host calls,
/// but [`Self::process_safe_point`] is the only operation that can install it.
/// The headless driver calls that method only between event and tree operations,
/// making the active generation, callbacks, and root coherent for the complete
/// duration of every operation.
#[cfg(feature = "wasm-hot-reload")]
pub struct HeadlessReloadHost<G: ReloadGuest, R, E> {
    coordinator: ReloadCoordinator<G, R, E>,
    pending_command: Option<ReloadCommand<G, R>>,
    request_frame: Box<dyn Fn()>,
}

#[cfg(feature = "wasm-hot-reload")]
impl<G: ReloadGuest, R, E> HeadlessReloadHost<G, R, E> {
    /// Creates a safe-point host with an explicit bounded event capacity.
    #[inline]
    pub fn new(
        active: ReloadSnapshot<G, R>,
        max_queued_events: usize,
        request_frame: impl Fn() + 'static,
    ) -> Self {
        Self {
            coordinator: ReloadCoordinator::new(active).max_queued_events(max_queued_events),
            pending_command: None,
            request_frame: Box::new(request_frame),
        }
    }

    /// Opens the event barrier and supersedes any command not yet committed.
    pub fn begin_reload(&mut self) -> ReloadTransactionId {
        self.pending_command.take();
        self.coordinator.begin_reload()
    }

    /// Queues one complete candidate without changing the active snapshot.
    pub fn queue_command(
        &mut self,
        command: ReloadCommand<G, R>,
    ) -> Result<(), ReloadTransactionError> {
        let current = self.coordinator.current_transaction();
        if current != Some(command.transaction) {
            let error = match current {
                Some(current) => ReloadTransactionError::Superseded {
                    supplied: command.transaction,
                    current,
                },
                None => ReloadTransactionError::NoTransaction,
            };
            return Err(error);
        }
        self.pending_command = Some(command);
        Ok(())
    }

    /// Borrows the coherent snapshot used by the next host operation.
    #[inline]
    pub const fn active(&self) -> &ReloadSnapshot<G, R> {
        self.coordinator.active()
    }

    /// Mutably borrows the coherent snapshot for one serialized host operation.
    #[inline]
    pub const fn active_mut(&mut self) -> &mut ReloadSnapshot<G, R> {
        self.coordinator.active_mut()
    }

    /// Returns whether the host owns a candidate waiting for its safe point.
    ///
    /// This diagnostic reports only bounded command ownership; it does not
    /// inspect guest state or execute candidate code. Reliability harnesses use
    /// it to verify that commit, rejection, supersession, and cancellation
    /// return the host to its candidate-free baseline.
    #[inline]
    pub const fn has_pending_command(&self) -> bool {
        self.pending_command.is_some()
    }

    /// Retains an event while preparation is active or returns it for dispatch.
    #[inline]
    pub fn route_event(
        &mut self,
        event: E,
    ) -> Result<ReloadEventDisposition<E>, ReloadEventOverflow<E>> {
        self.coordinator.route_event(event)
    }

    /// Commits the queued command between host event/tree operations.
    ///
    /// `preflight` is the final fallible operation and must be side-effect free.
    /// `commit` runs only after it succeeds and must perform only the infallible
    /// native state carry prepared by reconciliation. Exactly one frame is
    /// requested after a successful installation; an empty queue requests none.
    pub fn process_safe_point<P>(
        &mut self,
        preflight: impl FnOnce(&ReloadSnapshot<G, R>, &ReloadSnapshot<G, R>) -> Result<(), P>,
        commit: impl FnOnce(&mut ReloadSnapshot<G, R>, &mut ReloadSnapshot<G, R>),
    ) -> Result<Option<ReloadCommit<E>>, ReloadCommitError<P, E>> {
        let Some(command) = self.pending_command.take() else {
            return Ok(None);
        };
        self.coordinator
            .stage_candidate(command.transaction, command.candidate)
            .map_err(ReloadCommitError::Transaction)?;
        let committed = self
            .coordinator
            .commit(command.transaction, preflight, commit)?;
        (self.request_frame)();
        Ok(Some(committed))
    }

    /// Rejects the open transaction and releases its events to the active host.
    pub fn rollback(
        &mut self,
        transaction: ReloadTransactionId,
    ) -> Result<aimer_anteros::ReloadReplay<E>, ReloadTransactionError> {
        self.pending_command.take();
        self.coordinator.rollback(transaction)
    }
}

#[cfg(feature = "wasm-hot-reload")]
impl<G: ReloadGuest, E> HeadlessReloadHost<G, AnyElement, E> {
    /// Installs a disconnected Aimer element tree at the current safe point.
    ///
    /// Reconciliation planning and validation complete before native state or
    /// identities move from the active tree. The same unchanged pair is then
    /// committed synchronously, so the second validation cannot normally fail;
    /// such a failure indicates a violated single-threaded host invariant.
    pub fn process_element_safe_point(
        &mut self,
        ctx: &BuildContext,
    ) -> Result<Option<ReloadCommit<E>>, ReloadCommitError<ReconciliationPlanError, E>> {
        self.process_safe_point(
            |old, candidate| {
                plan_element_reconciliation(old.root().as_ref(), candidate.root().as_ref())
                    .validate()
            },
            |old, candidate| {
                plan_element_reconciliation(old.root().as_ref(), candidate.root().as_ref())
                    .commit(ctx)
                    .expect("validated reconciliation plan changed during safe-point commit");
            },
        )
    }
}

pub(crate) struct FramePreparation {
    scroll_result: EventResult,
    rebuild_generation: u64,
    layout_generation: u64,
}

/// The inputs, other than the render tree's own sync stamp, that a frame
/// reads and that can change without the tree being rebuilt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnimationBaseline {
    /// The rebuild generation less the advances that only kept an animating
    /// element reachable, which every frame of an animation makes.
    rebuild_generation: u64,
    texture_epoch: u64,
    /// The pointer position, in bits. Elements such as scroll bars and mouse
    /// regions read it while updating, so a pointer that moved needs the walk.
    cursor_bits: (u32, u32),
}

impl AnimationBaseline {
    fn new(rebuild_generation: u64, texture_epoch: u64, cursor: Vec2d) -> Self {
        Self {
            rebuild_generation,
            texture_epoch,
            cursor_bits: (cursor.x.to_bits(), cursor.y.to_bits()),
        }
    }
}

/// How often each kind of synchronization ran.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SyncCounts {
    pub(crate) full: u64,
    pub(crate) scoped: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RenderTreeSyncStamp {
    /// Advances of the tree generation that no single replaced subtree
    /// describes; the replacements themselves are synchronized separately.
    unscoped_element_generation: u64,
    render_structure_generation: u64,
    layout_generation: u64,
    target_size: (u32, u32),
    scale_bits: u32,
    root: Option<ElementId>,
}

struct AddedRenderNode<'element, 'context> {
    // Includes existing direct parents whose new child slots need state sync.
    element: &'element dyn Element,
    render_node: RenderNodeId,
    context: BuildContext<'context>,
    element_path: Vec<ElementId>,
}

struct RenderTreeSyncResult<'element, 'context> {
    refreshed: bool,
    added_nodes: bool,
    added_render_nodes: Vec<AddedRenderNode<'element, 'context>>,
}

impl<'element, 'context> Default for RenderTreeSyncResult<'element, 'context> {
    fn default() -> Self {
        Self {
            refreshed: false,
            added_nodes: false,
            added_render_nodes: Vec::new(),
        }
    }
}

/// Per-window retained render ownership and its stable element mapping.
#[derive(Default)]
pub(crate) struct WindowRenderTree {
    tree: RenderTree,
    element_nodes: Rc<ElementNodeMap>,
    synced: Option<RenderTreeSyncStamp>,
    /// What the last synchronization produced, for the scoped one to compare
    /// against. Dropped whenever the tree is not known to match it.
    sync_cache: Option<scoped_sync::SyncCache>,
    /// How far into the log of replaced subtrees this window has synchronized.
    scoped_cursor: aimer_widget::ScopedCursor,
    /// How far into the log of replaced subtrees the last walk or pass got. Unset
    /// until the first frame, whose build is not a rebuild.
    walk_cursor: Option<aimer_widget::ScopedCursor>,
    /// Makes every synchronization a full one. Tests set it to compare the two.
    pub(crate) scoped_sync_disabled: bool,
    /// How many synchronizations took each path, for tests and diagnostics.
    pub(crate) sync_counts: SyncCounts,
    /// What the last frame was drawn from. A frame that finds any of it moved
    /// cannot be animation-only.
    animation_baseline: Option<AnimationBaseline>,
    /// Whether the last frame held a draw the renderer cannot take through its
    /// persistent target, such as a glass panel. The renderer draws such a frame
    /// from scratch from its render plan, and leaves the target stale, so the
    /// frame after it needs the whole tree as well.
    drew_custom_pipeline: bool,
    /// Elements drawn in the last frame that still had no render node after its
    /// final sync (see [`census::PaintSourceCensus::drawn_unmapped`]).
    drawn_unmapped: Vec<census::CensusElement>,
    /// Elements that had no retained paint to show in some frame: they declined
    /// (`can_paint_local_v2` was false) or their state was not valid. One entry
    /// per element, accumulated.
    declined_paint: Vec<census::CensusElement>,
    /// Why the last [`Self::sync`] failed, if it did. A failed sync leaves the
    /// frame on the full legacy repaint path.
    sync_error: Option<RenderTreeError>,
    /// Elements whose bounds or clip were not finite at the last sync. They are
    /// collapsed to an empty rectangle so one bad node cannot fail the whole
    /// sync, and reported here so the bug stays visible.
    invalid_bounds: Vec<census::CensusElement>,
    /// How many elements took their hit area from the render tree at the last
    /// sync.
    interaction_adopted: usize,
    /// The same count by element type, so a gate can require each migrated
    /// widget to have been exercised rather than passing vacuously.
    interaction_adopted_by_type: std::collections::BTreeMap<&'static str, usize>,
    /// The element-tree generation and render-tree geometry revision the
    /// accessibility tree was last published at, so an unchanged frame is not
    /// collected again.
    accessibility_stamp: Option<(u64, u64)>,
    /// The last census that was reported, so only changes are logged.
    #[cfg(feature = "frame-stats")]
    last_census: Option<census::PaintSourceCensus>,
}

impl WindowRenderTree {
    /// Synchronizes the render tree with the element tree, remembering why it
    /// failed when it does (see [`census::PaintSourceCensus::sync_error`]).
    fn sync<'element, 'context>(
        &mut self,
        root: Option<&'element AnyElement>,
        ctx: &BuildContext<'context>,
        target_size: (u32, u32),
        scale: f32,
    ) -> Result<RenderTreeSyncResult<'element, 'context>, RenderTreeError> {
        let result = self.sync_inner(root, ctx, target_size, scale);
        self.sync_error = result.as_ref().err().copied();
        result
    }

    fn sync_stamp(
        root: Option<&AnyElement>,
        target_size: (u32, u32),
        scale: f32,
    ) -> RenderTreeSyncStamp {
        RenderTreeSyncStamp {
            unscoped_element_generation: aimer_widget::unscoped_element_tree_generation(),
            render_structure_generation: aimer_widget::retained_render_structure_generation(),
            layout_generation: aimer_widget::layout_invalidation_generation(),
            target_size,
            scale_bits: scale.to_bits(),
            root: root.map(Element::id),
        }
    }

    /// Whether the last synchronization succeeded and nothing it depends on has
    /// moved since, other than subtrees replaced by ones of the same shape.
    ///
    /// Those replacements are what the next synchronization is for, and it
    /// handles them without collecting the whole tree. It may still decide it
    /// cannot, so a caller that relies on this must check that the
    /// synchronization it then runs stayed on the scoped path.
    fn is_synchronized_apart_from_replacements(
        &self,
        root: &AnyElement,
        target_size: (u32, u32),
        scale: f32,
    ) -> bool {
        scale.is_finite()
            && scale > 0.0
            && self.synced == Some(Self::sync_stamp(Some(root), target_size, scale))
    }

    fn sync_inner<'element, 'context>(
        &mut self,
        root: Option<&'element AnyElement>,
        ctx: &BuildContext<'context>,
        target_size: (u32, u32),
        scale: f32,
    ) -> Result<RenderTreeSyncResult<'element, 'context>, RenderTreeError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(RenderTreeError::InvalidBounds);
        }
        let stamp = Self::sync_stamp(root, target_size, scale);
        if self.synced == Some(stamp) {
            if !aimer_widget::has_scoped_rebuilds_since(self.scoped_cursor) {
                return Ok(RenderTreeSyncResult::default());
            }
            // Only replaced subtrees changed since the last synchronization,
            // unless the log no longer reaches back that far.
            let dirty = aimer_widget::scoped_rebuilds_since(self.scoped_cursor);
            self.scoped_cursor = aimer_widget::scoped_rebuild_cursor();
            if let (Some(root), Some(dirty)) = (root, dirty)
                && self.sync_scoped(root, ctx, scale, &dirty)
            {
                self.sync_counts.scoped += 1;
                return Ok(RenderTreeSyncResult {
                    refreshed: true,
                    ..RenderTreeSyncResult::default()
                });
            }
        }
        // The full synchronization below sees every change, so replacements
        // logged so far describe nothing it has not already seen.
        self.scoped_cursor = aimer_widget::scoped_rebuild_cursor();
        self.sync_cache = None;
        self.sync_counts.full += 1;

        let mut specs = Vec::new();
        let mut element_ids = Vec::new();
        let mut clips = Vec::new();
        let mut clip_radii = Vec::new();
        let mut invalid_bounds = Vec::new();
        let mut new_elements = Vec::new();
        let mut interactive = Vec::new();
        if let Some(root) = root {
            let mut element_path = Vec::new();
            collect_render_tree_nodes(
                root.as_ref(),
                None,
                ctx,
                scale,
                &self.element_nodes,
                &mut specs,
                &mut element_ids,
                &mut clips,
                &mut clip_radii,
                &mut invalid_bounds,
                &mut new_elements,
                &mut interactive,
                None,
                [0.0; 4],
                (0.0, 0.0),
                &mut element_path,
            );
        }
        let added_nodes = element_ids
            .iter()
            .any(|element_id| !self.element_nodes.contains_key(element_id));
        let render_ids = self
            .tree
            .sync_structure_with_clip_radii(&specs, &clips, &clip_radii)?;
        self.sync_cache = Some(scoped_sync::SyncCache::new(
            element_ids.clone(),
            specs,
            clips,
            clip_radii,
            render_ids.clone(),
        ));
        self.element_nodes = Rc::new(
            element_ids
                .into_iter()
                .zip(render_ids.iter().copied())
                .collect(),
        );
        self.synced = Some(stamp);
        self.invalid_bounds = invalid_bounds;
        self.adopt_interaction_bounds(&interactive, &render_ids, scale);
        self.interaction_adopted = interactive.len();
        self.interaction_adopted_by_type.clear();
        for (_, element, _, _) in &interactive {
            *self
                .interaction_adopted_by_type
                .entry(element.debug_name())
                .or_default() += 1;
        }
        Ok(RenderTreeSyncResult {
            refreshed: true,
            added_nodes,
            added_render_nodes: new_elements
                .into_iter()
                .map(|(index, element, context, element_path)| AddedRenderNode {
                    element,
                    render_node: render_ids[index],
                    context,
                    element_path,
                })
                .collect(),
        })
    }

    /// Hands every element that opted into render-tree interaction bounds a
    /// live reference to its render node, plus the interaction size it reported
    /// while the tree was collected (logical pixels). The element reads the
    /// node's world rectangle when it is hit-tested, so a scroll that moves the
    /// node without a new synchronization is followed.
    fn adopt_interaction_bounds(
        &self,
        interactive: &[(usize, &dyn Element, ResolvedSize, (f32, f32))],
        render_ids: &[RenderNodeId],
        scale: f32,
    ) {
        for (index, element, size, offset) in interactive {
            let Some(node) = render_ids.get(*index) else {
                continue;
            };
            element.adopt_retained_v2_interaction_source(
                aimer_widget::InteractionSource::new(
                    self.tree.clone(),
                    *node,
                    (size.width / scale, size.height / scale),
                )
                .with_offset((offset.0 / scale, offset.1 / scale)),
            );
        }
    }

    /// Returns the retained tree owned by this window handler.
    #[inline]
    pub fn tree(&self) -> &RenderTree {
        &self.tree
    }

    /// Returns the render node currently associated with a stable element ID.
    #[inline]
    pub fn node_for_element(&self, element: ElementId) -> Option<RenderNodeId> {
        self.element_nodes.get(&element).copied()
    }

    fn covers_unbounded_stateful_invalidations(
        &self,
        invalidations: &ElementInvalidationBatch,
    ) -> bool {
        // A rebuild whose source is not tracked cannot say which elements it
        // touches, but it does not have to: every element it reconciles is
        // marked for a new recording, each recording that differs from the last
        // damages its own node (`DrawListWriter::commit`), and nodes added or
        // removed damage themselves in the render-tree sync. Nothing a rebuild
        // changes escapes those three, so the target need not be repainted whole.
        //
        // An element that went missing or whose identity could not be resolved
        // is the case they cannot see, and stays on the full path. So does any
        // change that is not a plain rebuild.
        let mut found_unbounded_state = false;
        for invalidation in invalidations
            .records()
            .iter()
            .filter(|record| record.requires_full_fallback)
        {
            if invalidation.change != ElementChangeKind::Unknown
                || invalidation.stale_id
                || invalidation.removed
            {
                return false;
            }
            found_unbounded_state = true;
        }
        found_unbounded_state
    }
}

/// Whether `rect` is something the render tree accepts: every coordinate is
/// finite and the extent is not negative.
#[inline]
fn render_rect_is_valid(rect: &RenderRect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

/// Where one element sits in the render tree, before validity checks.
struct NodeFrame {
    position: Vec2d,
    size: ResolvedSize,
    /// In the parent's coordinate space, with paint outsets and the parent's
    /// origin shift applied.
    bounds: RenderRect,
    clip: Option<RenderRect>,
    /// How far this node's origin sits outside its layout box.
    own_outset: (f32, f32),
}

/// Computes the geometry the render tree stores for `element`.
///
/// `geometry` is what the parent decided for this child, if it decided
/// anything; `origin_shift` is how far the parent's node origin sits outside
/// the parent's layout box.
fn node_frame(
    element: &dyn Element,
    ctx: &BuildContext<'_>,
    scale: f32,
    geometry: Option<(RenderRect, Option<RenderRect>)>,
    origin_shift: (f32, f32),
) -> NodeFrame {
    let position = element.pos().unwrap_or_default();
    let size = element
        .retained_v2_bounds(ctx)
        .unwrap_or_else(|| element.content_size(ctx));
    let (mut bounds, clip) = geometry.unwrap_or_else(|| {
        (
            RenderRect::new(
                position.x / scale,
                position.y / scale,
                size.width / scale,
                size.height / scale,
            ),
            element.retained_clip(ctx),
        )
    });
    bounds = RenderRect::new(
        bounds.x + origin_shift.0,
        bounds.y + origin_shift.1,
        bounds.width,
        bounds.height,
    );
    let mut own_outset = (0.0, 0.0);
    let mut clip = clip;
    if let Some([left, top, right, bottom]) = element.retained_v2_paint_outsets(ctx)
        && [left, top, right, bottom]
            .iter()
            .all(|outset| outset.is_finite() && *outset >= 0.0)
    {
        own_outset = (left, top);
        bounds = RenderRect::new(
            bounds.x - left,
            bounds.y - top,
            bounds.width + left + right,
            bounds.height + top + bottom,
        );
        // A clip is local to the node, whose origin is now the grown corner,
        // while the parent stated it from the layout box. Move it in by the
        // growth or it lands that far up and to the left of what it clips.
        clip = clip.map(|clip| {
            RenderRect::new(clip.x + left, clip.y + top, clip.width, clip.height)
        });
    }
    NodeFrame {
        position,
        size,
        bounds,
        clip,
        own_outset,
    }
}

/// The context `element` hands down to children that it does not give a
/// context of their own.
fn child_base_context<'c>(
    element: &dyn Element,
    ctx: &BuildContext<'c>,
    size: ResolvedSize,
    position: Vec2d,
) -> BuildContext<'c> {
    let mut child_base_ctx = ctx.clone();
    child_base_ctx.parent_size = size;
    child_base_ctx.parent_pos = Vec2d {
        x: ctx.parent_pos.x + position.x,
        y: ctx.parent_pos.y + position.y,
    };
    // A transparent wrapper hands `draw`'s own context to its child. It adds
    // nothing of its own, so its size is exactly what its only child measures
    // under the constraint it received. Feeding that size back down as the
    // child's constraint would shrink the child again at every wrapper level
    // (a padded descendant measures smaller than the space it was given).
    let mut retained_children = 0;
    let mut only_child_size = None;
    element.visit_retained_v2_children(&mut |_, child| {
        retained_children += 1;
        if retained_children == 1 {
            only_child_size = Some(child.content_size(ctx));
        }
    });
    let is_transparent =
        retained_children == 1 && only_child_size.is_some_and(|child| child == size);
    if is_transparent {
        child_base_ctx.box_constraint = ctx.box_constraint;
        child_base_ctx.parent_size = ctx.parent_size;
    } else {
        child_base_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
    }
    child_base_ctx
}

/// The context an element hands to the children it gives none of their own,
/// derived the first time a child asks for it.
///
/// Deriving it measures the element's only child, in the element's own
/// constraint, to tell a transparent wrapper from one that adds size. Most
/// elements that have one child give it a context (a container hands down the
/// space inside its padding), so the base context is never read, and measuring
/// for it would measure everything below the element a second time, in a space
/// nothing is laid out in.
struct ChildBaseContext<'e, 'a, 'c> {
    element: &'e dyn Element,
    ctx: &'a BuildContext<'c>,
    size: ResolvedSize,
    position: Vec2d,
    derived: Option<BuildContext<'c>>,
}

impl<'e, 'a, 'c> ChildBaseContext<'e, 'a, 'c> {
    fn new(
        element: &'e dyn Element,
        ctx: &'a BuildContext<'c>,
        size: ResolvedSize,
        position: Vec2d,
    ) -> Self {
        Self {
            element,
            ctx,
            size,
            position,
            derived: None,
        }
    }

    fn get(&mut self) -> &BuildContext<'c> {
        self.derived.get_or_insert_with(|| {
            child_base_context(self.element, self.ctx, self.size, self.position)
        })
    }
}

/// What a parent hands one retained child: its geometry override, the radius of
/// the clip it supplies, and the context to lay the child out in.
struct ChildInput<'c> {
    geometry: Option<(RenderRect, Option<RenderRect>)>,
    clip_radius: [f32; 4],
    ctx: BuildContext<'c>,
}

fn child_input<'c>(
    element: &dyn Element,
    ctx: &BuildContext<'c>,
    child_base_ctx: &mut ChildBaseContext<'_, '_, 'c>,
    child: &dyn Element,
    index: usize,
) -> ChildInput<'c> {
    let provided = element.retained_v2_child_context_at(ctx, child, index);
    // A child that was measured keeps the size of its last measurement, and the
    // geometry below measures it again in the space it is laid out in. The tree
    // reads the last one, so the measurement for the base context comes first.
    let child_ctx = provided.unwrap_or_else(|| child_base_ctx.get().clone());
    let geometry = element.retained_v2_child_geometry_at(ctx, child, index);
    let clip_radius = if geometry.is_some() {
        element.retained_v2_child_clip_radius(ctx, child)
    } else {
        [0.0; 4]
    };
    ChildInput {
        geometry,
        clip_radius,
        ctx: child_ctx,
    }
}

fn collect_render_tree_nodes<'element, 'context>(
    element: &'element dyn Element,
    parent_index: Option<usize>,
    ctx: &BuildContext<'context>,
    scale: f32,
    existing: &ElementNodeMap,
    specs: &mut Vec<RenderNodeSpec>,
    element_ids: &mut Vec<ElementId>,
    clips: &mut Vec<Option<RenderRect>>,
    clip_radii: &mut Vec<[f32; 4]>,
    invalid_bounds: &mut Vec<census::CensusElement>,
    new_elements: &mut Vec<(
        usize,
        &'element dyn Element,
        BuildContext<'context>,
        Vec<ElementId>,
    )>,
    interactive: &mut Vec<(usize, &'element dyn Element, ResolvedSize, (f32, f32))>,
    geometry: Option<(RenderRect, Option<RenderRect>)>,
    geometry_clip_radius: [f32; 4],
    // How far the parent's node origin sits outside its layout box, because
    // its bounds grew outward to cover its paint. Children are positioned
    // relative to the layout box, so they move back in by this much.
    origin_shift: (f32, f32),
    element_path: &mut Vec<ElementId>,
) {
    let element_id = element.id();
    element_path.push(element_id);
    let NodeFrame {
        position,
        size,
        bounds,
        clip,
        own_outset,
    } = node_frame(element, ctx, scale, geometry, origin_shift);
    // A node whose size or position is not a finite, non-negative rectangle
    // (for example a percentage of an unbounded axis) has nothing drawable.
    // Collapse it instead of failing the whole synchronization, which would put
    // every other element of the window on the legacy repaint path.
    let bounds_valid = render_rect_is_valid(&bounds);
    let clip_valid = clip.as_ref().is_none_or(render_rect_is_valid);
    let (bounds, clip) = if bounds_valid && clip_valid {
        (bounds, clip)
    } else {
        invalid_bounds.push(census::CensusElement {
            element: element_id,
            debug_name: element.debug_name(),
        });
        (
            if bounds_valid {
                bounds
            } else {
                RenderRect::new(0.0, 0.0, 0.0, 0.0)
            },
            clip.map(|clip| {
                if render_rect_is_valid(&clip) {
                    clip
                } else {
                    RenderRect::new(0.0, 0.0, 0.0, 0.0)
                }
            }),
        )
    };
    let node_index = specs.len();
    if let Some(interaction_size) = element.retained_v2_interaction_size(ctx) {
        // The node starts `own_outset` (logical) outside the element's layout
        // box, so the interaction rectangle starts that far inside the node.
        let offset = element.retained_v2_interaction_offset(ctx);
        interactive.push((
            node_index,
            element,
            interaction_size,
            (
                offset.0 + own_outset.0 * scale,
                offset.1 + own_outset.1 * scale,
            ),
        ));
    }
    let is_new = !existing.contains_key(&element_id);
    if is_new {
        new_elements.push((node_index, element, ctx.clone(), element_path.clone()));
    }
    specs.push(RenderNodeSpec::new(
        existing.get(&element_id).copied(),
        parent_index,
        bounds,
    ));
    element_ids.push(element_id);
    clips.push(clip);
    // The radius belongs to the clip the parent supplied with the geometry; an
    // element's own `retained_clip` is rectangular, and a radius without a clip
    // would only create spurious clip changes.
    clip_radii.push(if geometry.is_some() && clip.is_some() {
        geometry_clip_radius
    } else {
        [0.0; 4]
    });

    let mut child_base_ctx = ChildBaseContext::new(element, ctx, size, position);

    let mut has_new_child = false;
    element.visit_retained_v2_children(&mut |index, child| {
        has_new_child |= !existing.contains_key(&child.id());
        let ChildInput {
            geometry: child_geometry,
            clip_radius: child_clip_radius,
            ctx: child_ctx,
        } = child_input(element, ctx, &mut child_base_ctx, child, index);
        collect_render_tree_nodes(
            child,
            Some(node_index),
            &child_ctx,
            scale,
            existing,
            specs,
            element_ids,
            clips,
            clip_radii,
            invalid_bounds,
            new_elements,
            interactive,
            child_geometry,
            child_clip_radius,
            own_outset,
            element_path,
        );
    });
    if !is_new && has_new_child && element.can_paint_local_v2(ctx) {
        // This owner's previous synchronization saw the old child slots.
        // Reapply its presentation after the new children have render nodes.
        new_elements.push((node_index, element, ctx.clone(), element_path.clone()));
    }
    element_path.pop();
}

fn record_new_local_v2_paints<'element, 'context>(
    tree: &RenderTree,
    new_nodes: Vec<AddedRenderNode<'element, 'context>>,
) -> bool {
    if new_nodes
        .iter()
        .any(|node| !node.element.can_paint_local_v2(&node.context))
    {
        return false;
    }

    for node in new_nodes {
        let Ok(paint_context) = tree.context(node.render_node) else {
            return false;
        };
        aimer_widget::set_rebuild_source_path(node.element, &node.element_path);
        let resolved = node
            .context
            .with_local_v2_paint_context(paint_context, |ctx| {
                // A child created during drawing missed the normal mapped-node
                // state pass. Cached pixels must not appear at default opacity
                // before their animation's first sample is applied.
                if !node.element.sync_local_v2_state(ctx) {
                    return false;
                }
                if tree.needs_recording(node.render_node).unwrap_or(true)
                    || node.element.local_v2_paint_needs_recording(ctx)
                {
                    node.element.paint_local_v2(ctx);
                }
                true
            });
        if !resolved {
            return false;
        }
        if tree
            .set_paint_source(node.render_node, RenderPaintSource::LocalV2)
            .is_err()
        {
            return false;
        }
    }

    true
}

pub struct AimerApplicationHandler<W: Widget + 'static> {
    /// The window this application draws into and asks for frames.
    ///
    /// A [`WindowHandle`] rather than a `Window`, because a headless
    /// application has no platform window and still has to answer the very
    /// questions the event handlers ask of one: repaint, change the cursor,
    /// report its metrics. Keeping the handle here is what lets both drivers
    /// run the same event and frame code.
    pub window: Option<WindowHandle>,
    /// Whether a host with its own loop owns this window.
    ///
    /// A hosted window that finishes closing says so (see [`Self::closed`])
    /// and leaves it to the host to decide whether the application ends with
    /// it. An unhosted one is the whole application, and ends the loop.
    pub(crate) hosted: bool,
    /// Set once the window has stopped its renderer and is ready to be removed.
    pub(crate) closed: bool,
    /// The providers this window's tree starts with, when it was opened to
    /// share those of the window that opened it.
    pub(crate) inherited_states:
        Option<Rc<std::cell::RefCell<aimer_widget::components::context::InheritedStates>>>,
    pub window_attr: WindowAttr,
    /// Delays a requested visible state until the first DX12 present succeeds.
    #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
    pub(crate) show_window_after_first_frame: bool,
    pub(crate) macos_windowing: aimer_native::macos_windowing::MacosWindowing,
    pub render_ctx: AimerRenderContext,
    /// Retained render nodes and stable element identities for this window.
    pub(crate) render_tree: WindowRenderTree,
    /// Per-application heap used by retained widget and element allocations.
    pub(crate) ui_memory: UiMemory,
    #[cfg(debug_assertions)]
    pub(crate) debug_overlay: crate::debug_overlay::DebugOverlay,
    pub widget_root: Option<AnyElement>,
    pub event_dispatcher: EventDispatcher,
    pub(crate) scroll_smoother: DualScroller,
    /// Reason merged into the redraw currently waiting for this application.
    pub(crate) frame_request_reason: Rc<Cell<Option<FrameRequestKind>>>,
    /// Gesture boundaries inferred for the phase-less browser wheel stream.
    #[cfg(target_arch = "wasm32")]
    pub(crate) web_scroll_phase: crate::handler::web_scroll_phase::WebScrollPhase,
    pub pending_widget: Option<W>,
    pub cursor_pos: Vec2d,
    /// Whether the last pointer input came from a finger.
    ///
    /// A touch has to move [`Self::cursor_pos`], so the wheel and trackpad
    /// frames that arrive while a finger is down are routed to what is under
    /// it. It must not make anything *hovered*: a finger is not a cursor, it
    /// does not rest on what it last touched, and nothing tells the application
    /// when it leaves. Hover is read from [`Self::hover_pos`] instead.
    pub(crate) touch_pointer: bool,
    /// The mouse button currently held, if any.
    ///
    /// The platform reports a button only when it changes state, but a move or a
    /// release during a drag has to carry the button that started the drag —
    /// otherwise every move looks like a primary-button move and a
    /// secondary-button drag loses its identity halfway through.
    pub pressed_button: Option<aimer_events::pointer::PointerButton>,
    pub current_modifiers: aimer_events::element::Modifiers,
    pub ime_composing: bool,
    pub window_scale: f64,
    /// Last observed window size in physical pixels, independent of screen resolution.
    pub native_window_size: Option<ResolvedSize>,
    pub pending_resize: Option<PhysicalSize<u32>>,
    pub(crate) startup_hooks: Vec<StartupHook>,
    pub(crate) startup_resources: Vec<Box<dyn Any>>,
    pub active_touch_id: Option<u64>,
    /// The UI-thread runtime this application's frames are scheduled by.
    ///
    /// Held here rather than reached for through [`Venus::current`] because a
    /// frame drives the runtime it owns: a second window on a second thread is
    /// a second application, and it must drive its own.
    pub venus: Rc<Venus>,
    /// A handle to the application's async runtime.
    ///
    /// A handle rather than the runtime: windows share one, and whoever
    /// started the application owns it and outlives every window.
    #[cfg(not(target_arch = "wasm32"))]
    pub async_handle: Handle,
    /// The batch of files being dragged over the window, so the drag can be
    /// re-reported for every position it is found at rather than only for the
    /// one it came in at.
    pub(crate) file_drag: FileDrag,
    #[cfg(feature = "wasm-hot-reload")]
    pub(crate) live_reload: Option<crate::hot_reload::LiveReloadHost>,
}

impl<W: Widget + 'static> AimerApplicationHandler<W> {
    #[cfg(debug_assertions)]
    pub(crate) fn handle_debug_key(
        &mut self,
        key: &winit::keyboard::Key,
        state: winit::event::ElementState,
        repeat: bool,
    ) -> bool {
        let modifiers = &self.current_modifiers;
        let modified = modifiers.ctrl || modifiers.meta || modifiers.alt || modifiers.shift;
        let visible_before = self.debug_overlay.is_visible();
        let consumed = self.debug_overlay.handle_key(key, state, repeat, modified);
        if visible_before != self.debug_overlay.is_visible() {
            self.request_full_redraw();
        }
        consumed
    }

    /// Counts the active root's render nodes by paint source, or `None` before a
    /// root is mounted. Tests use it to assert that a page paints without legacy
    /// islands and without elements the render tree cannot see.
    #[doc(hidden)]
    pub fn paint_source_census(&self) -> Option<census::PaintSourceCensus> {
        self.active_root()
            .map(|root| self.render_tree.paint_source_census(root.as_ref()))
    }

    /// Compares every element's cached interaction rectangle with its render
    /// node's world rectangle, or `None` before a root is mounted.
    #[doc(hidden)]
    pub fn bounds_audit(&self) -> Option<bounds_audit::BoundsAudit> {
        self.active_root()
            .map(|root| self.render_tree.bounds_audit(root.as_ref()))
    }

    /// Borrows the one root currently visible to input and rendering.
    #[inline]
    pub(crate) fn active_root(&self) -> Option<&AnyElement> {
        #[cfg(feature = "wasm-hot-reload")]
        if let Some(root) = self
            .live_reload
            .as_ref()
            .and_then(crate::hot_reload::LiveReloadHost::active_root)
        {
            return Some(root);
        }
        self.widget_root.as_ref()
    }

    /// Returns the retained render tree associated with this window.
    #[inline]
    pub fn render_tree(&self) -> &RenderTree {
        self.render_tree.tree()
    }

    /// Looks up the retained render node for a stable element identity.
    #[inline]
    pub fn render_node_for_element(&self, element: ElementId) -> Option<RenderNodeId> {
        self.render_tree.node_for_element(element)
    }

    /// The platform window behind this application, if it has one.
    ///
    /// Only code that talks to the platform itself — surface creation, native
    /// appearance queries, AppKit drag polling — needs this; everything else
    /// goes through [`window`](Self::window) and works headlessly too.
    #[inline]
    pub(crate) fn native_window(&self) -> Option<&Arc<Window>> {
        self.window.as_ref().and_then(WindowHandle::native_window)
    }

    /// Tells a headless window the metrics the platform would already have
    /// given a real one.
    ///
    /// `size` is the size the window has just become, or `None` to keep the
    /// one it has and refresh the scale alone. A native window answers for
    /// itself and is left untouched.
    pub(crate) fn sync_headless_metrics(&self, size: Option<PhysicalSize<u32>>) {
        let Some(window) = &self.window else { return };
        let size = size.unwrap_or_else(|| window.inner_size());
        window.update_headless_metrics(size, self.window_scale);
    }

    /// Asks for the frame that continues an animation.
    ///
    /// Both native and headless applications go through the thread-local
    /// requester. Native setup installs the platform wake there; headless setup
    /// installs a requester that records the request on its in-memory window.
    /// Keeping one route also lets nested callers replace the requester while
    /// they test or embed an application.
    #[inline]
    pub(crate) fn request_animation_frame(&self) {
        aimer_events::window::request_animation_frame();
    }

    pub(crate) fn note_frame_request(&self, kind: FrameRequestKind) {
        let kind = self
            .frame_request_reason
            .get()
            .map_or(kind, |pending| pending.merge(kind));
        self.frame_request_reason.set(Some(kind));
    }

    pub(crate) fn take_frame_request_reason(&self) -> FrameRequestKind {
        self.frame_request_reason
            .take()
            .unwrap_or(FrameRequestKind::Full)
    }

    /// Records that this window has finished closing and, when nothing hosts
    /// it, ends the event loop.
    pub(crate) fn window_finished(&mut self, event_loop: &ActiveEventLoop) {
        self.closed = true;
        if !self.hosted {
            event_loop.exit();
        }
    }

    pub(crate) fn request_full_redraw(&self) {
        self.note_frame_request(FrameRequestKind::Full);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Rebuilds the widgets that read this window's metrics, after they changed.
    ///
    /// Other windows on the thread have metrics of their own, and a change to
    /// this one is not a change to them.
    pub(crate) fn notify_window_metrics_changed(&self) {
        match &self.window {
            Some(window) => {
                aimer_widget::window_metrics::notify_window_metrics_changed_for(window);
            }
            None => {
                aimer_widget::notify_window_metrics_changed();
            }
        }
    }

    /// Tells the runtime how fast the display it is drawing on actually is.
    ///
    /// winit reports the rate in millihertz, and only for a monitor it can
    /// identify: a window that has not been placed yet, a headless compositor,
    /// or a platform with no notion of a monitor all answer `None`. In that case
    /// nothing is said and the runtime keeps the rate it has, which is strictly
    /// better than dividing by a number the platform declined to give.
    fn tune_runtime_to_display(&self, window: &Window) {
        let Some(millihertz) = window
            .current_monitor()
            .or_else(|| window.primary_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz())
        else {
            return;
        };

        self.venus.set_refresh_rate(millihertz as f32 / 1_000.0);
    }

    /// The bookkeeping every frame does before anything is drawn.
    ///
    /// This frame's share of a scroll gesture is delivered here, and a gesture
    /// that is not finished asks for the frame that continues it — which is
    /// what keeps momentum alive without a platform timer. Shared by the
    /// windowed loop and the headless application so a frame costs the same
    /// work in both.
    pub(crate) fn begin_frame(&mut self) -> FramePreparation {
        let rebuild_generation = aimer_widget::rebuild_invalidation_generation();
        let layout_generation = aimer_widget::layout_invalidation_generation();

        // The budget starts here, because everything after this point is spent
        // out of this frame's time.
        self.venus.begin_frame();

        let mut need_redraw = false;

        // A browser never reports the end of a scroll, so the gesture is closed
        // here once its stream has gone quiet — before this frame's step is
        // dispatched, so the terminating phase rides along with it.
        #[cfg(target_arch = "wasm32")]
        if self.web_scroll_phase.poll_idle() {
            self.scroll_smoother.end_gesture();
            need_redraw = true;
        }

        let scroll_result = self.dispatch_smoothed_scroll();

        // An open web gesture keeps the frame loop alive even with no distance
        // left, because the idle poll above only runs on a rendered frame.
        #[cfg(target_arch = "wasm32")]
        let gesture_open = self.web_scroll_phase.is_open();
        #[cfg(not(target_arch = "wasm32"))]
        let gesture_open = false;
        if self.scroll_smoother.is_active() || gesture_open {
            // Whatever distance is still owed rides the next frame. This frame's
            // share was delivered above, once: the smoother covers a whole
            // frame in one step, so delivering again would walk the widget tree
            // a second time for nothing.
            crate::aimer_app::request_scroll_frame(|| self.request_animation_frame());
            need_redraw = true;
        }

        // Animation ticks first, so the values the tree is about to be built
        // from are this frame's; then every resolved effect, drained to
        // exhaustion. Both happen after the input above and before the build
        // below, which is the whole contract: a `set_state` from a future that
        // resolved is visible to *this* frame, not the next one.
        self.venus.run_frame_tasks();
        self.venus.run_microtasks();

        FramePreparation {
            scroll_result,
            rebuild_generation,
            layout_generation,
        }
    }

    pub(crate) fn should_skip_scroll_frame(
        &self,
        kind: FrameRequestKind,
        preparation: &FramePreparation,
        had_pending_resize: bool,
    ) -> bool {
        // Only a frame requested by scroll input can be dropped. One the scroll
        // engine requested for its own motion is advanced by drawing it.
        kind == FrameRequestKind::ScrollOnly
            && !preparation.scroll_result.needs_redraw()
            && self.frame_is_driven_by_scroll_alone(kind, preparation, had_pending_resize)
    }

    /// Whether this frame exists only because content scrolled.
    ///
    /// A widget animation, a state change, a rebuild, a resize or a new root
    /// each merges the frame request into a full one or moves one of the
    /// generations checked here. A frame that passes has nothing to update
    /// except what the scroll offset moves.
    fn frame_is_driven_by_scroll_alone(
        &self,
        kind: FrameRequestKind,
        preparation: &FramePreparation,
        had_pending_resize: bool,
    ) -> bool {
        kind.is_scroll_driven()
            && !had_pending_resize
            && self.pending_widget.is_none()
            && self.frame_request_reason.get() != Some(FrameRequestKind::Full)
            && aimer_widget::rebuild_invalidation_generation() == preparation.rebuild_generation
            && aimer_widget::layout_invalidation_generation() == preparation.layout_generation
    }

    /// Tells the widget tree whether the frame about to be drawn was requested
    /// by scrolling alone, so a container may leave prepared off-screen
    /// children alone. [`end_frame`](Self::end_frame) clears it again.
    pub(crate) fn mark_scroll_only_frame(
        &self,
        kind: FrameRequestKind,
        preparation: &FramePreparation,
        had_pending_resize: bool,
    ) {
        aimer_widget::set_scroll_only_frame(self.frame_is_driven_by_scroll_alone(
            kind,
            preparation,
            had_pending_resize,
        ));
    }

    /// Tells the widget tree whether the frame about to be drawn exists only to
    /// advance compositor animations.
    ///
    /// The mark is only the frame loop's half of the decision: drawing still
    /// requires that no generation moved, that no element reported a change and
    /// that the render tree is current, and otherwise walks the whole tree.
    /// [`end_frame`](Self::end_frame) clears it again.
    pub(crate) fn mark_animation_only_frame(
        &self,
        kind: FrameRequestKind,
        preparation: &FramePreparation,
        had_pending_resize: bool,
    ) {
        // Anything that arrived while the frame was being prepared — a scroll
        // step, a task that updated state — leaves a reason behind, and each
        // of them needs the whole tree.
        let eligible = kind == FrameRequestKind::Scoped
            && !had_pending_resize
            && self.pending_widget.is_none()
            // Requests made while the frame was prepared, by a task that
            // updated state, are scoped too; anything else needs the whole tree.
            && matches!(self.frame_request_reason.get(), None | Some(FrameRequestKind::Scoped))
            && !preparation.scroll_result.needs_redraw();
        aimer_widget::set_animation_only_frame(eligible);
    }

    /// Grants the focus the frame just built is owed.
    ///
    /// Focus is decided while an event is dispatched, which left a field that
    /// focuses itself on arrival — or a node asked to focus by a timer or a
    /// finished task — waiting for the user's next touch before it, and the
    /// focus animation its callback starts, heard of it. The dispatcher knows
    /// from counters alone whether anything is owed, so a frame with nothing to
    /// decide pays two comparisons. A grant is a change like any other and
    /// asks for the frame that shows it.
    fn settle_pending_focus(&mut self) {
        if !self.event_dispatcher.has_pending_focus_work() || self.active_root().is_none() {
            return;
        }
        let result = self.with_event_root(|dispatcher, root| dispatcher.settle_focus(root));
        // A request made while the grant was delivered stays pending, and so
        // does the frame that settles it.
        if result.needs_redraw() || self.event_dispatcher.has_pending_focus_work() {
            self.request_animation_frame();
        }
    }

    /// The bookkeeping every frame does once the tree has been drawn.
    ///
    /// Whatever the frame has left over is spent on background work — image
    /// decode, glyph rasterisation, prefetch — and not a microsecond more; a
    /// frame that overran leaves nothing behind and the next one spends
    /// nothing. Work still waiting when the slack runs out asks for the frame
    /// that continues it, which is what keeps a sliced task moving without a
    /// timer.
    pub(crate) fn end_frame(&mut self) {
        aimer_widget::set_scroll_only_frame(false);
        aimer_widget::set_animation_only_frame(false);
        self.settle_pending_focus();
        self.sync_accessibility();
        let budget = self.venus.idle_budget();
        self.venus.run_idle(&budget);
        self.venus.end_frame();
        // CALLED.print();
        // CALLED.reset();
        // The next platform-input interval is a new event frame. This shared
        // epoch also reaches dispatchers nested inside scrollables and regions.
        begin_event_frame();

        #[cfg(feature = "frame-stats")]
        if let Some((breakdown, content, timing)) = crate::frame_stats::take_debug_report() {
            let request_stats = crate::frame_stats::frame_request_stats();
            crate::frame_stats::reset_frame_request_stats();
            let target_size = self
                .window
                .as_ref()
                .map(|window| window.inner_size())
                .unwrap_or_default();
            let scale_factor = self
                .window
                .as_ref()
                .map_or(self.window_scale, |window| window.scale_factor());
            let refresh_millihertz = self
                .native_window()
                .and_then(|window| {
                    window
                        .current_monitor()
                        .or_else(|| window.primary_monitor())
                })
                .and_then(|monitor| monitor.refresh_rate_millihertz());
            aimer_utils::debug!(
                "[frame-stats] frames={} build={:.2}ms encode={:.2}ms present={:.2}ms cpu-frame-samples={} cpu-frame={:.2}ms cpu-frame-p95<={:.2}ms cpu-frame-p95-saturated={} gpu-frame-samples={} gpu-frame={:.2}ms gpu-frame-p95<={:.2}ms gpu-frame-p95-saturated={} ui={:.2}ms ui-p95<={:.2}ms queue={:.2}ms acquire={:.2}ms target={}x{}px scale={:.3} refresh-millihz={:?} nodes/frame={:.1} commands/frame={:.1} retained-layers/frame={:.1} text/frame={:.1} image-draws/frame={:.1} image-uploads/frame={:.1} text-cache-hit/frame={:.1} text-cache-miss/frame={:.1} rebuild-visits/frame={:.1} rebuild-pruned/frame={:.1} stateful-checks/frame={:.1} stateless-checks/frame={:.1} stateful-builds/frame={:.1} stateless-builds/frame={:.1} layout/frame={:.1} hit-test/frame={:.1} paint/frame={:.1} root-draw/frame={:.1} scroll-events/frame={:.1} scroll-steps/frame={:.1} smoothing-steps/frame={:.1} state-updates/frame={:.1} invalidations-queued/frame={:.1} invalidations-coalesced/frame={:.1} element-index-lookups/frame={:.1} stale-element-ids/frame={:.1} scroll-offset-updates/frame={:.1} redraw-requests/frame={:.1} frame-ready-accepted={} frame-ready-coalesced={} display-ticks={}",
                content.frames,
                breakdown.build.average().as_secs_f64() * 1_000.0,
                breakdown.encode.average().as_secs_f64() * 1_000.0,
                breakdown.present.average().as_secs_f64() * 1_000.0,
                timing.cpu_frame.samples,
                timing.cpu_frame.average().as_secs_f64() * 1_000.0,
                timing.cpu_frame_p95.as_secs_f64() * 1_000.0,
                timing.cpu_frame_p95_saturated,
                timing.gpu_frame.samples,
                timing.gpu_frame.average().as_secs_f64() * 1_000.0,
                timing.gpu_frame_p95.as_secs_f64() * 1_000.0,
                timing.gpu_frame_p95_saturated,
                timing.pipeline.ui.average().as_secs_f64() * 1_000.0,
                timing.pipeline.ui_p95.as_secs_f64() * 1_000.0,
                timing.pipeline.queue.average().as_secs_f64() * 1_000.0,
                timing.pipeline.acquire.average().as_secs_f64() * 1_000.0,
                target_size.width,
                target_size.height,
                scale_factor,
                refresh_millihertz,
                content.average_drawn_nodes(),
                content.average_draw_commands(),
                content.average_retained_layers(),
                content.average_text_commands(),
                content.average_image_draws(),
                content.average_image_uploads(),
                content.text_cache_hits as f64 / content.frames as f64,
                content.average_text_cache_misses(),
                content.rebuild_visits as f64 / content.frames as f64,
                content.rebuild_pruned as f64 / content.frames as f64,
                content.stateful_rebuild_checks as f64 / content.frames as f64,
                content.stateless_rebuild_checks as f64 / content.frames as f64,
                content.stateful_builds as f64 / content.frames as f64,
                content.stateless_builds as f64 / content.frames as f64,
                content.layout_calls as f64 / content.frames as f64,
                content.hit_test_visits as f64 / content.frames as f64,
                content.paint_calls as f64 / content.frames as f64,
                content.root_draw_calls as f64 / content.frames as f64,
                content.scroll_events as f64 / content.frames as f64,
                content.scroll_steps as f64 / content.frames as f64,
                content.smoothing_steps as f64 / content.frames as f64,
                content.state_updates as f64 / content.frames as f64,
                content.invalidations_queued as f64 / content.frames as f64,
                content.invalidations_coalesced as f64 / content.frames as f64,
                content.element_index_lookups as f64 / content.frames as f64,
                content.stale_element_ids as f64 / content.frames as f64,
                content.scroll_offset_updates as f64 / content.frames as f64,
                content.redraw_requests as f64 / content.frames as f64,
                request_stats.accepted,
                request_stats.coalesced,
                request_stats.display_ticks,
            );
        }

        if self.venus.has_ready_work() {
            self.request_animation_frame();
        }
    }

    pub(crate) fn dispatch_element_event(
        &mut self,
        pos: Vec2d,
        event: &aimer_events::element::ElementEvent,
    ) -> EventResult {
        self.with_event_root(|dispatcher, root| dispatcher.dispatch(root, pos, event))
    }

    pub(crate) fn broadcast_element_event(
        &mut self,
        event: &aimer_events::element::ElementEvent,
    ) -> EventResult {
        self.with_event_root(|dispatcher, root| dispatcher.broadcast(root, event))
    }

    fn with_event_root(
        &mut self,
        dispatch: impl FnOnce(&mut EventDispatcher, &dyn Element) -> EventResult,
    ) -> EventResult {
        let Self {
            event_dispatcher,
            widget_root,
            #[cfg(feature = "wasm-hot-reload")]
            live_reload,
            ..
        } = self;
        #[cfg(feature = "wasm-hot-reload")]
        let root = live_reload
            .as_ref()
            .and_then(crate::hot_reload::LiveReloadHost::active_root)
            .or(widget_root.as_ref());
        #[cfg(not(feature = "wasm-hot-reload"))]
        let root = widget_root.as_ref();
        let Some(root) = root else {
            return EventResult::ignored();
        };
        let result = dispatch(event_dispatcher, root.as_ref());
        // Captures only change inside a dispatch, so this is the one place that
        // keeps the paint-time hover check informed.
        aimer_widget::set_pointer_capture_active(event_dispatcher.has_capture());
        result
    }

    pub(crate) fn cancel_element_events(&mut self) -> EventResult {
        if self.active_root().is_none() {
            return EventResult::ignored();
        }
        let result = self.broadcast_element_event(&aimer_events::element::ElementEvent::Cancel);
        self.event_dispatcher.clear_captures();
        aimer_widget::set_pointer_capture_active(false);
        result
    }

    /// Delivers this frame's share of the pending scroll distance to the
    /// widget tree.
    ///
    /// Each channel carries the gesture phase resolved by the smoother, so a
    /// child sees `Started` when the gesture opens, `Moved` while it glides,
    /// and `Ended` or `Cancelled` when it finishes — instead of an endless
    /// stream of `Moved`.
    pub(crate) fn dispatch_smoothed_scroll(&mut self) -> EventResult {
        if self.scroll_smoother.is_active() {
            aimer_widget::record_smoothing_step();
        }
        let frame = self.scroll_smoother.tick();
        let mut result = EventResult::ignored();

        if let Some(step) = frame.trackpad {
            aimer_widget::record_scroll_step();
            result = result.merge(self.dispatch_element_event(
                self.cursor_pos,
                &aimer_events::element::ElementEvent::Scroll {
                    delta: Vec2d {
                        x: step.delta.x as f32,
                        y: step.delta.y as f32,
                    },
                    phase: step.phase,
                    kind: aimer_events::element::ScrollDeltaKind::Pixel,
                    is_direct_manipulation: step.is_direct_manipulation,
                },
            ));
        }
        if let Some(step) = frame.wheel {
            aimer_widget::record_scroll_step();
            result = result.merge(self.dispatch_element_event(
                self.cursor_pos,
                &aimer_events::element::ElementEvent::Scroll {
                    delta: Vec2d {
                        x: step.delta.x as f32,
                        y: step.delta.y as f32,
                    },
                    phase: step.phase,
                    kind: aimer_events::element::ScrollDeltaKind::Line,
                    is_direct_manipulation: step.is_direct_manipulation,
                },
            ));
        }

        result
    }
}

/// Builds and draws the widget tree of a single frame.
///
/// Owns nothing: it borrows the tree, the widget waiting to become one, and
/// the window they are drawn for, which is what allows the same code to paint
/// a platform surface and a headless canvas. The window it carries is the one
/// the tree sees in its [`BuildContext`], so a widget that asks for a repaint
/// or changes the cursor reaches the same handle the event handlers do.
pub(crate) struct FrameDrawer<'a, W: Widget + 'static> {
    #[cfg(debug_assertions)]
    debug_overlay: &'a mut crate::debug_overlay::DebugOverlay,
    widget_root: &'a mut Option<AnyElement>,
    pending_widget: &'a mut Option<W>,
    event_dispatcher: &'a mut EventDispatcher,
    window_render_tree: &'a mut WindowRenderTree,
    #[cfg(feature = "wasm-hot-reload")]
    live_reload: &'a mut Option<crate::hot_reload::LiveReloadHost>,
    window: WindowHandle,
    inherited_states:
        Option<Rc<std::cell::RefCell<aimer_widget::components::context::InheritedStates>>>,
    ui_allocator: UiAllocator,
    scale: f32,
    cursor_pos: Vec2d,
    #[cfg(not(target_arch = "wasm32"))]
    async_handle: tokio::runtime::Handle,
}

impl<'a, W: Widget + 'static> FrameDrawer<'a, W> {
    /// Draws one frame of `width` by `height` physical pixels into `canvas`.
    ///
    /// The root element is created on the first frame and reused afterwards,
    /// exactly as the windowed loop does — a headless application that renders
    /// twice does not rebuild its tree twice. The tree is drawn inside a saved
    /// canvas scope so a widget that leaves a transform behind cannot leak it
    /// into the next frame.
    pub(crate) fn draw(
        &mut self,
        canvas: &aimer_canvas::InnerCanvas,
        width: u32,
        height: u32,
    ) -> (f32, DamageSet) {
        let allocator = self.ui_allocator.clone();
        #[cfg(debug_assertions)]
        let debug_started = self
            .debug_overlay
            .is_visible()
            .then(aimer_utils::AnimInstant::now);
        let result = allocator.scope(|| self.draw_scoped(canvas, width, height));
        #[cfg(debug_assertions)]
        if let Some(started) = debug_started {
            self.debug_overlay.record_draw(started);
        }
        result
    }

    fn draw_scoped(
        &mut self,
        canvas: &aimer_canvas::InnerCanvas,
        width: u32,
        height: u32,
    ) -> (f32, DamageSet) {
        #[cfg(feature = "frame-stats")]
        let slow_frame = aimer_utils::AnimInstant::now();
        aimer_widget::begin_paint_frame(width, height);
        let rebuild_generation_before = aimer_widget::rebuild_invalidation_generation();
        // The rebuild generation less the advances that only kept an animating
        // element reachable, which what `rebuild_changed` below judges by, and
        // less the advances that state updates made as well, which is what the
        // scoped-frame baseline compares: those rebuild one subtree each, and the
        // log of replaced subtrees says which.
        let keepalive_net_rebuild_generation_before =
            rebuild_generation_before.wrapping_sub(aimer_widget::rebuild_keepalive_count());
        let net_rebuild_generation_before = keepalive_net_rebuild_generation_before
            .wrapping_sub(aimer_widget::rebuild_state_mark_count());
        let layout_generation_before = aimer_widget::layout_invalidation_generation();
        let texture_epoch_before = canvas.texture_cache_epoch();
        let inner_canvas = canvas;
        let frame_canvas = aimer_canvas::FrameCanvas::new(canvas);
        let mut build_ctx = BuildContext::new(
            frame_canvas,
            ResolvedSize {
                width: width as f32,
                height: height as f32,
            },
            self.scale,
            Default::default(),
            self.cursor_pos,
            self.window.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            self.async_handle.clone(),
        );
        build_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: width as f32,
            max_height: height as f32,
        };
        // A window opened to share its opener's providers starts every frame
        // with them. Scoped providers are added and removed on this map as the
        // traversal enters and leaves them, so it is the same map every frame.
        if let Some(states) = &self.inherited_states {
            build_ctx.inherited_states = states.clone();
        }

        #[cfg(feature = "wasm-hot-reload")]
        let live_layout_required = if let Some(host) = self.live_reload.as_mut() {
            if let Err(error) = host.process_safe_point(&build_ctx) {
                aimer_utils::error!("live reload safe point failed: {error}");
            }
            host.take_layout_required()
        } else {
            false
        };

        if self.widget_root.is_none()
            && let Some(widget) = self.pending_widget.take()
        {
            *self.widget_root = Some(widget.to_element(&build_ctx));
        }

        #[cfg(feature = "wasm-hot-reload")]
        let root = self
            .live_reload
            .as_ref()
            .and_then(crate::hot_reload::LiveReloadHost::active_root)
            .or(self.widget_root.as_ref());
        #[cfg(not(feature = "wasm-hot-reload"))]
        let root = self.widget_root.as_ref();

        let mut invalidations = if let Some(root) = root {
            self.event_dispatcher
                .take_pending_element_invalidations(root.as_ref())
        } else {
            ElementInvalidationBatch::default()
        };
        #[cfg(feature = "frame-stats")]
        aimer_widget::reset_draw_traversal_count();
        #[cfg(feature = "frame-stats")]
        aimer_widget::reset_rebuild_stats();

        let mut prepared_root = None;
        if let Some(root) = root {
            #[cfg(feature = "wasm-hot-reload")]
            if live_layout_required {
                aimer_widget::mark_paint_damage_full();
                root.rebuild_if_dirty(&build_ctx);
                root.layout(&build_ctx);
            } else {
                root.rebuild_if_dirty(&build_ctx);
            }
            #[cfg(not(feature = "wasm-hot-reload"))]
            root.rebuild_if_dirty(&build_ctx);
            prepared_root = Some(root.id());
        }

        #[cfg(feature = "frame-stats")]
        let slow_rebuilt = slow_frame.elapsed();
        if let Some(root) = root {
            aimer_modal::prepare_retained_render_tree(root.as_ref(), &build_ctx);
        }

        // A frame requested only to advance compositor animations revisits the
        // elements that animate instead of walking the tree. That is exact only
        // while nothing else changed since the last frame, so every input to the
        // last frame is compared: the rebuild generation and texture epoch, the
        // pointer, the render tree's own sync stamp, and the elements that
        // reported a change. Subtrees an animator replaced by ones of the same
        // shape do not count as a change: the synchronization below handles
        // them, and the check after it makes sure it did.
        #[cfg(not(feature = "wasm-hot-reload"))]
        let animation_only_eligible = match root {
            Some(root) => {
                aimer_widget::is_animation_only_frame()
                    && self.window_render_tree.animation_baseline
                        == Some(AnimationBaseline::new(
                            net_rebuild_generation_before,
                            texture_epoch_before,
                            self.cursor_pos,
                        ))
                    && self
                        .window_render_tree
                        .is_synchronized_apart_from_replacements(root, (width, height), self.scale)
            }
            None => false,
        };
        #[cfg(feature = "wasm-hot-reload")]
        let animation_only_eligible = false;
        let full_syncs_before = self.window_render_tree.sync_counts.full;

        let mut render_tree_sync_incomplete = self
            .window_render_tree
            .sync(root, &build_ctx, (width, height), self.scale)
            .is_err();
        // The subtrees rebuilt since the last walk or pass. A frame that rebuilt
        // some can still be scoped, but only if each can be revisited alone, and
        // only if every element that reported a change is one of their owners.
        let rebuilt = match self.window_render_tree.walk_cursor {
            Some(cursor) => aimer_widget::scoped_rebuilds_since(cursor),
            None => Some(Vec::new()),
        };
        self.window_render_tree.walk_cursor = Some(aimer_widget::scoped_rebuild_cursor());
        let rebuilt_known = rebuilt.is_some();
        let rebuilt_roots = rebuilt.unwrap_or_default();

        #[cfg(not(feature = "wasm-hot-reload"))]
        let animation_only_targets = match root {
            Some(root)
                if animation_only_eligible
                    && rebuilt_known
                    && !render_tree_sync_incomplete
                    && self.window_render_tree.sync_counts.full == full_syncs_before
                    && invalidations.records().iter().all(|record| {
                        record.element_id.is_some_and(|owner| {
                            rebuilt_roots.iter().any(|rebuilt| {
                                self.event_dispatcher.is_ancestor_or_self(
                                    root.as_ref(),
                                    owner,
                                    *rebuilt,
                                )
                            })
                        })
                    }) =>
            {
                aimer_widget::animation_only_targets(&rebuilt_roots, |id| {
                    self.event_dispatcher.resolve_element(root.as_ref(), id)
                })
            }
            _ => None,
        };
        #[cfg(feature = "wasm-hot-reload")]
        let animation_only_targets: Option<Vec<(ElementId, &dyn Element)>> = None;

        #[cfg(feature = "frame-stats")]
        let slow_synced = slow_frame.elapsed();
        // Stale entries from earlier work must not be mistaken for this frame's.
        let _ = aimer_widget::take_unmapped_draws();
        {
            let tree = self.window_render_tree.tree.clone();
            let element_nodes = self.window_render_tree.element_nodes.clone();
            let draw_widget_tree = || {
                if let Some(root) = root {
                    build_ctx.canvas.save();
                    aimer_widget::record_root_draw_call();
                    root.update(&build_ctx);
                    build_ctx.canvas.restore();
                }
            };
            aimer_widget::set_frame_rebuilt_roots(&rebuilt_roots);
            if let Some(targets) = &animation_only_targets {
                aimer_widget::with_v2_animation_only_context(tree, element_nodes, || {
                    aimer_widget::run_animation_only_pass(targets, &build_ctx.canvas);
                });
            } else {
                aimer_widget::with_v2_render_tree_context(
                    tree,
                    element_nodes,
                    prepared_root,
                    draw_widget_tree,
                );
            }
            aimer_widget::set_frame_rebuilt_roots(&[]);
        }
        #[cfg(feature = "frame-stats")]
        let slow_drawn = slow_frame.elapsed();

        match self
            .window_render_tree
            .sync(root, &build_ctx, (width, height), self.scale)
        {
            Ok(sync) if sync.refreshed => {
                if sync.added_nodes
                    && !record_new_local_v2_paints(
                        &self.window_render_tree.tree,
                        sync.added_render_nodes,
                    )
                {
                    self.window.request_redraw();
                    render_tree_sync_incomplete = true;
                }
            }
            Ok(_) => {}
            Err(_) => render_tree_sync_incomplete = true,
        }

        self.window_render_tree.settle_unmapped_draws();

        // The periodic report averages thirty frames, which hides one frame that
        // hitches. A frame past two display intervals at 60 Hz is named on its own.
        #[cfg(feature = "frame-stats")]
        {
            let total = slow_frame.elapsed();
            if total >= std::time::Duration::from_millis(25) {
                let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1_000.0;
                aimer_utils::warn!(
                    "[slow-frame] build={:.1}ms rebuild={:.1}ms sync={:.1}ms draw={:.1}ms resync={:.1}ms full-syncs={} scoped-syncs={}",
                    ms(total),
                    ms(slow_rebuilt),
                    ms(slow_synced - slow_rebuilt),
                    ms(slow_drawn - slow_synced),
                    ms(total - slow_drawn),
                    self.window_render_tree.sync_counts.full,
                    self.window_render_tree.sync_counts.scoped,
                );
            }
        }

        #[cfg(feature = "frame-stats")]
        if let Some(root) = root {
            self.window_render_tree.report_census_change(root.as_ref());
        }

        if let Some(root) = root {
            self.event_dispatcher
                .complete_pending_element_invalidations(root.as_ref(), &mut invalidations);
        }
        let retained_v2_damage_covers_stateful_invalidations = !render_tree_sync_incomplete
            && self.scale.is_finite()
            && self.scale > 0.0
            && self
                .window_render_tree
                .covers_unbounded_stateful_invalidations(&invalidations);
        if invalidations.requires_full_fallback()
            && !retained_v2_damage_covers_stateful_invalidations
        {
            aimer_widget::mark_paint_damage_full();
        }

        if crate::render_ctx::take_frame_not_presented() {
            // The frame before this one was built and lost, taking its damage
            // with it; the screen is out of date by an unknown amount.
            aimer_widget::mark_paint_damage_full();
        }
        let mut damage = aimer_widget::take_paint_frame_damage(width, height);
        if render_tree_sync_incomplete {
            // The tree could not be brought up to date, so the plan below is
            // composed from its last consistent state. Repaint everything from
            // that rather than trusting what this frame says changed.
            aimer_utils::error!(
                "the render tree could not be synchronized with the widget tree; \
                 presenting its last consistent state"
            );
            damage.mark_full();
        }
        // An animating element advances the rebuild generation every frame only
        // to stay reachable. What it changed is in the retained tree's damage, so
        // that advance must not turn the frame into a full repaint.
        let rebuild_changed = keepalive_net_rebuild_generation_before
            != aimer_widget::rebuild_invalidation_generation()
                .wrapping_sub(aimer_widget::rebuild_keepalive_count());
        let layout_or_texture_changed = layout_generation_before
            != aimer_widget::layout_invalidation_generation()
            || texture_epoch_before != build_ctx.canvas.texture_cache_epoch();
        // A rebuild that left damage in the retained tree has said what it
        // changed: each element that recorded something different, and each node
        // added, moved or removed, damaged its own bounds. Only a rebuild that
        // left no footprint anywhere is unaccounted for.
        let rebuild_unaccounted = rebuild_changed
            && damage.is_empty()
            && !self.window_render_tree.tree.has_pending_damage();
        if layout_or_texture_changed || rebuild_unaccounted {
            // Layout and texture changes can affect paint outside a retained
            // owner's footprint. Keep those transitions on the correctness-
            // first full-target path until every producer has a precise
            // footprint contract.
            damage.mark_full();
        }

        if self.scale.is_finite() && self.scale > 0.0 {
            #[cfg(debug_assertions)]
            if let Err(error) = self.debug_overlay.prepare(
                &self.window_render_tree.tree,
                width,
                height,
                self.scale,
                &self.ui_allocator,
                self.cursor_pos,
            ) {
                aimer_utils::error!("debug overlay recording failed: {error:?}");
            }
            let mut render_damage = self.window_render_tree.tree.take_damage();
            if damage.is_full() {
                render_damage.clear();
                render_damage.push(RenderRect::new(
                    0.0,
                    0.0,
                    width as f32 / self.scale,
                    height as f32 / self.scale,
                ));
            } else {
                render_damage.extend(damage.regions().iter().map(|region| {
                    RenderRect::new(
                        region.x as f32 / self.scale,
                        region.y as f32 / self.scale,
                        region.width as f32 / self.scale,
                        region.height as f32 / self.scale,
                    )
                }));
            }
            // A damage set that covers enough of the target is repainted in
            // full, which needs the whole tree rather than the culled slice.
            if let Some(full) = FramePacket::full_target_if_damage_promotes(
                &render_damage,
                self.scale,
                width,
                height,
            ) {
                render_damage.clear();
                render_damage.push(full);
            }
            // The retained target keeps pixels outside damage, so even a
            // direct plan only needs the nodes that can affect this frame's
            // dirty regions. Culling before adaptation avoids snapshotting and
            // lowering every clean local list on every frame.
            let mut render_operations = self.window_render_tree.tree.render_order(&render_damage);
            // The renderer draws a frame that holds a custom-pipeline draw from
            // scratch, from the operations in front of it, and leaves its
            // persistent target stale. A plan culled to a small damage holds
            // only the nodes that changed, so drawing it from scratch leaves the
            // rest of the window blank, and the frame after it finds the target
            // showing an older page. Such a frame, and the one that follows it,
            // are built from the whole visible tree.
            let full_target = RenderRect::new(
                0.0,
                0.0,
                width as f32 / self.scale,
                height as f32 / self.scale,
            );
            let covers_target = render_damage.as_slice() == [full_target];
            if !covers_target
                && (self.window_render_tree.drew_custom_pipeline
                    || operations_use_custom_pipeline(&render_operations))
            {
                render_damage.clear();
                render_damage.push(full_target);
                render_operations = self.window_render_tree.tree.render_order(&render_damage);
            }
            self.window_render_tree.drew_custom_pipeline =
                operations_use_custom_pipeline(&render_operations);
            let render_frame = RenderFrame {
                operations: render_operations,
                damage: render_damage,
            };
            let adapter_metadata =
                FrameRenderMetadata::new(self.scale, 0, 0, 0, 0, DamageSet::new(width, height));
            let mut legacy_frame = Frame::new(inner_canvas.take_draw_list(), width, height);
            match FramePacket::prepare_v2_direct_render_plan(
                &render_frame,
                &adapter_metadata,
                &mut legacy_frame,
            ) {
                Ok((plan, packet_damage)) => {
                    damage = packet_damage;
                    inner_canvas.set_retained_render_plan(Some(plan));
                    inner_canvas.recycle_draw_list(legacy_frame.into_draw_list());
                }
                Err(error) => {
                    // The tree describes something the renderer cannot lower.
                    // Present nothing new instead of a blank frame: the target
                    // keeps its last pixels, and no redraw is requested so a
                    // persistent error cannot spin the frame loop.
                    aimer_utils::error!("the render plan could not be lowered: {error:?}");
                    inner_canvas.recycle_draw_list(legacy_frame.into_draw_list());
                    damage = DamageSet::new(width, height);
                }
            }
        } else {
            // Without a usable scale nothing can be placed on the target. Keep
            // the whole frame damaged so the first valid scale repaints it.
            inner_canvas.begin_frame();
            damage.mark_full();
            aimer_widget::mark_paint_damage_full();
        }

        #[cfg(feature = "frame-stats")]
        {
            let draw_list = inner_canvas.draw_list();
            let draw_list_stats = draw_list.stats();
            let (text_cache_hits, text_cache_misses) = inner_canvas.text_cache_stats();
            let rebuild_stats = aimer_widget::take_rebuild_stats();
            let work_stats = aimer_widget::take_frame_work_stats();
            crate::frame_stats::record_frame_content(
                aimer_widget::take_draw_traversal_count(),
                draw_list_stats,
                text_cache_hits,
                text_cache_misses,
                rebuild_stats,
                work_stats,
            );
        }
        self.window_render_tree.animation_baseline = Some(AnimationBaseline::new(
            aimer_widget::rebuild_invalidation_generation()
                .wrapping_sub(aimer_widget::rebuild_keepalive_count())
                .wrapping_sub(aimer_widget::rebuild_state_mark_count()),
            build_ctx.canvas.texture_cache_epoch(),
            self.cursor_pos,
        ));
        (self.scale, damage)
    }
}

/// Whether any of `operations` records a custom-pipeline draw, which the
/// renderer cannot composite through its persistent target.
fn operations_use_custom_pipeline(operations: &[RenderOp]) -> bool {
    operations.iter().any(|operation| match operation {
        RenderOp::Draw(item) => item
            .snapshot()
            .commands
            .iter()
            .any(|command| matches!(command, V2DrawCommand::DrawCustom { .. })),
        RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => false,
    })
}

/// Runs every hook the application registered, once, in registration order,
/// and keeps whatever they return alive for the rest of the run.
pub(crate) fn run_startup_hooks(
    startup_hooks: &mut Vec<StartupHook>,
    startup_resources: &mut Vec<Box<dyn Any>>,
) {
    startup_resources.extend(startup_hooks.drain(..).map(|hook| hook()));
}

impl<W: Widget + 'static> AimerApplicationHandler<W> {
    /// Creates this application's platform window, unless it already has one.
    ///
    /// The window is made before everything else a resume does so that a host
    /// with several windows can register the new one with the thread (see
    /// [`aimer_events::window::register_window_scope`]) before any of the
    /// work that asks it for frames.
    pub(crate) fn create_native_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.macos_windowing = aimer_native::macos_windowing::take_pending();
        }

        #[cfg(target_os = "android")]
        {
            use winit::event_loop::ControlFlow;
            event_loop.set_control_flow(ControlFlow::Poll);
            debug!("Set ControlFlow::Poll for Android");
        }

        let window_attributes = {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                self.window_attr.to_winit()
            }
            #[cfg(target_os = "android")]
            {
                self.window_attr.to_winit()
            }
            #[cfg(target_os = "ios")]
            {
                // UIKit owns the scene's resizable bounds, not UIScreen.
                WindowAttributes::default()
            }
        };
        let window_attributes = self.macos_windowing.apply_attributes(window_attributes);
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        let window_attributes = if self.show_window_after_first_frame {
            window_attributes.with_visible(false)
        } else {
            window_attributes
        };
        // The DXGI swap chain is the window's only content. With a GDI
        // redirection surface underneath it, DWM shows that never-painted
        // surface as a white strip whenever it composites the window at a size
        // the presented buffer does not cover, which a drag that shrinks the
        // window does on most frames.
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        let window_attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            window_attributes.with_no_redirection_bitmap(true)
        };

        if self.window.is_none() {
            // Owned by whoever still holds it: the window is destroyed when the
            // last handle goes, which is what lets a closed window be freed.
            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());
            aimer_events::window::set_window(&window);
            self.window = Some(WindowHandle::native(window));
        }
    }

    /// Brings this application's window up: renderer, metrics, appearance and
    /// the first frame. Creates the window first if it does not exist yet.
    pub(crate) fn resume_window(&mut self, event_loop: &ActiveEventLoop) {
        self.create_native_window(event_loop);

        // Cloned: resuming changes the application while it uses the window.
        let window = self
            .native_window()
            .cloned()
            .expect("the windowed loop always owns a native window");
        let window = &window;
        self.macos_windowing.window_created(window);
        aimer_platform::accessibility::attach(window);

        // The runtime was built before this window existed, so it has been
        // budgeting frames against an assumed 60 Hz. Told the real rate here —
        // the first moment there is a monitor to ask — a ProMotion or gaming
        // display stops being handed twice the idle time its frames have, and an
        // overrun is recognised after one missed frame instead of two.
        self.tune_runtime_to_display(window);

        // winit's iOS window is created without a `UIWindowScene`. On the
        // iOS 26/27 SDK the scene life cycle is mandatory, so a scene-less
        // window stays invisible (black screen) and never redraws. Attach it to
        // the active window scene so it becomes visible and starts redrawing.
        #[cfg(target_os = "ios")]
        crate::ios_screen::attach_window_to_active_scene(window);

        // The appearance the system is in right now: a theme that follows the
        // system has to start in the right one, not switch into it on the first
        // change. Platforms that do not report an appearance leave the light
        // default in place. Asked after the window is on screen, because UIKit
        // resolves the appearance against a window and has no answer before
        // that.
        crate::system_appearance::announce(window);

        // Where the platform does not deliver appearance changes as a window
        // event — iOS reports them as UIKit trait changes — they are subscribed
        // to here instead.
        crate::system_appearance::start_observing(window);

        // The region the system reserves in the window — the status bar, the
        // notch, the home indicator. Read after the window is on screen for the
        // same reason the appearance is: UIKit resolves it against a laid-out
        // view. A rotation reports it again, as a resize.
        crate::system_safe_area::announce(window);

        // Which way the device is turned, for the same reason: it is asked once
        // the window is up, and again whenever the window changes shape. iOS
        // reports the hinge from a callback of its own, which wakes the loop
        // through the waker registered first.
        #[cfg(target_os = "ios")]
        aimer_screen::set_change_waker(crate::aimer_app::wake_for_screen_change);
        aimer_screen::refresh();

        // A browser changes the reservation without winit hearing about it, so
        // its own resize notifications are subscribed to here.
        crate::system_safe_area::start_observing(window);

        #[allow(unused_mut)]
        let mut size = WindowHandle::native(window.clone()).inner_size();

        #[cfg(target_os = "android")]
        {
            if let Some(android_app) = crate::aimer_app::ANDROID_APP.get() {
                if let Some(native_window) = android_app.native_window() {
                    let width = native_window.width() as u32;
                    let height = native_window.height() as u32;
                    size = winit::dpi::PhysicalSize::new(width, height);
                }
            }
        }

        // debug!("Logical Window Size : {:?}", window.outer_size());
        // debug!("Physical Window Size : {size:?}");

        self.render_ctx.initialize(window, size);

        self.window_scale = window.scale_factor();

        // On Android the surface may be (re-)created with the correct size now.
        // Schedule a resize so the GPU surface matches the actual window dimensions.
        self.pending_resize = Some(size);
        self.native_window_size = Some(ResolvedSize {
            width: size.width as f32,
            height: size.height as f32,
        });
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        if self.show_window_after_first_frame {
            // Start rendering while hidden; worker completion reveals the
            // window only after its first application frame is presented.
            self.render(event_loop);
        } else {
            self.request_full_redraw();
        }
        #[cfg(not(all(target_os = "windows", feature = "native", not(feature = "wgpu"))))]
        self.request_full_redraw();
    }
}

impl<W: Widget + 'static> ApplicationHandler<AimerNativePlatformEvent>
    for AimerApplicationHandler<W>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Before the first frame builds the widget tree: the link that launched
        // the app has to be waiting when the navigator is created.
        crate::link_inbox::deliver_posted_links();
        run_startup_hooks(&mut self.startup_hooks, &mut self.startup_resources);
        self.resume_window(event_loop);
    }


    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AimerNativePlatformEvent) {
        if let AimerNativePlatformEvent::Back(signal) = event {
            // A back nothing answered is the platform's to resolve, and on
            // Android that means leaving the app: winit never does it for us.
            if system_back::handle_signal(signal) == system_back::BackOutcome::Unhandled
                && system_back::UNHANDLED_BACK_EXITS
            {
                event_handler::WindowEventHandler::close_window(self, _event_loop);
            }
            return;
        }
        if matches!(event, AimerNativePlatformEvent::ShutdownRequested) {
            // `exiting` follows, which tells the app's services to finish.
            _event_loop.exit();
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if matches!(event, AimerNativePlatformEvent::RenderWorkerReady) {
            // A hidden window never receives WM_PAINT, so `request_redraw`
            // cannot produce the first frame that reveals it. Render directly;
            // `render` delivers the pending completions itself.
            #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
            if self.show_window_after_first_frame {
                self.render(_event_loop);
                return;
            }
            self.deliver_render_completions(_event_loop);
            return;
        }
        // debug!("User event {:?}", event);
        #[cfg(target_os = "ios")]
        handle_user_event(self, event, |app, _kind| app.render(_event_loop));
        #[cfg(not(target_os = "ios"))]
        handle_user_event(
            self,
            event,
            crate::handler::user_events::request_window_redraw,
        );
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // First, while the app is still fully alive: on a mobile OS this is the
        // last warning before the process can be killed, and state that has not
        // reached storage by the time this returns may be lost.
        aimer_venus::notify_lifecycle(aimer_venus::Lifecycle::Background);
        // winit reports iOS's `applicationWillResignActive:` as `suspended`, and
        // UIKit sends that for Control Center, Notification Center, a call or the
        // app switcher gesture while the window is still on screen. Releasing the
        // surface here removes its layer, which blacks the window out (and the
        // app switcher snapshot with it). Real backgrounding arrives as
        // `Occluded(true)`, which pauses drawing without releasing anything.
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "ios")))]
        self.render_ctx.suspend();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Every way out of the loop ends here: a window close, a quit command,
        // and the platform's own terminate notification.
        aimer_venus::notify_lifecycle(aimer_venus::Lifecycle::Exit);
        #[cfg(not(target_arch = "wasm32"))]
        self.render_ctx.suspend();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        WindowEventHandler::handle_events(self, event_loop, _id, event);
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "macos")]
        if let Some(window) = self.window.as_ref().and_then(WindowHandle::native_window) {
            self.macos_windowing.window_redraw_requested(window);
        }

        // A file drag is the one gesture the platform stops describing once it
        // has begun: macOS delivers no cursor motion at all while its own drag
        // session runs. So the position is asked for here instead, and a frame
        // is asked for in turn, which brings us back here for as long as the
        // drag lasts and not one wake-up longer.
        #[cfg(target_os = "macos")]
        if self.file_drag.is_active() {
            WindowEventHandler::poll_file_drag(self);
            self.request_full_redraw();
        }
    }
}
#[allow(dead_code)]
impl<W: Widget + 'static> AimerApplicationHandler<W> {
    /// Splits the handler into the renderer that owns the frame and a drawer
    /// for the widget tree that goes into it.
    ///
    /// The two have to be borrowed apart because the tree is drawn from inside
    /// a closure the renderer runs: the closure cannot hold the handler while
    /// the renderer is being borrowed out of it. A headless application uses
    /// the drawer alone and paints into its own canvas, so the tree is built,
    /// laid out, and drawn by identical code whichever way the frame was asked
    /// for.

    /// Where the pointer is for the purpose of hover: nowhere, while a finger
    /// is what last touched the window.
    #[inline]
    pub(crate) fn hover_pos(&self) -> Vec2d {
        if self.touch_pointer {
            crate::handler::event_handler::CURSOR_OUTSIDE_POSITION
        } else {
            self.cursor_pos
        }
    }

    pub(crate) fn split_for_frame(
        &mut self,
        window: WindowHandle,
    ) -> (&mut AimerRenderContext, FrameDrawer<'_, W>) {
        let scale = self.window_scale as f32;
        let cursor_pos = self.hover_pos();
        let ui_allocator = self.ui_memory.allocator();
        let inherited_states = self.inherited_states.clone();
        let Self {
            render_ctx,
            widget_root,
            pending_widget,
            event_dispatcher,
            render_tree,
            #[cfg(debug_assertions)]
            debug_overlay,
            #[cfg(feature = "wasm-hot-reload")]
            live_reload,
            #[cfg(not(target_arch = "wasm32"))]
            async_handle,
            ..
        } = self;
        #[cfg(not(target_arch = "wasm32"))]
        let async_handle = async_handle.clone();

        (
            render_ctx,
            FrameDrawer {
                #[cfg(debug_assertions)]
                debug_overlay,
                widget_root,
                pending_widget,
                event_dispatcher,
                window_render_tree: render_tree,
                #[cfg(feature = "wasm-hot-reload")]
                live_reload,
                window,
                inherited_states,
                ui_allocator,
                scale,
                cursor_pos,
                #[cfg(not(target_arch = "wasm32"))]
                async_handle,
            },
        )
    }

    #[allow(unused)]
    pub(crate) fn render(&mut self, event_loop: &ActiveEventLoop) {
        // UIKit can lay out the view before winit delivers its queued Resized
        // event. Refresh before rebuilding or measuring the tree for this frame.
        #[cfg(target_os = "ios")]
        WindowEventHandler::refresh_uikit_viewport(self);
        #[cfg(target_arch = "wasm32")]
        self.note_frame_request(crate::aimer_app::frame_ready_delivered());
        let had_pending_resize = self.pending_resize.is_some();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.deliver_render_completions(event_loop);
            // Admission precedes animation/layout and damage consumption.
            if !self.admit_render_frame() {
                return;
            }
        }
        let kind = self.take_frame_request_reason();
        let preparation = self.begin_frame();

        #[cfg(target_os = "android")]
        {
            if let Some(android_app) = crate::aimer_app::ANDROID_APP.get() {
                let Some(native_window) = android_app.native_window() else {
                    debug!("Android native window is not ready yet");
                    return;
                };
            }
        }

        #[allow(clippy::collapsible_if)]
        if self.render_ctx.is_ready() {
            if let Some(size) = self.pending_resize.take() {
                self.render_ctx.resize(size);
            }
        }

        if self.should_skip_scroll_frame(kind, &preparation, had_pending_resize) {
            self.end_frame();
            return;
        }
        self.mark_scroll_only_frame(kind, &preparation, had_pending_resize);
        self.mark_animation_only_frame(kind, &preparation, had_pending_resize);

        let Some(window) = self.window.clone() else {
            return;
        };
        let (render_ctx, mut drawer) = self.split_for_frame(window);

        #[cfg(target_arch = "wasm32")]
        let outcome = render_ctx
            .render_frame_packet(move |canvas, width, height| drawer.draw(canvas, width, height));
        #[cfg(not(target_arch = "wasm32"))]
        let outcome = render_ctx
            .render_frame_packet(move |canvas, width, height| drawer.draw(canvas, width, height));
        #[cfg(all(target_os = "windows", feature = "native", not(feature = "wgpu")))]
        if outcome.is_presented() && self.show_window_after_first_frame {
            if let Some(window) = self.native_window() {
                window.set_visible(true);
                self.show_window_after_first_frame = false;
            }
        }
        // Worker completions dispatch this same notification on the UI thread.
        crate::first_frame::notify_first_frame_presented(outcome.is_presented());
        if outcome.needs_retry() {
            crate::render_ctx::note_frame_not_presented();
            // Surface texture was not available (e.g. surface outdated or
            // window not ready).  Request a redraw so we retry next frame
            // instead of staying blank.  Critical on web (async GPU init)
            // and iOS (late surface availability).
            self.note_frame_request(FrameRequestKind::Full);
            // UIKit coalesces setNeedsDisplay during drawRect. A dropped
            // resize frame needs a later display tick, after this draw returns.
            #[cfg(target_os = "ios")]
            aimer_events::window::request_animation_frame();
            #[cfg(not(target_os = "ios"))]
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            begin_event_frame();
            return;
        }

        self.end_frame();
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::census::CensusElement;
    use super::{WindowRenderTree, record_new_local_v2_paints, run_startup_hooks};
    use aimer_attribute::Dimension;
    use aimer_attribute::position::Vec2d;
    use aimer_attribute::size::{ResolvedSize, Size};
    use aimer_container::Container;
    use aimer_cupid::draw_cmd_v2::{Rect, RenderOp, RenderPaintSource};
    use aimer_flex::Column;
    use aimer_scroll::{ScrollController, Scrollable};
    use aimer_widget::base::{BuildContext, Color, WindowHandle};
    use aimer_widget::{
        AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
        Widget,
    };
    use std::cell::Cell;

    struct SizedLeaf {
        width: Rc<Cell<f32>>,
    }

    impl VisitorElement for SizedLeaf {
        fn debug_name(&self) -> &'static str {
            "RenderTreeSyncLeaf"
        }
    }

    impl EventElement for SizedLeaf {}
    impl Rebuildable for SizedLeaf {}

    impl LayoutElement for SizedLeaf {
        fn pos(&self) -> Option<Vec2d> {
            Some(Vec2d { x: 4.0, y: 6.0 })
        }

        fn size(&self) -> Option<Size> {
            Some(Size::new(
                Dimension::Px(self.width.get()),
                Dimension::Px(12.0),
            ))
        }
    }

    impl Drawable for SizedLeaf {
        fn update(&self, _ctx: &BuildContext) {}
    }

    struct V2Parent {
        child: AnyElement,
        paints: Rc<Cell<u32>>,
    }

    impl VisitorElement for V2Parent {
        fn debug_name(&self) -> &'static str {
            "V2Parent"
        }

        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl EventElement for V2Parent {}
    impl Rebuildable for V2Parent {}

    impl LayoutElement for V2Parent {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(30.0), Dimension::Px(30.0)))
        }
    }

    impl Drawable for V2Parent {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            self.paints.set(self.paints.get() + 1);
            let canvas = aimer_canvas::Canvas::of(ctx);
            canvas.fill_rect(Rect::new(0.0, 0.0, 30.0, 30.0), [30, 90, 180, 255]);
            canvas.finish();
        }
    }

    /// Draws a child without reporting it in `visit_children`, the mistake that
    /// left `GestureDetector`'s and `Animated`'s content without a render node.
    struct HiddenChildParent {
        child: AnyElement,
    }

    impl VisitorElement for HiddenChildParent {
        fn debug_name(&self) -> &'static str {
            "HiddenChildParent"
        }
    }

    impl EventElement for HiddenChildParent {}
    impl Rebuildable for HiddenChildParent {}

    impl LayoutElement for HiddenChildParent {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(30.0), Dimension::Px(30.0)))
        }
    }

    impl Drawable for HiddenChildParent {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            aimer_canvas::Canvas::of(ctx).finish();
        }
    }

    /// An element whose measured size is not a number, which the render tree
    /// rejects as invalid bounds.
    struct NonFiniteLeaf;

    impl VisitorElement for NonFiniteLeaf {
        fn debug_name(&self) -> &'static str {
            "NonFiniteLeaf"
        }
    }

    impl EventElement for NonFiniteLeaf {}
    impl Rebuildable for NonFiniteLeaf {}

    impl LayoutElement for NonFiniteLeaf {
        fn content_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: f32::NAN,
                height: 10.0,
            }
        }
    }

    impl Drawable for NonFiniteLeaf {
        fn update(&self, _ctx: &BuildContext) {}
    }

    struct LegacyBranch {
        child: AnyElement,
    }

    impl VisitorElement for LegacyBranch {
        fn debug_name(&self) -> &'static str {
            "LegacyBranch"
        }

        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl EventElement for LegacyBranch {}
    impl Rebuildable for LegacyBranch {}

    impl LayoutElement for LegacyBranch {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(20.0), Dimension::Px(20.0)))
        }
    }

    impl Drawable for LegacyBranch {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }
    }

    struct V2Grandchild {
        paints: Rc<Cell<u32>>,
    }

    impl VisitorElement for V2Grandchild {
        fn debug_name(&self) -> &'static str {
            "V2Grandchild"
        }
    }

    impl EventElement for V2Grandchild {}
    impl Rebuildable for V2Grandchild {}

    impl LayoutElement for V2Grandchild {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(10.0), Dimension::Px(10.0)))
        }
    }

    impl Drawable for V2Grandchild {
        fn update(&self, _ctx: &BuildContext) {}

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            self.paints.set(self.paints.get() + 1);
            let canvas = aimer_canvas::Canvas::of(ctx);
            canvas.finish();
        }
    }

    struct V2PaintLeaf {
        position: Vec2d,
        width: f32,
        height: f32,
        color: Color,
        paints: Rc<Cell<u32>>,
    }

    impl VisitorElement for V2PaintLeaf {
        fn debug_name(&self) -> &'static str {
            "V2PaintLeaf"
        }
    }

    impl EventElement for V2PaintLeaf {}
    impl Rebuildable for V2PaintLeaf {}

    impl LayoutElement for V2PaintLeaf {
        fn pos(&self) -> Option<Vec2d> {
            Some(self.position)
        }

        fn size(&self) -> Option<Size> {
            Some(Size::new(
                Dimension::Px(self.width),
                Dimension::Px(self.height),
            ))
        }
    }

    impl Drawable for V2PaintLeaf {
        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            self.paints.set(self.paints.get() + 1);
            let (red, green, blue, alpha) = self.color.to_rgba();
            let canvas = aimer_canvas::Canvas::of(ctx);
            canvas.fill_rect(
                Rect::new(0.0, 0.0, self.width, self.height),
                [red, green, blue, alpha],
            );
            canvas.finish();
        }
    }

    impl Widget for V2PaintLeaf {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            aimer_widget::Element::boxed(self)
        }
    }

    impl aimer_widget::PortableWidget for V2PaintLeaf {}

    struct ScrollContentClip<W> {
        child: W,
    }

    struct ScrollContentClipElement {
        child: AnyElement,
    }

    impl<W: Widget + 'static> Widget for ScrollContentClip<W> {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            ScrollContentClipElement {
                child: self.child.to_element(ctx),
            }
            .boxed()
        }
    }

    impl<W: Widget + 'static> aimer_widget::PortableWidget for ScrollContentClip<W> {}

    impl VisitorElement for ScrollContentClipElement {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn debug_name(&self) -> &'static str {
            "ScrollContentClipTest"
        }
    }

    impl EventElement for ScrollContentClipElement {
        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl Rebuildable for ScrollContentClipElement {}

    impl LayoutElement for ScrollContentClipElement {
        fn pos(&self) -> Option<Vec2d> {
            Some(Vec2d::ZERO)
        }

        fn size(&self) -> Option<Size> {
            self.child.size()
        }

        fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.layout(ctx)
        }

        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.computed_size(ctx)
        }

        fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.content_size(ctx)
        }
    }

    impl Drawable for ScrollContentClipElement {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            aimer_canvas::Canvas::of(ctx).finish();
        }

        fn prepare_layout(&self, ctx: &BuildContext) -> bool {
            self.child.prepare_layout(ctx)
        }

        fn retained_clip(&self, _ctx: &BuildContext) -> Option<Rect> {
            Some(Rect::new(10.0, 400.0, 80.0, 100.0))
        }
    }

    struct V2PaintTreeRoot {
        children: [AnyElement; 2],
    }

    impl VisitorElement for V2PaintTreeRoot {
        fn debug_name(&self) -> &'static str {
            "V2PaintTreeRoot"
        }

        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.children {
                visitor(child.as_ref());
            }
        }
    }

    impl EventElement for V2PaintTreeRoot {}
    impl Rebuildable for V2PaintTreeRoot {}

    impl LayoutElement for V2PaintTreeRoot {
        fn size(&self) -> Option<Size> {
            Some(Size::new(Dimension::Px(100.0), Dimension::Px(80.0)))
        }
    }

    impl Drawable for V2PaintTreeRoot {
        fn update(&self, ctx: &BuildContext) {
            for child in &self.children {
                child.update(ctx);
            }
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            let canvas = aimer_canvas::Canvas::of(ctx);
            canvas.finish();
        }
    }

    /// A wrapper that adds nothing: its sizes are its child's, and it draws the
    /// child with the context it was given (as a theme, provider or stateful
    /// wrapper does).
    struct PassThrough {
        child: AnyElement,
    }

    impl VisitorElement for PassThrough {
        fn debug_name(&self) -> &'static str {
            "PassThrough"
        }

        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }
    }

    impl EventElement for PassThrough {}
    impl Rebuildable for PassThrough {}

    impl LayoutElement for PassThrough {
        fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.computed_size(ctx)
        }

        fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
            self.child.content_size(ctx)
        }
    }

    impl Drawable for PassThrough {
        fn update(&self, ctx: &BuildContext) {
            self.child.update(ctx);
        }

        fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
            true
        }

        fn paint_local_v2(&self, ctx: &BuildContext) {
            aimer_canvas::Canvas::of(ctx).finish();
        }
    }

    fn build_context<'a>(
        canvas: &'a aimer_canvas::InnerCanvas,
        scale: f32,
    ) -> (tokio::runtime::Runtime, BuildContext<'a>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create render-tree sync runtime");
        let context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(canvas),
            ResolvedSize {
                width: 100.0,
                height: 80.0,
            },
            scale,
            Vec2d::ZERO,
            Vec2d::ZERO,
            WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 80), f64::from(scale)),
            runtime.handle().clone(),
        );
        (runtime, context)
    }

    fn find_element_id(root: &dyn Element, debug_name: &str) -> Option<aimer_widget::ElementId> {
        let mut pending = vec![root];
        while let Some(element) = pending.pop() {
            if element.debug_name() == debug_name {
                return Some(element.id());
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        None
    }

    fn find_element_ids(root: &dyn Element, debug_name: &str) -> Vec<aimer_widget::ElementId> {
        let mut found = Vec::new();
        let mut pending = vec![root];
        while let Some(element) = pending.pop() {
            if element.debug_name() == debug_name {
                found.push(element.id());
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        found
    }

    fn draw_v2_until_tree_is_current(
        root: &AnyElement,
        context: &BuildContext<'_>,
        window_tree: &mut WindowRenderTree,
    ) -> usize {
        let root_id = root.id();
        let mut drawn_frames = 0;
        for _ in 0..8 {
            drawn_frames += 1;
            root.rebuild_if_dirty(context);
            let _ = aimer_widget::take_unmapped_draws();
            window_tree
                .sync(Some(root), context, (100, 80), context.scale)
                .expect("synchronize the scroll render tree");
            context.canvas.begin_frame();
            aimer_widget::begin_paint_frame(100, 80);
            let tree = window_tree.tree.clone();
            let element_nodes = window_tree.element_nodes.clone();
            aimer_widget::with_v2_render_tree_context(tree, element_nodes, Some(root_id), || {
                context.canvas.save();
                root.update(context);
                context.canvas.restore();
            });
            let sync = window_tree
                .sync(Some(root), context, (100, 80), context.scale)
                .expect("refresh the scroll render tree after drawing");
            if !sync.refreshed
                || !sync.added_nodes
                || record_new_local_v2_paints(&window_tree.tree, sync.added_render_nodes)
            {
                window_tree.settle_unmapped_draws();
                return drawn_frames;
            }
        }
        panic!("the scroll render tree did not settle after virtualized updates");
    }

    #[test]
    fn window_render_tree_keeps_element_ids_across_layout_updates() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 2.0);
        let width = Rc::new(Cell::new(24.0));
        let root = SizedLeaf {
            width: width.clone(),
        }
        .boxed();
        let element_id = root.id();
        let mut window_tree = WindowRenderTree::default();

        let initial_sync = window_tree
            .sync(Some(&root), &context, (100, 80), 2.0)
            .unwrap();
        assert!(initial_sync.refreshed);
        assert!(initial_sync.added_nodes);
        let render_id = window_tree.node_for_element(element_id).unwrap();
        assert_eq!(
            window_tree.tree().element_bounds(render_id).unwrap(),
            Rect::new(2.0, 3.0, 24.0, 12.0)
        );
        let unchanged = window_tree
            .sync(Some(&root), &context, (100, 80), 2.0)
            .unwrap();
        assert!(!unchanged.refreshed);
        assert!(!unchanged.added_nodes);

        width.set(36.0);
        root.invalidate_layout();
        let layout_sync = window_tree
            .sync(Some(&root), &context, (100, 80), 2.0)
            .unwrap();
        assert!(layout_sync.refreshed);
        assert!(!layout_sync.added_nodes);
        assert_eq!(window_tree.node_for_element(element_id), Some(render_id));
        assert_eq!(
            window_tree.tree().element_bounds(render_id).unwrap(),
            Rect::new(2.0, 3.0, 36.0, 12.0)
        );
    }

    #[test]
    fn an_element_without_retained_paint_does_not_swallow_its_retained_children() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let grandchild_paints = Rc::new(Cell::new(0));
        let grandchild = V2Grandchild {
            paints: grandchild_paints.clone(),
        }
        .boxed();
        let grandchild_id = grandchild.id();
        let legacy_child = LegacyBranch { child: grandchild }.boxed();
        let legacy_element_id = legacy_child.id();
        let parent_paints = Rc::new(Cell::new(0));
        let root = V2Parent {
            child: legacy_child,
            paints: parent_paints.clone(),
        }
        .boxed();
        let root_id = root.id();

        root.rebuild_if_dirty(&context);
        let mut window_tree = WindowRenderTree::default();
        let sync = window_tree
            .sync(Some(&root), &context, (100, 80), 1.0)
            .unwrap();
        assert!(sync.added_nodes);

        for _ in 0..2 {
            canvas.begin_frame();
            aimer_widget::begin_paint_frame(100, 80);
            let tree = window_tree.tree.clone();
            let nodes = window_tree.element_nodes.clone();
            aimer_widget::with_v2_render_tree_context(tree, nodes, Some(root_id), || {
                context.canvas.save();
                root.update(&context);
                context.canvas.restore();
            });
            window_tree
                .sync(Some(&root), &context, (100, 80), 1.0)
                .unwrap();
        }

        let root_node = window_tree.node_for_element(root_id).unwrap();
        let legacy_node = window_tree.node_for_element(legacy_element_id).unwrap();
        let grandchild_node = window_tree.node_for_element(grandchild_id).unwrap();
        // The legacy-only branch paints nothing itself and owns nothing below
        // it: the retained grandchild paints on its own.
        assert_eq!(parent_paints.get(), 1);
        assert_eq!(grandchild_paints.get(), 1);
        for node in [root_node, grandchild_node] {
            assert_eq!(
                window_tree.tree.paint_source(node).unwrap(),
                RenderPaintSource::LocalV2
            );
        }
        assert_eq!(
            window_tree.tree.paint_source(legacy_node).unwrap(),
            RenderPaintSource::Unresolved,
            "with no paint to record it is asked again next frame"
        );
        assert_eq!(window_tree.tree.draw_list_revision(root_node).unwrap(), 1);

        let draw_items = window_tree
            .tree
            .render_all()
            .into_iter()
            .filter_map(|operation| match operation {
                RenderOp::Draw(item) => Some((item.element, item.paint_source)),
                RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
            })
            .collect::<Vec<_>>();
        let painted = draw_items
            .iter()
            .filter(|(_, source)| *source == RenderPaintSource::LocalV2)
            .map(|(element, _)| *element)
            .collect::<Vec<_>>();
        assert_eq!(painted, [root_node, grandchild_node]);
        let position = |node| draw_items.iter().position(|(element, _)| *element == node);
        assert!(position(root_node).unwrap() < position(grandchild_node).unwrap());
    }

    #[test]
    fn census_leaves_an_element_without_retained_paint_unresolved_and_counts_its_children() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        // The grandchild could paint locally and nothing stops it: its
        // legacy-only parent paints nothing and owns nothing.
        let grandchild = V2Grandchild {
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let legacy_child = LegacyBranch { child: grandchild }.boxed();
        let legacy_id = legacy_child.id();
        let root = V2Parent {
            child: legacy_child,
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();

        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);
        let census = window_tree.paint_source_census(root.as_ref());

        assert_eq!(census.local_v2, 2);
        assert_eq!(census.unresolved, 1);
        assert_eq!(
            census.unresolved_roots,
            vec![CensusElement {
                element: legacy_id,
                debug_name: "LegacyBranch",
            }]
        );
    }

    #[test]
    fn census_flags_an_element_that_draws_a_child_the_render_tree_cannot_see() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let child = V2Grandchild {
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let child_id = child.id();
        let root = HiddenChildParent { child }.boxed();

        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);
        let census = window_tree.paint_source_census(root.as_ref());

        // The child ran `draw` but was never given a render node, so whatever it
        // painted was dropped: exactly what must never go unnoticed.
        assert_eq!(
            census.drawn_unmapped,
            vec![CensusElement {
                element: child_id,
                debug_name: "V2Grandchild",
            }]
        );
    }

    #[test]
    fn census_does_not_flag_elements_that_are_exposed_to_the_render_tree() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let root = V2Parent {
            child: V2Grandchild {
                paints: Rc::new(Cell::new(0)),
            }
            .boxed(),
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();

        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        assert!(
            window_tree
                .paint_source_census(root.as_ref())
                .drawn_unmapped
                .is_empty()
        );
    }

    #[test]
    fn a_node_with_non_finite_bounds_does_not_abort_the_whole_sync() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let root = V2Parent {
            child: NonFiniteLeaf.boxed(),
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let root_id = root.id();
        let leaf_id = {
            let mut id = None;
            root.visit_children(&mut |child| id = Some(child.id()));
            id.unwrap()
        };

        let mut window_tree = WindowRenderTree::default();
        // One unusable node used to fail the sync, which sent every element of
        // the page down the full legacy repaint path.
        window_tree
            .sync(Some(&root), &context, (100, 80), 1.0)
            .expect("a single bad node must not fail the sync");

        let root_node = window_tree
            .node_for_element(root_id)
            .expect("the parent is mapped");
        let leaf_node = window_tree
            .node_for_element(leaf_id)
            .expect("the bad node is still mapped");
        assert!(window_tree.tree.element_bounds(root_node).is_ok());
        // It paints nothing visible: its bounds collapse to an empty rectangle.
        let bounds = window_tree.tree.element_bounds(leaf_node).unwrap();
        assert!(bounds.x.is_finite() && bounds.y.is_finite());
        assert_eq!((bounds.width, bounds.height), (0.0, 0.0));

        // ... and the problem stays visible rather than being swallowed.
        let census = window_tree.paint_source_census(root.as_ref());
        assert_eq!(census.sync_error, None);
        assert_eq!(
            census.invalid_bounds,
            vec![CensusElement {
                element: leaf_id,
                debug_name: "NonFiniteLeaf",
            }]
        );
    }

    #[test]
    fn census_reports_why_the_render_tree_could_not_synchronize() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let broken = V2Grandchild {
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let mut window_tree = WindowRenderTree::default();

        // A zero scale cannot be rendered at all.
        assert!(
            window_tree
                .sync(Some(&broken), &context, (100, 80), 0.0)
                .is_err()
        );
        // A failed sync sends the whole frame down the legacy repaint path, so
        // the reason must be visible rather than swallowed.
        assert_eq!(
            window_tree
                .paint_source_census(broken.as_ref())
                .sync_error
                .as_deref(),
            Some("InvalidBounds")
        );

        // The next successful sync clears it.
        let healthy = V2Grandchild {
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        window_tree
            .sync(Some(&healthy), &context, (100, 80), 1.0)
            .unwrap();
        assert_eq!(
            window_tree.paint_source_census(healthy.as_ref()).sync_error,
            None
        );
    }

    #[test]
    fn census_separates_elements_that_have_no_render_node_yet() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let root = V2Parent {
            child: LegacyBranch {
                child: V2Grandchild {
                    paints: Rc::new(Cell::new(0)),
                }
                .boxed(),
            }
            .boxed(),
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        root.rebuild_if_dirty(&context);

        // Never synchronized: the tree knows none of these elements.
        let window_tree = WindowRenderTree::default();
        let census = window_tree.paint_source_census(root.as_ref());

        assert_eq!(census.unmapped, 3);
        // Unmapped elements are also unresolved: they have no paint source.
        assert_eq!(census.unresolved, 3);
    }

    #[test]
    fn census_names_the_roots_of_regions_that_never_selected_a_paint_source() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let grandchild = V2Grandchild {
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let child = LegacyBranch { child: grandchild }.boxed();
        let root = V2Parent {
            child,
            paints: Rc::new(Cell::new(0)),
        }
        .boxed();
        let root_id = root.id();

        // Synchronized but never drawn: no node has chosen a paint source.
        root.rebuild_if_dirty(&context);
        let mut window_tree = WindowRenderTree::default();
        window_tree
            .sync(Some(&root), &context, (100, 80), 1.0)
            .unwrap();
        let census = window_tree.paint_source_census(root.as_ref());

        assert_eq!(census.unresolved, 3);
        // Synchronized elements all have a render node, so none are unmapped.
        assert_eq!(census.unmapped, 0);
        // Only the top of the unresolved region is named, not each descendant.
        assert_eq!(
            census.unresolved_roots,
            vec![CensusElement {
                element: root_id,
                debug_name: "V2Parent",
            }]
        );
    }

    /// A wrapper that adds nothing hands `draw`'s own context to its child. The
    /// sync must do the same: feeding the wrapper's content size back down as
    /// the constraint shrinks a padded descendant once per wrapper, so a centred
    /// column ended up higher than `draw` placed it.
    #[test]
    fn transparent_wrappers_do_not_shrink_what_a_padded_descendant_is_offered() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, mut context) = build_context(&canvas, 1.0);
        context.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 80.0,
        };
        let leaf = || V2PaintLeaf {
            position: Vec2d { x: 0.0, y: 0.0 },
            width: 10.0,
            height: 10.0,
            color: Color::RED,
            paints: Rc::new(Cell::new(0)),
        };
        let padded_column = || {
            Container::new()
                .color(Color::BLUE)
                .padding(aimer_style::LayoutSpacing {
                    top: aimer_style::Spacing::Px(10),
                    ..Default::default()
                })
                .child(
                    aimer_flex::Column::new()
                        .vertical_alignment(aimer_flex::BoxAlignment::Center)
                        .children([leaf()]),
                )
        };
        let mut root = padded_column().to_element(&context);
        for _ in 0..4 {
            root = aimer_widget::Element::boxed(PassThrough { child: root });
        }
        let mut window_tree = WindowRenderTree::default();

        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        // The context is 100x80 and the padding takes 10 of it. A 10px leaf
        // centred in the remaining 70px sits at 10 + (70 - 10) / 2 = 40.
        let mut pending = vec![root.as_ref() as &dyn Element];
        let mut leaf_y = None;
        while let Some(element) = pending.pop() {
            if element.debug_name() == "V2PaintLeaf" {
                let node = window_tree
                    .node_for_element(element.id())
                    .expect("leaf is mapped");
                leaf_y = Some(window_tree.tree.element_bounds(node).unwrap().y);
            }
            element.visit_retained_v2_children(&mut |_, child| pending.push(child));
        }
        assert_eq!(leaf_y, Some(40.0));
    }

    /// A node whose paint extends past its layout box (a shadow) is given
    /// larger bounds, which moves its origin outward. Its children are placed
    /// relative to the layout box, so they must not move with it.
    #[test]
    fn paint_outsets_do_not_displace_the_children_of_a_decorated_node() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, mut context) = build_context(&canvas, 1.0);
        context.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 80.0,
        };
        let shadow = aimer_style::BoxShadow {
            blur: 4.0,
            ..aimer_style::BoxShadow::new()
        };
        let container = Container::new()
            .width(Dimension::Px(60.0))
            .height(Dimension::Px(40.0))
            .box_decoration(aimer_style::BoxDecoration::new().box_shadow([shadow]))
            .child(V2PaintLeaf {
                position: Vec2d { x: 0.0, y: 0.0 },
                width: 10.0,
                height: 10.0,
                color: Color::RED,
                paints: Rc::new(Cell::new(0)),
            })
            .to_element(&context);
        let container_id = container.id();
        let root = V2PaintTreeRoot {
            children: [
                container,
                aimer_widget::Element::boxed(V2PaintLeaf {
                    position: Vec2d { x: 70.0, y: 0.0 },
                    width: 5.0,
                    height: 5.0,
                    color: Color::GREEN,
                    paints: Rc::new(Cell::new(0)),
                }),
            ],
        }
        .boxed();
        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        let node_of = |id| window_tree.node_for_element(id).expect("mapped");
        let container_bounds = window_tree
            .tree
            .element_bounds(node_of(container_id))
            .unwrap();
        let mut leaf_bounds = None;
        let mut pending = vec![root.as_ref() as &dyn Element];
        while let Some(element) = pending.pop() {
            if element.debug_name() == "V2PaintLeaf"
                && element.pos() != Some(Vec2d { x: 70.0, y: 0.0 })
            {
                leaf_bounds = Some(
                    window_tree
                        .tree
                        .element_bounds(node_of(element.id()))
                        .unwrap(),
                );
            }
            element.visit_retained_v2_children(&mut |_, child| pending.push(child));
        }
        let leaf_bounds = leaf_bounds.expect("the container's child is in the tree");

        // The shadow makes the container's own node larger than 60x40 ...
        assert!(container_bounds.width > 60.0, "{container_bounds:?}");
        // ... but the child sits at the layout box's origin, which is the
        // origin of the container's *layout* box (0, 0), not of the larger node.
        assert_eq!(
            (leaf_bounds.x, leaf_bounds.y),
            (0.0, 0.0),
            "{leaf_bounds:?}"
        );
    }

    #[test]
    fn the_bounds_audit_compares_a_cached_rectangle_with_its_render_node() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let container = Container::new()
            .width(Dimension::Px(60.0))
            .height(Dimension::Px(40.0))
            .child(V2PaintLeaf {
                position: Vec2d { x: 0.0, y: 0.0 },
                width: 20.0,
                height: 20.0,
                color: Color::RED,
                paints: Rc::new(Cell::new(0)),
            })
            .to_element(&context);
        let container_id = container.id();
        let root = V2PaintTreeRoot {
            children: [
                container,
                aimer_widget::Element::boxed(V2PaintLeaf {
                    position: Vec2d { x: 70.0, y: 0.0 },
                    width: 10.0,
                    height: 10.0,
                    color: Color::GREEN,
                    paints: Rc::new(Cell::new(0)),
                }),
            ],
        }
        .boxed();
        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        let audit = window_tree.bounds_audit(root.as_ref());

        assert!(
            audit.compared >= 1,
            "a sized container publishes a rectangle: {audit:?}"
        );
        assert_eq!(audit.unmapped, 0);
        assert!(
            audit
                .mismatches
                .iter()
                .all(|mismatch| mismatch.element != container_id),
            "a fixed-size container's cached box is its render node: {audit:?}"
        );
    }

    #[test]
    fn rounded_container_stays_on_the_v2_path_and_clips_its_child_with_its_radius() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let container = Container::new()
            .width(Dimension::Px(60.0))
            .height(Dimension::Px(40.0))
            .box_decoration(
                aimer_style::BoxDecoration::new()
                    .background_color(Color::BLUE)
                    .border_radius(8),
            )
            .child(V2PaintLeaf {
                position: Vec2d { x: 0.0, y: 0.0 },
                width: 20.0,
                height: 20.0,
                color: Color::RED,
                paints: Rc::new(Cell::new(0)),
            })
            .to_element(&context);
        let container_id = container.id();
        let child_id = {
            let mut child_id = None;
            container.visit_children(&mut |child| child_id = Some(child.id()));
            child_id.expect("Container exposes its child render element")
        };
        let sibling = aimer_widget::Element::boxed(V2PaintLeaf {
            position: Vec2d { x: 70.0, y: 0.0 },
            width: 10.0,
            height: 10.0,
            color: Color::GREEN,
            paints: Rc::new(Cell::new(0)),
        });
        let root = V2PaintTreeRoot {
            children: [container, sibling],
        }
        .boxed();

        let mut window_tree = WindowRenderTree::default();
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);
        let census = window_tree.paint_source_census(root.as_ref());

        // A rounded container used to become a legacy island and swallow its
        // whole subtree.
        assert_eq!(
            window_tree
                .tree
                .paint_source(window_tree.node_for_element(container_id).unwrap())
                .unwrap(),
            RenderPaintSource::LocalV2
        );

        // Its child is clipped to the container's rounded interior.
        let child_node = window_tree.node_for_element(child_id).unwrap();
        let child_clip = window_tree
            .tree
            .render_all()
            .into_iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == child_node => {
                    Some((item.clip, item.clip_radius))
                }
                RenderOp::Draw(_)
                | RenderOp::BeginOpacityGroup { .. }
                | RenderOp::EndOpacityGroup { .. } => None,
            });
        assert_eq!(
            child_clip,
            Some((Some(Rect::new(0.0, 0.0, 60.0, 40.0)), [8.0; 4]))
        );
    }

    #[test]
    fn container_background_repaints_locally_and_clips_its_v2_child() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, context) = build_context(&canvas, 1.0);
        let child_paints = Rc::new(Cell::new(0));
        let container = Container::new()
            .width(Dimension::Px(30.0))
            .height(Dimension::Px(20.0))
            .color(Color::BLUE)
            .child(V2PaintLeaf {
                position: Vec2d { x: -5.0, y: -5.0 },
                width: 40.0,
                height: 30.0,
                color: Color::RED,
                paints: child_paints.clone(),
            })
            .to_element(&context);
        let container_id = container.id();
        let child_id = {
            let mut child_id = None;
            container.visit_children(&mut |child| child_id = Some(child.id()));
            child_id.expect("Container exposes its child render element")
        };
        let sibling_paints = Rc::new(Cell::new(0));
        let sibling = aimer_widget::Element::boxed(V2PaintLeaf {
            position: Vec2d { x: 60.0, y: 0.0 },
            width: 12.0,
            height: 12.0,
            color: Color::GREEN,
            paints: sibling_paints.clone(),
        });
        let sibling_id = sibling.id();
        let root = V2PaintTreeRoot {
            children: [container, sibling],
        }
        .boxed();
        let root_id = root.id();
        root.rebuild_if_dirty(&context);

        let mut window_tree = WindowRenderTree::default();
        window_tree
            .sync(Some(&root), &context, (100, 80), 1.0)
            .unwrap();
        let container_node = window_tree.node_for_element(container_id).unwrap();
        let child_node = window_tree.node_for_element(child_id).unwrap();
        let sibling_node = window_tree.node_for_element(sibling_id).unwrap();
        let retained_tree = window_tree.tree.clone();
        let element_nodes = window_tree.element_nodes.clone();
        let draw_frame = || {
            context.canvas.begin_frame();
            aimer_widget::begin_paint_frame(100, 80);
            aimer_widget::with_v2_render_tree_context(
                retained_tree.clone(),
                element_nodes.clone(),
                Some(root_id),
                || {
                    context.canvas.save();
                    root.update(&context);
                    context.canvas.restore();
                },
            );
        };

        draw_frame();
        let background_before = retained_tree.draw_list_snapshot(container_node).unwrap();
        assert_eq!(background_before.revision, 1);
        assert!(matches!(
            background_before.commands.as_ref(),
            [aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, .. }]
                if (color.r, color.g, color.b, color.a) == (0, 0, 255, 255)
        ));
        let child_clip =
            retained_tree
                .render_all()
                .into_iter()
                .find_map(|operation| match operation {
                    RenderOp::Draw(item) if item.element == child_node => Some(item.clip),
                    RenderOp::Draw(_)
                    | RenderOp::BeginOpacityGroup { .. }
                    | RenderOp::EndOpacityGroup { .. } => None,
                });
        assert_eq!(child_clip, Some(Some(Rect::new(0.0, 0.0, 30.0, 20.0))));
        assert_eq!(retained_tree.draw_list_revision(sibling_node).unwrap(), 1);

        retained_tree.invalidate_paint(container_node).unwrap();
        draw_frame();

        assert_eq!(
            retained_tree.draw_list_revision(container_node).unwrap(),
            background_before.revision + 1
        );
        assert_eq!(retained_tree.draw_list_revision(child_node).unwrap(), 1);
        assert_eq!(retained_tree.draw_list_revision(sibling_node).unwrap(), 1);
        assert_eq!(child_paints.get(), 1);
        assert_eq!(sibling_paints.get(), 1);
    }

    #[test]
    fn scroll_v2_moves_a_clipped_child_and_damages_only_the_viewport() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, mut context) = build_context(&canvas, 1.0);
        context.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 80.0,
        };
        let controller = ScrollController::new();
        let child_paints = Rc::new(Cell::new(0));
        let root = Scrollable::new()
            .controller(controller.clone())
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None)
            .child(ScrollContentClip {
                child: V2PaintLeaf {
                    position: Vec2d::ZERO,
                    width: 100.0,
                    height: 1_000.0,
                    color: Color::RED,
                    paints: child_paints.clone(),
                },
            })
            .to_element(&context);
        let scroll_element = find_element_id(root.as_ref(), "RawScrollableContainer")
            .expect("Scrollable exposes its retained viewport");
        let content_element = find_element_id(root.as_ref(), "ScrollContentClipTest")
            .expect("the viewport exposes its content clip node");
        let leaf_element = find_element_id(root.as_ref(), "V2PaintLeaf")
            .expect("the clipped content exposes its painted child");
        let mut window_tree = WindowRenderTree::default();

        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        assert!(controller.is_attached());
        let scroll_node = window_tree.node_for_element(scroll_element).unwrap();
        let content_node = window_tree.node_for_element(content_element).unwrap();
        let leaf_node = window_tree.node_for_element(leaf_element).unwrap();
        assert_eq!(
            window_tree.tree.paint_source(scroll_node).unwrap(),
            RenderPaintSource::LocalV2
        );
        assert_eq!(
            window_tree.tree.paint_source(leaf_node).unwrap(),
            RenderPaintSource::LocalV2
        );

        window_tree.tree.take_damage();
        controller.jump_to(Vec2d { x: 0.0, y: 400.0 });
        draw_v2_until_tree_is_current(&root, &context, &mut window_tree);

        let content_item = window_tree
            .tree
            .render_all()
            .into_iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == leaf_node => Some(item),
                RenderOp::Draw(_)
                | RenderOp::BeginOpacityGroup { .. }
                | RenderOp::EndOpacityGroup { .. } => None,
            })
            .unwrap_or_else(|| {
                let snapshot = window_tree.tree.draw_list_snapshot(leaf_node).unwrap();
                let items = window_tree
                    .tree
                    .render_all()
                    .into_iter()
                    .filter_map(|operation| match operation {
                        RenderOp::Draw(item) => Some((
                            item.element,
                            item.origin,
                            item.bounds,
                            item.clip,
                            item.paint_source,
                        )),
                        RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                panic!(
                    "the clipped content has no render item: ids=({scroll_element:?},{content_element:?},{leaf_element:?}), nodes=({scroll_node:?},{content_node:?},{leaf_node:?}), source={:?}, bounds={:?}, scroll={:?}, content={:?}, revision={}, commands={}, items={:?}",
                    window_tree.tree.paint_source(leaf_node),
                    window_tree.tree.element_bounds(leaf_node),
                    window_tree.tree.element_bounds(scroll_node),
                    window_tree.tree.element_bounds(content_node),
                    snapshot.revision,
                    snapshot.commands.len(),
                    items,
                )
            });
        assert_eq!(content_item.origin, (0.0, -400.0));
        assert_eq!(content_item.clip, Some(Rect::new(10.0, 0.0, 80.0, 80.0)));
        assert_eq!(
            window_tree.tree.element_bounds(scroll_node).unwrap(),
            Rect::new(0.0, 0.0, 100.0, 80.0)
        );

        let damage = window_tree.tree.take_damage();
        assert!(
            !damage.is_empty(),
            "scrolling must damage the retained viewport"
        );
        assert!(
            damage.iter().all(|rect| {
                rect.x >= 0.0
                    && rect.y >= 0.0
                    && rect.x + rect.width <= 100.0
                    && rect.y + rect.height <= 80.0
            }),
            "scroll damage escaped the viewport: {damage:?}"
        );
        assert!(child_paints.get() >= 1);
    }

    #[test]
    fn virtualized_scroll_v2_updates_only_windowed_rows_and_viewport_damage() {
        let canvas = aimer_canvas::InnerCanvas::new();
        let (_runtime, mut context) = build_context(&canvas, 1.0);
        context.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 80.0,
        };
        let controller = ScrollController::new();
        let rows = Column::new()
            .list(0..1_000_u32)
            .item_extent(Dimension::Px(20.0))
            .builder(move |_| V2PaintLeaf {
                position: Vec2d::ZERO,
                width: 100.0,
                height: 20.0,
                color: Color::RED,
                paints: Rc::new(Cell::new(0)),
            });
        let root = Scrollable::new()
            .controller(controller.clone())
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None)
            .child(rows)
            .to_element(&context);
        let scroll_element = find_element_id(root.as_ref(), "RawScrollableContainer")
            .expect("Scrollable exposes its retained viewport");
        let mut window_tree = WindowRenderTree::default();

        let initial_draws = draw_v2_until_tree_is_current(&root, &context, &mut window_tree);
        assert_eq!(
            initial_draws, 1,
            "initial virtual rows draw in the first frame"
        );

        let initial_rows = find_element_ids(root.as_ref(), "V2PaintLeaf");
        assert!(!initial_rows.is_empty(), "the viewport materializes rows");
        assert!(initial_rows.len() < 32, "the list should remain windowed");
        let initial_render_nodes = initial_rows
            .iter()
            .map(|element| {
                window_tree
                    .node_for_element(*element)
                    .expect("each materialized row has a retained render node")
            })
            .collect::<Vec<_>>();
        assert!(
            initial_render_nodes.iter().all(|node| {
                window_tree.tree.paint_source(*node) == Ok(RenderPaintSource::LocalV2)
                    && window_tree.tree.draw_list_revision(*node) == Ok(1)
            }),
            "newly materialized rows record v2 paint in the first frame"
        );
        window_tree.tree.take_damage();

        controller.jump_to(Vec2d { x: 0.0, y: 400.0 });
        let scrolled_draws = draw_v2_until_tree_is_current(&root, &context, &mut window_tree);
        assert_eq!(
            scrolled_draws, 1,
            "new virtual rows draw in the scroll frame"
        );

        let updated_rows = find_element_ids(root.as_ref(), "V2PaintLeaf");
        assert!(
            !updated_rows.is_empty(),
            "the scrolled viewport materializes rows"
        );
        assert!(
            updated_rows.len() < 32,
            "scrolling must not materialize the full list"
        );
        let updated_render_nodes = updated_rows
            .iter()
            .map(|element| {
                window_tree
                    .node_for_element(*element)
                    .expect("each scrolled row has a retained render node")
            })
            .collect::<Vec<_>>();
        assert!(
            updated_render_nodes.iter().all(|node| {
                window_tree.tree.paint_source(*node) == Ok(RenderPaintSource::LocalV2)
                    && window_tree.tree.draw_list_revision(*node) == Ok(1)
            }),
            "newly visible rows record v2 paint in the scroll frame"
        );
        let visible_render_items = window_tree
            .tree
            .render_all()
            .into_iter()
            .filter_map(|operation| match operation {
                RenderOp::Draw(item) => {
                    Some((item.element, item.paint_source, item.bounds, item.clip))
                }
                RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
            })
            .collect::<Vec<_>>();
        let mut render_sources = Vec::new();
        let mut pending = vec![root.as_ref() as &dyn Element];
        while let Some(element) = pending.pop() {
            if let Some(node) = window_tree.node_for_element(element.id()) {
                render_sources.push((
                    element.debug_name(),
                    node,
                    window_tree.tree.paint_source(node).unwrap(),
                    window_tree.tree.element_bounds(node).unwrap(),
                ));
            }
            element.visit_children(&mut |child| pending.push(child));
        }
        assert!(
            visible_render_items
                .iter()
                .any(|(render_node, paint_source, _, _)| {
                    updated_render_nodes.contains(render_node)
                        && *paint_source == RenderPaintSource::LocalV2
                }),
            "the scroll frame plan includes visible virtualized v2 rows: items={visible_render_items:?}, sources={render_sources:?}, row_nodes={updated_render_nodes:?}"
        );
        assert_ne!(
            initial_render_nodes, updated_render_nodes,
            "scrolling advances the virtualized row window"
        );
        assert_eq!(
            window_tree
                .tree
                .element_bounds(window_tree.node_for_element(scroll_element).unwrap())
                .unwrap(),
            Rect::new(0.0, 0.0, 100.0, 80.0)
        );

        let damage = window_tree.tree.take_damage();
        assert!(
            !damage.is_empty(),
            "virtualized scrolling damages the viewport"
        );
        assert!(
            damage.iter().all(|rect| {
                rect.x >= 0.0
                    && rect.y >= 0.0
                    && rect.x + rect.width <= 100.0
                    && rect.y + rect.height <= 80.0
            }),
            "virtualized row damage escaped the viewport: damage={damage:?}, sources={render_sources:?}, items={visible_render_items:?}"
        );
    }

    #[test]
    fn startup_hooks_run_once_in_registration_order() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let mut hooks: Vec<Box<dyn FnOnce() -> Box<dyn Any>>> = [1_u8, 2, 3]
            .into_iter()
            .map(|call| {
                let calls = calls.clone();
                Box::new(move || {
                    calls.borrow_mut().push(call);
                    Box::new(call) as Box<dyn Any>
                }) as Box<dyn FnOnce() -> Box<dyn Any>>
            })
            .collect();
        let mut resources = Vec::new();

        run_startup_hooks(&mut hooks, &mut resources);
        run_startup_hooks(&mut hooks, &mut resources);

        assert_eq!(*calls.borrow(), vec![1, 2, 3]);
        assert!(hooks.is_empty());
        assert_eq!(
            resources
                .iter()
                .map(|resource| *resource.downcast_ref::<u8>().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn missing_startup_hooks_are_a_no_op() {
        let mut hooks = Vec::new();
        let mut resources = Vec::new();

        run_startup_hooks(&mut hooks, &mut resources);

        assert!(hooks.is_empty());
        assert!(resources.is_empty());
    }
}
