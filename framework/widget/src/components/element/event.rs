use super::*;
use crate::event_tree::{EventTargetId, EventTree};

type EventChildren<'a> = smallvec::SmallVec<[&'a dyn Element; 32]>;

/// A compact link from an indexed element to its structural parent.
///
/// The public name is retained for source compatibility, but the dispatcher no
/// longer stores a separately allocated root-relative slice for every element.
/// Links live in one reusable arena and are followed only when a captured owner
/// has to be resolved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ElementPath {
    parent: Option<usize>,
    child_index: u32,
}

/// Provides a routed element with the dispatcher's shared pointer-capture
/// state while it forwards an event into a private child view.
///
/// The context is valid only for the duration of
/// [`EventElement::on_event_with_context`]. It is intentionally opaque: a
/// forwarding element may query capture state for its boundary and dispatch
/// its child, but it cannot retain the context after the current event.
pub struct EventDispatchContext<'dispatcher, 'tree> {
    dispatcher: &'dispatcher mut EventDispatcher,
    path_root: &'tree dyn Element,
    boundary: Option<ElementId>,
    position: Vec2d,
}

impl<'dispatcher, 'tree> EventDispatchContext<'dispatcher, 'tree> {
    #[inline]
    fn new(
        dispatcher: &'dispatcher mut EventDispatcher,
        path_root: &'tree dyn Element,
        boundary: Option<ElementId>,
        position: Vec2d,
    ) -> Self {
        Self {
            dispatcher,
            path_root,
            boundary,
            position,
        }
    }

    /// The hit-test position supplied by the dispatcher, including for Scroll
    /// events whose payload has no pointer position.
    #[inline]
    pub fn position(&self) -> Vec2d {
        self.position
    }

    /// Returns whether a pointer is captured by this forwarding boundary.
    #[inline]
    pub fn is_captured(&self, pointer: PointerKey) -> bool {
        self.boundary.is_some_and(|boundary| {
            self.dispatcher
                .nested_captures
                .contains_key(&(boundary, pointer))
        })
    }

    /// Takes `pointer` back from everything inside this element's child view that
    /// captured it, telling each that its interaction is cancelled.
    ///
    /// A forwarding element that wins an arbitration after its child has already
    /// been pressed uses this, as a scroll does when it takes a drag from a button.
    /// The captures are found by pointer, at any depth, because a child that is
    /// itself a forwarding boundary (a retained child is one) records its own
    /// descendant's capture under its own boundary, where a cancellation sent to
    /// the child as a whole would not find it.
    pub fn cancel_pointer(&mut self, pointer: PointerKey) -> EventResult {
        self.dispatcher.cancel_nested_pointer(self.path_root, pointer, self.boundary)
    }

    /// Hands the focus target found by a press in `child`, a private dispatcher
    /// that [defers its focus](EventDispatcher::defer_focus_to_parent), to the
    /// dispatcher this event is being routed by, which then moves focus as it
    /// does for any other press.
    #[inline]
    pub fn adopt_deferred_focus(&mut self, child: &mut EventDispatcher) {
        let found = child.deferred_focus_owner.take();
        if self.dispatcher.nested_focus_owner.is_none() {
            self.dispatcher.nested_focus_owner = found;
        }
    }

    /// Dispatches an event into the forwarding element's child view using the
    /// same path index and capture state as the owning dispatcher.
    #[inline]
    pub fn dispatch_child(
        &mut self,
        child: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
    ) -> EventResult {
        self.dispatcher
            .dispatch_nested(self.path_root, self.boundary, child, pos, event)
    }
}

/// One retained element in the last uncaptured pointer hit chain.
///
/// The pointer is valid only while the dispatch root has the same subtree
/// generation and address as the cache entry. `EventDispatcher` is UI-thread
/// state, and the cache is retired before either condition can be reused.
#[derive(Clone, Copy)]
struct CachedHitElement {
    id: ElementId,
    element: *const (dyn Element + 'static),
}

struct CachedHitChain {
    pointer: PointerKey,
    root: ElementId,
    generation: u64,
    last_position: Vec2d,
    elements: SmallVec<[CachedHitElement; 16]>,
}

/// Retains an element pointer for a same-generation dispatch side path.
///
/// The caller must discard the pointer before the dispatch root changes its
/// identity or subtree generation. That is the same lifetime invariant used by
/// [`CachedHitElement`].
#[inline]
fn retained_element_pointer(element: &dyn Element) -> *const (dyn Element + 'static) {
    // SAFETY: callers keep this pointer only while the retained element tree
    // remains at the same root and subtree generation.
    unsafe { std::mem::transmute::<&dyn Element, *const (dyn Element + 'static)>(element) }
}

struct HoverHitChain {
    pointer: PointerKey,
    root: ElementId,
    generation: u64,
    elements: SmallVec<[ElementId; 16]>,
}

/// O(1) lookup data aligned with the structural path links.
///
/// The element pointer is valid only while the dispatcher's indexed root
/// address and subtree generation still match the tree it was built from.
#[derive(Clone, Copy)]
struct IndexedElement {
    element: *const (dyn Element + 'static),
    has_structural_children: bool,
}

struct HitChainRecorder {
    elements: SmallVec<[CachedHitElement; 16]>,
    hover_elements: SmallVec<[ElementId; 16]>,
    empty_hit_test_node_has_children: bool,
    cacheable: bool,
    forwarding_boundary: bool,
}

impl HitChainRecorder {
    #[inline]
    fn new() -> Self {
        Self {
            elements: SmallVec::new(),
            hover_elements: SmallVec::new(),
            empty_hit_test_node_has_children: false,
            cacheable: true,
            forwarding_boundary: false,
        }
    }

    #[inline]
    fn record_element(&mut self, element: &dyn Element) {
        if let Some(id) = element.element_id() {
            self.hover_elements.push(id);
        }
        if !self.cacheable || self.forwarding_boundary {
            return;
        }
        if is_indexed_hit_test_boundary(element.event_tree_role()) {
            // A boundary can change its eligible child branches without any
            // target bounds moving. Replaying a single cached chain would skip
            // its position-aware child selection on the next pointer move.
            self.cacheable = false;
            self.elements.clear();
            return;
        }
        let Some(id) = element.element_id() else {
            self.cacheable = false;
            self.elements.clear();
            return;
        };
        if element.has_overlapping_hit_targets() {
            self.cacheable = false;
            self.elements.clear();
            return;
        }
        self.elements.push(CachedHitElement {
            id,
            // The pointer is retained only behind the generation/root checks
            // in `dispatch_cached_hit_chain`.
            element: retained_element_pointer(element),
        });
    }

    #[inline]
    fn record_hit_test_children(&mut self, count: usize) {
        if self.forwarding_boundary || count <= 1 {
            return;
        }
        self.cacheable = false;
        self.elements.clear();
    }

    #[inline]
    fn record_empty_hit_test_node(&mut self, has_structural_children: bool) {
        if !self.cacheable
            || self.forwarding_boundary
            || self.elements.is_empty()
        {
            return;
        }
        self.empty_hit_test_node_has_children |= has_structural_children;
    }

    #[inline]
    fn record_miss(&mut self) {
        if self.forwarding_boundary {
            return;
        }
        self.cacheable = false;
        self.elements.clear();
    }

    #[inline]
    fn mark_forwarding_boundary(&mut self) {
        if self.cacheable && !self.elements.is_empty() {
            self.forwarding_boundary = true;
        }
    }

    #[inline]
    fn finish_hover(
        &self,
        pointer: PointerKey,
        root: &dyn Element,
        generation: u64,
    ) -> Option<HoverHitChain> {
        Some(HoverHitChain {
            pointer,
            root: root.element_id()?,
            generation,
            elements: self.hover_elements.clone(),
        })
    }

    #[inline]
    fn finish(
        self,
        pointer: PointerKey,
        root: &dyn Element,
        generation: u64,
        position: Vec2d,
    ) -> Option<CachedHitChain> {
        let root_id = root.element_id()?;
        let first = self.elements.first()?;
        let empty_node_has_children =
            !self.forwarding_boundary && self.empty_hit_test_node_has_children;
        (self.cacheable
            && !empty_node_has_children
            && first.id == root_id)
            .then_some(CachedHitChain {
                pointer,
                root: root_id,
                generation,
                last_position: position,
                elements: self.elements,
            })
    }
}

#[derive(Default)]
struct EventHitFrame {
    marks: Vec<u32>,
    boundary_filtered: Vec<u32>,
    epoch: u32,
    roots: Vec<EventTargetId>,
    active_children: Vec<Vec<EventTargetId>>,
    touched_parents: Vec<usize>,
}

impl EventHitFrame {
    fn prepare(&mut self, target_count: usize) -> u32 {
        for parent in self.touched_parents.drain(..) {
            if let Some(children) = self.active_children.get_mut(parent) {
                children.clear();
            }
        }
        self.roots.clear();
        self.marks.resize(target_count, 0);
        self.boundary_filtered.resize(target_count, 0);
        self.active_children.resize_with(target_count, Vec::new);

        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.marks.fill(0);
            self.boundary_filtered.fill(0);
            self.epoch = 1;
        }
        self.epoch
    }

    fn mark_hit_path(&mut self, tree: &EventTree, target: EventTargetId, epoch: u32) {
        let mut current = Some(target);
        while let Some(target) = current {
            let index = target.index();
            if self.marks[index] == epoch {
                break;
            }
            self.marks[index] = epoch;
            let parent = tree.get(target).and_then(|element| element.parent());
            if let Some(parent) = parent {
                let children = &mut self.active_children[parent.index()];
                if children.is_empty() {
                    self.touched_parents.push(parent.index());
                }
                children.push(target);
                current = Some(parent);
            } else {
                self.roots.push(target);
                current = None;
            }
        }
    }

    fn mark_boundary_filtered(&mut self, target: EventTargetId, epoch: u32) {
        self.boundary_filtered[target.index()] = epoch;
    }

    fn boundary_was_filtered(&self, target: EventTargetId, epoch: u32) -> bool {
        self.boundary_filtered.get(target.index()).copied() == Some(epoch)
    }
}

struct EventHitDepthGuard {
    depth: Rc<Cell<usize>>,
    previous: usize,
}

impl EventHitDepthGuard {
    fn enter(depth: Rc<Cell<usize>>) -> Self {
        let previous = depth.get();
        depth.set(
            previous
                .checked_add(1)
                .expect("event hit-query nesting overflow"),
        );
        Self { depth, previous }
    }
}

impl Drop for EventHitDepthGuard {
    fn drop(&mut self) {
        self.depth.set(self.previous);
    }
}

/// Routes pointer events and persists capture ownership across event calls.
///
/// Capture lookup is an average `O(1)` hash-map operation. The saved owner ID
/// resolves through a generation-checked structural index, avoiding a repeated
/// walk of the retained tree.
/// Uncaptured, non-consuming pointer moves additionally replay the last
/// single hit chain after validating its element bounds and subtree
/// generation; overlapping containers and forwarding boundaries retain their
/// conservative full-walk behavior where necessary.
pub struct EventDispatcher {
    captures: HashMap<PointerKey, ElementId>,
    nested_captures: HashMap<(ElementId, PointerKey), ElementId>,
    /// The focus target a forwarding element's private child view found for the
    /// press being routed. [`Self::dispatch_nested`] can only hand an
    /// [`EventResult`] back through [`EventDispatchContext::dispatch_child`], so
    /// the candidate waits here until the forwarding element returns and its
    /// caller merges it into the enclosing hit outcome.
    nested_focus_owner: Option<FocusCandidate<ElementId>>,
    /// Whether this dispatcher serves a private child view whose focus belongs to
    /// the dispatcher enclosing it; see [`Self::defer_focus_to_parent`].
    defers_focus: bool,
    /// The focus target the latest press found while focus is deferred, for
    /// the forwarding element to hand to the enclosing dispatcher.
    deferred_focus_owner: Option<FocusCandidate<ElementId>>,
    /// Whether the latest press landed on something that can take focus. Such a
    /// target takes the press through the focus system and need not consume
    /// it, so this is what tells a host the press was taken.
    last_press_hit_focus_target: bool,
    path_indices: HashMap<ElementId, usize>,
    path_links: Vec<ElementPath>,
    /// Preorder-aligned with `path_links`; stores direct pointers and child flags.
    indexed_elements: Vec<IndexedElement>,
    indexed_subtree_generation: u64,
    indexed_root: Option<ElementId>,
    paths_dirty: bool,
    /// The unscoped tree generation and log position the index was last brought
    /// up to date at, so a later change can be told apart as scoped.
    indexed_unscoped: u64,
    /// How many times the path index was rebuilt in full and patched in place.
    index_work: (u64, u64),
    /// Makes every path-index synchronization a full rebuild. Tests set it to
    /// compare the patched index with a rebuilt one.
    index_patching_disabled: bool,
    /// The element-tree generation the index was last brought up to date at.
    indexed_tree_generation: u64,
    /// The live-children generation the index was last rebuilt at. A windowed
    /// container drops and builds rows without replacing itself, which no tree
    /// generation reports, and every pointer to a dropped row is dangling.
    indexed_live_children: u64,
    scoped_cursor: ScopedCursor,
    /// Whether the event tree must be rebuilt because the path index was.
    event_tree_stale: bool,
    generation_checked_frame: Option<u64>,
    hit_chain_cache: Option<CachedHitChain>,
    hit_chain_recorder: Option<HitChainRecorder>,
    hover_chains: HashMap<PointerKey, HoverHitChain>,
    hover_membership_marks: Vec<u32>,
    hover_membership_epoch: u32,
    focus_scope: Option<ElementId>,
    focus: FocusManager<ElementId>,
    focus_candidates: FocusCandidates<ElementId>,
    event_tree: EventTree,
    event_target_by_element: HashMap<ElementId, EventTargetId>,
    indexed_root_address: Option<*const ()>,
    event_hit_frames: Vec<EventHitFrame>,
    event_hit_depth: Rc<Cell<usize>>,
    indexed_layout_generation: u64,
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatcher {
    /// Creates an empty dispatcher with no captured pointers.
    #[inline]
    pub fn new() -> Self {
        Self {
            captures: HashMap::new(),
            nested_captures: HashMap::new(),
            nested_focus_owner: None,
            defers_focus: false,
            deferred_focus_owner: None,
            path_indices: HashMap::new(),
            path_links: Vec::new(),
            indexed_elements: Vec::new(),
            indexed_subtree_generation: u64::MAX,
            indexed_root: None,
            paths_dirty: true,
            indexed_unscoped: u64::MAX,
            index_work: (0, 0),
            index_patching_disabled: false,
            indexed_tree_generation: u64::MAX,
            indexed_live_children: u64::MAX,
            scoped_cursor: ScopedCursor::default(),
            event_tree_stale: true,
            generation_checked_frame: None,
            hit_chain_cache: None,
            hit_chain_recorder: None,
            hover_chains: HashMap::new(),
            hover_membership_marks: Vec::new(),
            hover_membership_epoch: 0,
            focus_scope: None,
            last_press_hit_focus_target: false,
            focus: FocusManager::new(),
            focus_candidates: FocusCandidates::new(),
            event_tree: EventTree::new(),
            event_target_by_element: HashMap::new(),
            indexed_root_address: None,
            event_hit_frames: Vec::new(),
            event_hit_depth: Rc::new(Cell::new(0)),
            indexed_layout_generation: u64::MAX,
        }
    }

    /// How many times the path index was rebuilt in full, and how many times it
    /// was patched in place after a subtree was replaced by one of the same
    /// shape. For tests and diagnostics.
    #[doc(hidden)]
    pub fn path_index_work(&self) -> (u64, u64) {
        self.index_work
    }

    /// Turns patching of the path index off or on. Off, every synchronization
    /// that finds the tree changed rebuilds the index in full. For tests.
    #[doc(hidden)]
    pub fn set_index_patching(&mut self, enabled: bool) {
        self.index_patching_disabled = !enabled;
    }

    #[inline]
    fn invalidate_hit_chain(&mut self) {
        self.hit_chain_cache = None;
        self.hit_chain_recorder = None;
    }

    #[inline]
    fn begin_hit_chain_recording(&mut self) {
        self.hit_chain_recorder = Some(HitChainRecorder::new());
    }

    #[inline]
    fn record_hit_chain_element(&mut self, element: &dyn Element) {
        if let Some(recorder) = self.hit_chain_recorder.as_mut() {
            recorder.record_element(element);
        }
    }

    #[inline]
    fn record_hit_chain_children(&mut self, count: usize) {
        if let Some(recorder) = self.hit_chain_recorder.as_mut() {
            recorder.record_hit_test_children(count);
        }
    }

    #[inline]
    fn record_empty_hit_chain_node(&mut self, element: &dyn Element) {
        let has_structural_children = element
            .element_id()
            .and_then(|id| self.path_indices.get(&id).copied())
            .and_then(|index| self.indexed_elements.get(index))
            .map(|element| element.has_structural_children)
            .unwrap_or(true);
        if let Some(recorder) = self.hit_chain_recorder.as_mut() {
            recorder.record_empty_hit_test_node(has_structural_children);
        }
    }

    #[inline]
    fn record_hit_chain_miss(&mut self) {
        if let Some(recorder) = self.hit_chain_recorder.as_mut() {
            recorder.record_miss();
        }
    }

    #[inline]
    fn mark_hit_chain_forwarding_boundary(&mut self) {
        if let Some(recorder) = self.hit_chain_recorder.as_mut() {
            recorder.mark_forwarding_boundary();
        }
    }

    fn finish_hit_chain_recording(
        &mut self,
        root: &dyn Element,
        pos: Vec2d,
        pointer: PointerKey,
        outcome: &RoutedEventResult,
    ) -> Option<HoverHitChain> {
        let generation = root.subtree_generation();
        let root_id = root.element_id();
        let recorder = self.hit_chain_recorder.take()?;
        let hover_chain = recorder.finish_hover(pointer, root, generation);
        self.hit_chain_cache = if outcome.result.is_consumed()
            || !matches!(outcome.result.capture_request(), CaptureRequest::None)
            || !matches!(outcome.result.follow_up(), FollowUp::None)
            || root_id != self.indexed_root
            || generation != self.indexed_subtree_generation
        {
            None
        } else {
            recorder.finish(pointer, root, generation, pos)
        };
        hover_chain
    }

    fn dispatch_uncaptured_pointer_move(
        &mut self,
        root: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
        pointer: PointerKey,
    ) -> RoutedEventResult {
        let previous_hover = self.hover_chains.remove(&pointer);
        if let Some((outcome, current_hover)) =
            self.dispatch_cached_hit_chain(root, pos, event, pointer)
        {
            return self.finish_hover_transition(
                root,
                pointer,
                previous_hover,
                current_hover,
                outcome,
            );
        }

        self.begin_hit_chain_recording();
        let outcome = dispatch_routed_event(self, root, pos, event);
        let Some(current_hover) = self
            .finish_hit_chain_recording(root, pos, pointer, &outcome)
        else {
            return outcome;
        };
        self.finish_hover_transition(
            root,
            pointer,
            previous_hover,
            current_hover,
            outcome,
        )
    }

    fn finish_hover_transition(
        &mut self,
        root: &dyn Element,
        pointer: PointerKey,
        previous: Option<HoverHitChain>,
        current: HoverHitChain,
        mut outcome: RoutedEventResult,
    ) -> RoutedEventResult {
        // The chain of the last move is still the right list of what the pointer
        // was over when the tree has changed since: every element in it is looked
        // up again by identity below, an element that has gone is skipped, and one
        // that was replaced under the same identity is still owed its exit.
        // Dropping the chain whenever the tree changed, which any hover-driven
        // rebuild or running animation does between two moves, lost the exit of an
        // element that is not offered the move that leaves it, and left it hovered.
        if aimer_utils::pointer_trace_enabled() {
            aimer_utils::pointer_trace!(
                "hover dispatcher {:p} root {:?}, previous root {:?} (same pointer: {})",
                self as *const Self,
                current.root,
                previous.as_ref().map(|previous| previous.root),
                previous.as_ref().is_some_and(|previous| previous.pointer == pointer)
            );
        }
        // The root is not compared: a scrollable's dispatcher is rooted at its
        // child, which is replaced whenever the content rebuilds, while the rows
        // inside live on by identity and keep whatever hover they had. A chain
        // from a root that is unrelated simply resolves to nothing below.
        let previous = previous.filter(|previous| previous.pointer == pointer);
        let exit = ElementEvent::PointerExited(pointer.source, pointer.id);
        if aimer_utils::pointer_trace_enabled() {
            match &previous {
                Some(previous) => aimer_utils::pointer_trace!(
                    "hover chain: previous {} elements, current {}",
                    previous.elements.len(),
                    current.elements.len()
                ),
                None => aimer_utils::pointer_trace!(
                    "hover chain: previous chain dropped (tree changed or none), current {} elements",
                    current.elements.len()
                ),
            }
        }
        if let Some(previous) = previous {
            let current_mark = self.mark_hover_membership(&current.elements);
            for previous_element in previous.elements.iter().rev() {
                if self.is_current_hover_element(*previous_element, current_mark) {
                    continue;
                }

                let Some(element) = self.resolve_owner(root, *previous_element) else {
                    continue;
                };
                if event_callback_enabled(element) {
                    aimer_utils::pointer_trace!("exit sent to {}", element.debug_name());
                    outcome.result = outcome.result.merge(element.on_event(&exit));
                }
            }
        }
        self.hover_chains.insert(pointer, current);
        outcome
    }

    /// Marks the current hover chain in reusable path-indexed scratch storage.
    ///
    /// Every element that can appear in a routed hover chain is present in the
    /// structural path index. A mark epoch avoids clearing the whole vector on
    /// every pointer move, and the vector itself retains capacity across moves
    /// and tree generations.
    #[inline]
    fn mark_hover_membership(&mut self, elements: &[ElementId]) -> u32 {
        if self.hover_membership_marks.len() < self.path_links.len() {
            self.hover_membership_marks.resize(self.path_links.len(), 0);
        }

        let next_epoch = self.hover_membership_epoch.wrapping_add(1);
        if next_epoch == 0 {
            self.hover_membership_marks.fill(0);
            self.hover_membership_epoch = 1;
        } else {
            self.hover_membership_epoch = next_epoch;
        }
        let mark = self.hover_membership_epoch;

        for &element in elements {
            let Some(path_index) = self.path_indices.get(&element).copied() else {
                continue;
            };
            if let Some(slot) = self.hover_membership_marks.get_mut(path_index) {
                *slot = mark;
            }
        }
        mark
    }

    #[inline]
    fn is_current_hover_element(&self, element: ElementId, mark: u32) -> bool {
        record_hover_membership_check();
        let Some(path_index) = self.path_indices.get(&element).copied() else {
            return false;
        };
        self.hover_membership_marks.get(path_index).copied() == Some(mark)
    }

    fn dispatch_cached_hit_chain(
        &mut self,
        root: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
        pointer: PointerKey,
    ) -> Option<(RoutedEventResult, HoverHitChain)> {
        let mut cached = self.hit_chain_cache.take()?;
        let Some(first) = cached.elements.first() else {
            return None;
        };
        let generation = root.subtree_generation();
        let valid = cached.pointer == pointer
            && cached.root == root.element_id()?
            && cached.generation == generation
            && cached.generation == self.indexed_subtree_generation
            && self.indexed_root == Some(cached.root)
            && std::ptr::addr_eq(root as *const dyn Element, first.element);
        if !valid {
            return None;
        }

        for entry in &cached.elements {
            // SAFETY: the cache is used only when the root identity and its
            // subtree generation are unchanged. Reconciliation retires the
            // cache before an element can be replaced or moved, and the UI
            // tree is not concurrently mutated during dispatch.
            let element = unsafe { &*entry.element };
            if element.element_id() != Some(entry.id)
                || !contains(element, cached.last_position)
                || !contains(element, pos)
            {
                return None;
            }
        }

        let outcome = dispatch_cached_hit_chain_inner(self, root, &cached.elements, 0, pos, event);
        let hover_chain = HoverHitChain {
            pointer,
            root: cached.root,
            generation,
            elements: cached
                .elements
                .iter()
                .map(|element| element.id)
                .collect(),
        };
        if !outcome.result.is_consumed()
            && matches!(outcome.result.capture_request(), CaptureRequest::None)
            && matches!(outcome.result.follow_up(), FollowUp::None)
        {
            cached.last_position = pos;
            self.hit_chain_cache = Some(cached);
        }
        Some((outcome, hover_chain))
    }

    /// Places the focus of this dispatcher inside `trap`.
    ///
    /// A dispatch root that is presented *over* the application — a modal, whose
    /// content is not part of the tree it covers — belongs to the region its
    /// [`FocusTrap`](crate::focus::FocusTrap) confines. Focus is then granted
    /// here only while that region is the innermost trapping one, so the
    /// application underneath owns nothing while the overlay is up and gets its
    /// owner back when the trap is released.
    #[inline]
    pub fn with_focus_trap(mut self, trap: FocusTrapId) -> Self {
        self.focus.set_trap(Some(trap));
        self
    }

    /// Returns the region this dispatcher grants focus in, if it is confined.
    #[inline]
    pub fn focus_trap(&self) -> Option<FocusTrapId> {
        self.focus.trap()
    }

    /// Makes this dispatcher leave focus to the one enclosing it.
    ///
    /// A forwarding element that routes its child through a dispatcher of its own
    /// (a scrollable does) is still part of the enclosing tree, and keyboard text
    /// is delivered to whoever the *enclosing* dispatcher says owns focus. A
    /// private dispatcher that moved focus itself would set the node's flag and
    /// show the soft keyboard, yet the enclosing dispatcher would never learn of
    /// the owner and typed text would reach no one.
    ///
    /// Once set, this dispatcher neither resolves focus requests nor traverses
    /// nor moves focus on a press. It only remembers the target a press found,
    /// for [`EventDispatchContext::adopt_deferred_focus`] to pass on.
    #[inline]
    pub fn defer_focus_to_parent(&mut self) {
        self.defers_focus = true;
    }

    /// Returns the element that currently owns keyboard focus, if any.
    #[inline]
    pub fn focused(&self) -> Option<ElementId> {
        self.focus.focused()
    }

    /// Returns the element whose subtree keyboard focus is confined to, if any.
    ///
    /// This is the innermost element of the last indexed tree that reported
    /// [`EventElement::traps_focus`].
    #[inline]
    pub fn focus_scope(&self) -> Option<ElementId> {
        self.focus_scope
    }

    /// Returns whether the latest press landed on something that can take focus,
    /// such as a text field.
    ///
    /// A focus target takes a press through the focus system and does not have
    /// to consume it, so [`EventResult::is_consumed`] alone cannot tell a host
    /// that the press was taken. A press on a captured pointer's owner does not
    /// reach this answer; it is taken by definition.
    #[inline]
    pub fn last_press_hit_focus_target(&self) -> bool {
        self.last_press_hit_focus_target
    }

    /// Returns whether focus is owed a decision that no event has made yet: a
    /// node asked for it, a region that wants it on arrival was built, or a
    /// trap changed.
    ///
    /// Reads counters only and never touches the tree, so a frame loop can ask
    /// after every frame; [`Self::settle_focus`] is what answers a `true`.
    #[inline]
    pub fn has_pending_focus_work(&self) -> bool {
        self.focus.has_unsettled_requests()
    }

    /// Resolves focus without an input event, notifying both sides of a change.
    ///
    /// Focus is otherwise decided while an event is dispatched, which leaves a
    /// field that focuses itself on arrival — or a node asked to focus from a
    /// timer — waiting for the user's next touch before it hears of it. A frame
    /// loop calls this once the frame has built the tree, when
    /// [`Self::has_pending_focus_work`] says there is something to decide.
    ///
    /// Unlike a dispatch this brings only the id-to-element index up to date,
    /// not the event tree, which stays lazy until an event needs it. The result
    /// is what the notified elements returned, so a caller can tell whether the
    /// change needs another frame.
    pub fn settle_focus(&mut self, root: &dyn Element) -> EventResult {
        let _settling = SettlingBetweenEvents::enter();
        self.synchronize_path_index_for_current_tree(root);
        self.synchronize_focus(root)
    }

    /// Dispatches one event using persistent capture state.
    ///
    /// Uncaptured events use normal hit testing. A captured move, exit, or up
    /// resolves only the saved root-to-owner path. Pointer-up releases its
    /// capture after delivery, and cancellation is delivered once to every
    /// distinct captured owner before all captures are cleared.
    ///
    /// Focus-directed events — see [`ElementEvent::is_focus_directed`] — skip
    /// hit testing entirely and are offered to the whole tree until an element
    /// that owns keyboard focus consumes them, so text and composition reach the
    /// focused field no matter where the pointer is.
    ///
    /// Delivery is also where a [pointer claim](crate::pointer_claim) expires: a
    /// claim describes a gesture in progress, so it cannot outlive the pointer
    /// that made it. Whatever a descendant forgot to give back is released once
    /// the pointer goes up, and cancellation drops every claim at once — without
    /// which a single missed release would leave an enclosing scrollable
    /// standing down forever.
    pub fn dispatch(
        &mut self,
        root: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
    ) -> EventResult {
        if let Some(pointer) = event.pointer() {
            aimer_events::pointer::note_pointer_source(pointer.source);
        }
        // Hit testing is by position, so an element that must see a scroll before
        // its children can absorb it learns here that this is one.
        aimer_events::pointer::note_routing_scroll(matches!(event, ElementEvent::Scroll { .. }));
        let result = self.route(root, pos, event);
        aimer_events::pointer::note_routing_scroll(false);
        match event {
            ElementEvent::PointerUp(pointer) => {
                pointer_claim::release_pointer(PointerKey::new(pointer.source, pointer.id));
            }
            ElementEvent::PointerExited(source, id) => {
                // The exit was routed like any other event, by position, which
                // reaches only what is under wherever the pointer happens to be
                // reported. What it was hovering is somewhere else, and forgetting
                // that without saying so would leave it lit for good.
                if let Some(chain) = self.hover_chains.remove(&PointerKey::new(*source, *id)) {
                    for element_id in chain.elements.iter().rev() {
                        let Some(element) = self.resolve_owner(root, *element_id) else {
                            continue;
                        };
                        if event_callback_enabled(element) {
                            aimer_utils::pointer_trace!("explicit exit sent to {}", element.debug_name());
                            let _ = element.on_event(event);
                        }
                    }
                }
            }
            ElementEvent::Cancel => {
                pointer_claim::release_all_pointers();
            }
            _ => {}
        }
        result.merge(self.settle_focus_requests(root))
    }

    /// Dispatches a forwarding element's private child view with this
    /// dispatcher's path index and capture state.
    ///
    /// A forwarding boundary remains the externally captured owner, while the
    /// nested owner is retained in [`Self::nested_captures`]. This preserves
    /// hover wrappers' ability to observe an out-of-bounds release without
    /// giving every wrapper its own dispatcher and path map.
    fn dispatch_nested(
        &mut self,
        path_root: &dyn Element,
        boundary: Option<ElementId>,
        root: &dyn Element,
        pos: Vec2d,
        event: &ElementEvent,
    ) -> EventResult {
        let pointer = event_pointer_key(event);
        self.mark_hit_chain_forwarding_boundary();
        let captured_owner = boundary.and_then(|boundary| {
            pointer.and_then(|pointer| {
                self.nested_captures
                    .get(&(boundary, pointer))
                    .copied()
            })
        });
        let was_captured = captured_owner.is_some();

        let outcome = if matches!(event, ElementEvent::Cancel) {
            if let Some(boundary) = boundary {
                self.cancel_nested_captures(path_root, boundary, event)
            } else {
                dispatch_nested_event(self, path_root, boundary, root, pos, event)
            }
        } else if let Some(owner) = captured_owner {
            match self.dispatch_nested_captured(path_root, owner, event) {
                Some(outcome) => outcome,
                None => {
                    if let (Some(boundary), Some(pointer)) = (boundary, pointer) {
                        self.nested_captures.remove(&(boundary, pointer));
                    }
                    RoutedEventResult {
                        result: EventResult::ignored(),
                        capture_owner: None,
                        focus_owner: None,
                    }
                }
            }
        } else {
            dispatch_nested_event(self, path_root, boundary, root, pos, event)
        };

        if let Some(boundary) = boundary {
            match outcome.result.capture_request() {
                CaptureRequest::Capture(pointer) => {
                    if let Some(owner) = outcome.capture_owner {
                        self.nested_captures.insert((boundary, pointer), owner);
                    }
                }
                CaptureRequest::Release(pointer) => {
                    self.nested_captures.remove(&(boundary, pointer));
                }
                CaptureRequest::None => {}
            }
            if was_captured && matches!(event, ElementEvent::PointerUp(_))
                && let Some(pointer) = pointer {
                    self.nested_captures.remove(&(boundary, pointer));
                }
            if matches!(event, ElementEvent::Cancel) {
                self.nested_captures
                    .retain(|(captured_boundary, _), _| *captured_boundary != boundary);
            }
        }

        // The press landed on a focus target inside the forwarding element's
        // child view. The first one found is the topmost, as in an ordinary walk.
        if self.nested_focus_owner.is_none() {
            self.nested_focus_owner = outcome.focus_owner;
        }

        outcome.result.without_capture_request()
    }

    /// Adds the focus target a forwarding element's child view found to `slot`,
    /// unless the enclosing walk already has a deeper one, and clears the
    /// hand-off so it cannot reach a later element.
    #[inline]
    fn merge_nested_focus_owner(&mut self, slot: &mut Option<FocusCandidate<ElementId>>) {
        let nested = self.nested_focus_owner.take();
        if slot.is_none() {
            *slot = nested;
        }
    }

    fn dispatch_nested_captured(
        &mut self,
        path_root: &dyn Element,
        owner: ElementId,
        event: &ElementEvent,
    ) -> Option<RoutedEventResult> {
        let target = self.resolve_indexed_element(path_root, owner)?;
        if target.element_id() != Some(owner) {
            return None;
        }

        let result = {
            if event_callback_enabled(target) {
                // Continue nested capture routing from this owner. Reusing its
                // parent boundary would resolve the same captured owner again
                // when a forwarding element dispatches to its own child.
                let mut context = EventDispatchContext::new(self, path_root, Some(owner), event.get_pointer_pos().unwrap_or_default());
                target.on_event_with_context(event, &mut context)
            } else {
                EventResult::ignored()
            }
        };
        let capture_owner = (!matches!(result.capture_request(), CaptureRequest::None))
            .then_some(owner);
        Some(RoutedEventResult {
            result,
            capture_owner,
            focus_owner: None,
        })
    }

    fn cancel_nested_captures(
        &mut self,
        path_root: &dyn Element,
        boundary: ElementId,
        event: &ElementEvent,
    ) -> RoutedEventResult {
        let owners: HashSet<ElementId> = self
            .nested_captures
            .iter()
            .filter_map(|((captured_boundary, _), owner)| {
                (*captured_boundary == boundary).then_some(*owner)
            })
            .collect();
        let mut result = EventResult::ignored();
        for owner in owners {
            if let Some(outcome) = self.dispatch_nested_captured(path_root, owner, event) {
                result = result.merge(outcome.result);
            }
        }
        self.nested_captures
            .retain(|(captured_boundary, _), _| *captured_boundary != boundary);
        RoutedEventResult {
            result,
            capture_owner: None,
            focus_owner: None,
        }
    }

    /// Grants the focus a handler asked for while this event was delivered.
    ///
    /// Focus is resolved once, before the event is routed, so a handler that
    /// calls [`FocusNode::request_focus`] — a button focusing the field it
    /// belongs to — records its wish after the only pass that would have read
    /// it. Left there, the wish would wait for whatever input happens next: a
    /// mouse hides that behind the pixel it moves after a click, but a finger
    /// that taps and lifts sends nothing more, so the field would be focused by
    /// the *next* tap.
    ///
    /// Nothing is walked for an event that asked for nothing:
    /// [`FocusManager::begin_synchronization`] compares the request counter it
    /// recorded moments ago, so the common case is a handful of comparisons.
    #[inline]
    fn settle_focus_requests(&mut self, root: &dyn Element) -> EventResult {
        self.synchronize_paths(root);
        self.synchronize_focus(root)
    }

    /// Routes one event, leaving pointer-claim housekeeping to
    /// [`Self::dispatch`].
    fn route(&mut self, root: &dyn Element, pos: Vec2d, event: &ElementEvent) -> EventResult {
        self.nested_focus_owner = None;
        self.deferred_focus_owner = None;
        let pointer = event_pointer_key(event);
        let routes_to_capture = matches!(
            event,
            ElementEvent::PointerMove(_)
                | ElementEvent::PointerUp(_)
                | ElementEvent::PointerExited(_, _)
        );
        let was_captured = routes_to_capture
            && pointer.is_some_and(|pointer| self.captures.contains_key(&pointer));
        let uncaptured_pointer_move = matches!(event, ElementEvent::PointerMove(_))
            && !was_captured
            && pointer.is_some();
        if !uncaptured_pointer_move {
            self.invalidate_hit_chain();
        }

        self.synchronize_paths(root);
        let focus_result = self.synchronize_focus(root);

        if let ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed | KeyAction::Repeat,
            modifiers,
        } = event
            && let Some(traversal_result) = self.traverse_focus(root, modifiers.shift)
        {
            return focus_result
                .merge(traversal_result)
                .merge(EventResult::consumed());
        }

        if matches!(event, ElementEvent::Cancel) {
            return focus_result.merge(
                self.cancel_captures(root, event)
                    .without_capture_request(),
            );
        }

        if event.is_focus_directed() {
            let focused_result = self.dispatch_to_focused(root, event);
            if focused_result.is_consumed() || !self.focus.is_suspended() {
                return focus_result.merge(focused_result);
            }

            // Focus is trapped elsewhere, so this tree owns none of it. The
            // trapping region is presented by an element of this tree — an
            // overlay host dispatching into its own root — so the event is
            // routed to reach it, which is the only way typed text arrives at
            // the field inside a modal.
            let outcome = dispatch_routed_event(self, root, pos, event);
            return focus_result
                .merge(focused_result)
                .merge(outcome.result.without_capture_request().without_follow_up());
        }

        if matches!(event, ElementEvent::KeyInput { .. }) {
            let focused_result = self.dispatch_to_focused(root, event);
            if focused_result.is_consumed() {
                return focus_result.merge(focused_result);
            }

            let outcome = dispatch_routed_event(self, root, pos, event);
            return focus_result
                .merge(focused_result)
                .merge(outcome.result.without_capture_request().without_follow_up());
        }

        if routes_to_capture
            && was_captured
            && let Some(pointer) = pointer
        {
            let result = self.dispatch_captured(root, pointer, event);
            return focus_result.merge(self.run_follow_up(root, pos, pointer, result));
        }

        let outcome = if let Some(pointer) = pointer.filter(|_| uncaptured_pointer_move) {
            self.dispatch_uncaptured_pointer_move(root, pos, event, pointer)
        } else {
            dispatch_routed_event(self, root, pos, event)
        };
        self.apply_capture_request(outcome.result.capture_request(), outcome.capture_owner);
        if matches!(event, ElementEvent::PointerDown(_)) {
            self.last_press_hit_focus_target = outcome.focus_owner.is_some();
        }
        if self.defers_focus && matches!(event, ElementEvent::PointerDown(_)) {
            self.deferred_focus_owner = outcome.focus_owner.clone();
        }
        let pointer_focus_result = if matches!(event, ElementEvent::PointerDown(_))
            && !self.defers_focus
            && self.press_may_move_focus(root, outcome.focus_owner.as_ref())
        {
            self.transition_focus(root, outcome.focus_owner.clone())
        } else {
            EventResult::ignored()
        };
        let result = outcome.result.without_capture_request();
        match pointer {
            Some(pointer) => {
                focus_result
                    .merge(pointer_focus_result)
                    .merge(self.run_follow_up(root, pos, pointer, result))
            }
            None => focus_result
                .merge(pointer_focus_result)
                .merge(result.without_follow_up()),
        }
    }

    /// Runs the extra routed pass a handler asked for, if it asked for one.
    ///
    /// This is the whole of drag routing: the element carrying the drag owns the
    /// pointer and therefore hears about it alone, so it asks for one more
    /// ordinary hit-tested dispatch at the position already in hand, and the
    /// topmost element under the pointer receives the drag event. Nothing
    /// happens — no traversal, no allocation — unless a handler asked, so an
    /// application that never drags pays only the cost of reading one field.
    ///
    /// A [`FollowUp::DragDrop`] ends the drag, so the capture that asked for it
    /// is released once the drop has been delivered.
    fn run_follow_up(
        &mut self,
        root: &dyn Element,
        pos: Vec2d,
        pointer: PointerKey,
        result: EventResult,
    ) -> EventResult {
        let follow_up = result.follow_up();
        let follow_up_event = match follow_up {
            FollowUp::None => return result,
            FollowUp::DragOver => ElementEvent::DragOver {
                pos,
                source: pointer.source,
                id: pointer.id,
            },
            FollowUp::DragDrop => ElementEvent::DragDrop {
                pos,
                source: pointer.source,
                id: pointer.id,
            },
        };

        let outcome = dispatch_routed_event(self, root, pos, &follow_up_event);
        if matches!(follow_up, FollowUp::DragDrop) {
            self.captures.remove(&pointer);
        }

        result
            .merge(outcome.result)
            .without_capture_request()
            .without_follow_up()
    }

    /// Returns the element currently owning `pointer`, if any.
    #[inline]
    pub fn captured_owner(&self, pointer: PointerKey) -> Option<ElementId> {
        self.captures.get(&pointer).copied()
    }

    /// Returns the number of independently captured pointers.
    #[inline]
    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    /// Returns whether any pointer is captured, by an element of this tree or by
    /// one inside a forwarding boundary's private view.
    ///
    /// [`capture_count`](Self::capture_count) sees only the former, which is not
    /// enough to know that a drag is in progress: a press on an element behind a
    /// stateful or animated wrapper is recorded in the nested table.
    #[inline]
    pub fn has_capture(&self) -> bool {
        !self.captures.is_empty() || !self.nested_captures.is_empty()
    }

    /// Returns whether `pointer` currently has a live capture entry.
    #[inline]
    pub fn is_captured(&self, pointer: PointerKey) -> bool {
        self.captures.contains_key(&pointer)
    }

    /// Returns whether `pointer` is hovering something in the tree last
    /// dispatched to, and so is owed an exit when it leaves.
    ///
    /// A container that stops offering moves to its own dispatcher — because
    /// the pointer left the part of it that shows content — uses this to tell
    /// whether the dispatcher still holds a hover it must release.
    #[inline]
    pub fn is_hovering(&self, pointer: PointerKey) -> bool {
        self.hover_chains.contains_key(&pointer)
    }

    /// Clears all capture entries without delivering cancellation.
    ///
    /// Use this only after the owning boundary has already broadcast a single
    /// cancellation event to its subtree.
    #[inline]
    pub fn clear_captures(&mut self) {
        self.captures.clear();
        self.nested_captures.clear();
    }

    /// Delivers cancellation to the owner of `pointer` and releases it.
    ///
    /// This is primarily used by nested routing boundaries when an ancestor
    /// wins gesture arbitration after a descendant initially captured.
    pub fn cancel_pointer(&mut self, root: &dyn Element, pointer: PointerKey) -> EventResult {
        self.synchronize_paths(root);
        let mut result = EventResult::ignored();

        // The pointer's owner, when a descendant asked this dispatcher to
        // capture it. A forwarding boundary below it relays the cancellation to
        // whatever it holds in its own child view.
        if let Some(owner) = self.captures.remove(&pointer)
            && let Some(target) = self.resolve_indexed_element(root, owner)
            && target.element_id() == Some(owner)
            && event_callback_enabled(target)
        {
            let mut context = EventDispatchContext::new(self, root, Some(owner), Vec2d::default());
            result = target
                .on_event_with_context(&ElementEvent::Cancel, &mut context)
                .without_capture_request();
        }

        result.merge(self.cancel_nested_pointer(root, pointer, None))
    }

    /// Cancels `pointer` for every element that captured it inside a forwarding
    /// element's child view, and forgets those captures.
    ///
    /// With `within`, only captures made inside the child view of that boundary
    /// are touched: the boundary may itself be nested in another forwarding
    /// element, whose record of *this* boundary holding the pointer is not
    /// something the boundary cancels in its own child view.
    fn cancel_nested_pointer(
        &mut self,
        root: &dyn Element,
        pointer: PointerKey,
        within: Option<ElementId>,
    ) -> EventResult {
        let mut result = EventResult::ignored();
        // A capture made inside a forwarding element's child view is recorded
        // only in the nested table, and the dispatcher that owns that view has no
        // capture of its own to look up. Without this a descendant that took the
        // pointer, such as a button inside a list, never learned that a scroll
        // took it back, and stayed in the state the press had put it in.
        let entries: Vec<(ElementId, ElementId)> = self
            .nested_captures
            .iter()
            .filter_map(|((boundary, captured), owner)| (*captured == pointer).then_some((*boundary, *owner)))
            .collect();
        let mut owners: HashSet<ElementId> = HashSet::new();
        let mut cancelled: Vec<ElementId> = Vec::new();
        for (boundary, owner) in entries {
            if let Some(within) = within
                && !self.is_ancestor_or_self(root, within, boundary)
            {
                continue;
            }
            cancelled.push(boundary);
            owners.insert(owner);
        }
        for owner in owners {
            if let Some(outcome) = self.dispatch_nested_captured(root, owner, &ElementEvent::Cancel) {
                result = result.merge(outcome.result.without_capture_request());
            }
        }
        self.nested_captures
            .retain(|(boundary, captured), _| *captured != pointer || !cancelled.contains(boundary));
        result
    }

    /// Finds the element behind each queued invalidation, in record order.
    ///
    /// Most come straight from the path index. The ones it does not know, which
    /// are elements added since it was built and elements that have gone, are
    /// found together by one walk of the tree. Walking it once for each of them
    /// made a change that touches every element, a theme switch for instance,
    /// cost the square of the size of the tree.
    fn resolve_invalidation_owners<'a>(
        &mut self,
        root: &'a dyn Element,
        batch: &ElementInvalidationBatch,
    ) -> Vec<Option<&'a dyn Element>> {
        let records = batch.records();
        let mut resolved = Vec::with_capacity(records.len());
        let mut missing = HashSet::new();
        for invalidation in records {
            let Some(id) = invalidation.element_id.filter(|_| !invalidation.stale_id) else {
                resolved.push(None);
                continue;
            };
            crate::frame_work_stats::record_element_index_lookup();
            let element = self.resolve_indexed_element(root, id);
            if element.is_none() {
                missing.insert(id);
            }
            resolved.push(element);
        }
        if !missing.is_empty() {
            let found = find_structural_elements(root, &missing);
            for (invalidation, slot) in records.iter().zip(resolved.iter_mut()) {
                if slot.is_none()
                    && let Some(id) = invalidation.element_id
                {
                    *slot = found.get(&id).copied();
                }
            }
        }
        resolved
    }

    /// Drains pending element mutations and resolves their pre-frame owners
    /// through the generation-checked structural index.
    #[doc(hidden)]
    pub fn take_pending_element_invalidations(
        &mut self,
        root: &dyn Element,
    ) -> ElementInvalidationBatch {
        self.synchronize_path_index_for_current_tree(root);
        let mut batch = crate::element_invalidation::take_pending();
        if batch.records().is_empty() {
            return batch;
        }

        let before_frame = current_element_invalidation_revisions();
        let resolved = self.resolve_invalidation_owners(root, &batch);
        for (invalidation, element) in batch.records_mut().iter_mut().zip(resolved) {
            invalidation.before_frame_revisions = Some(before_frame);
            if invalidation.queued_during_frame && invalidation.damage_hint.is_none() {
                // A draw-time mutation without a tracked damage footprint has
                // no reliable pre-walk bounds.
                invalidation.requires_full_fallback = true;
            }

            if invalidation.element_id.is_none() {
                invalidation.requires_full_fallback = true;
                continue;
            }
            match element {
                Some(element) => {
                    invalidation.old_bounds = element_invalidation_bounds(element);
                    if invalidation.old_bounds.is_none() && invalidation.damage_hint.is_none() {
                        invalidation.requires_full_fallback = true;
                    }
                }
                None => {
                    invalidation.stale_id = true;
                    invalidation.requires_full_fallback = true;
                    crate::frame_work_stats::record_stale_element_id();
                }
            }
        }
        batch
    }

    /// Resolves post-rebuild bounds for invalidations captured before the frame
    /// walk. Removed IDs retain their old footprint and have no new footprint.
    #[doc(hidden)]
    pub fn complete_pending_element_invalidations(
        &mut self,
        root: &dyn Element,
        batch: &mut ElementInvalidationBatch,
    ) {
        if batch.records().is_empty() {
            return;
        }

        self.synchronize_path_index_for_current_tree(root);
        let after_frame = current_element_invalidation_revisions();
        let resolved = self.resolve_invalidation_owners(root, batch);
        for (invalidation, element) in batch.records_mut().iter_mut().zip(resolved) {
            invalidation.after_frame_revisions = Some(after_frame);
            if invalidation.stale_id {
                continue;
            }
            if invalidation.element_id.is_none() {
                continue;
            }
            match element {
                Some(element) => {
                    invalidation.new_bounds = element_invalidation_bounds(element);
                    if invalidation.new_bounds.is_none() && invalidation.damage_hint.is_none() {
                        invalidation.requires_full_fallback = true;
                    }
                }
                None if invalidation.old_bounds.is_some() => {
                    invalidation.removed = true;
                    // Dirty-region consumption is not enabled yet, so removal
                    // stays on the full-frame path even with a known old box.
                    invalidation.requires_full_fallback = true;
                }
                None => {
                    invalidation.stale_id = true;
                    invalidation.requires_full_fallback = true;
                    crate::frame_work_stats::record_stale_element_id();
                }
            }

            if invalidation.requires_full_fallback
                && !invalidation.stale_id
                && !invalidation.removed
                && invalidation.affected_path.as_deref().is_some_and(|path| {
                    FRAME_PAINT_DAMAGE_OWNERS.with(|owners| {
                        let owners = owners.borrow();
                        path.iter().any(|id| owners.contains(id))
                    })
                })
            {
                // A bounded stateful paint owner reported its old/new target
                // footprint while drawing. Its rectangle is precise even when
                // an event wrapper in this path (such as MouseRegion) has no
                // stable event-tree bounds of its own.
                invalidation.requires_full_fallback = false;
            }
        }
    }

    fn synchronize_paths_for_current_tree(&mut self, root: &dyn Element) {
        self.synchronize_path_index_for_current_tree(root);
        self.synchronize_event_tree(root);
    }

    /// Like [`Self::synchronize_paths_for_current_tree`], for a caller that
    /// only resolves elements by identity.
    fn synchronize_path_index_for_current_tree(&mut self, root: &dyn Element) {
        if self.indexed_subtree_generation != root.subtree_generation()
            || self.indexed_root != root.element_id()
        {
            self.paths_dirty = true;
        }
        self.synchronize_path_index(root);
    }

    /// Finds the retained element `id` below `root`, bringing the path index up
    /// to date first.
    ///
    /// A lookup is constant time once the index matches `root`; it fails, and
    /// returns `None`, for an element that is not part of the structural tree.
    #[doc(hidden)]
    pub fn resolve_element<'a>(
        &mut self,
        root: &'a dyn Element,
        id: ElementId,
    ) -> Option<&'a dyn Element> {
        // The tree may have been rebuilt since this frame's first look at it, so
        // the subtree generation is compared again rather than trusted.
        self.synchronize_path_index_for_current_tree(root);
        self.resolve_indexed_element(root, id)
    }

    fn resolve_indexed_element<'a>(
        &self,
        root: &'a dyn Element,
        id: ElementId,
    ) -> Option<&'a dyn Element> {
        if self.indexed_subtree_generation != root.subtree_generation()
            || self.indexed_root != root.element_id()
            || self.indexed_root_address != Some(root as *const dyn Element as *const ())
            // A row dropped since the index was built is a dangling pointer.
            || self.indexed_live_children != live_children_generation()
        {
            return None;
        }

        let index = *self.path_indices.get(&id)?;
        let element = self.indexed_elements.get(index)?.element;
        // SAFETY: `synchronize_paths` rebuilds `indexed_elements` from this
        // exact root address and subtree generation. Reconciliation retires
        // the index before its elements can move or be replaced, and event
        // dispatch is synchronous on the UI thread without concurrent tree
        // mutation.
        let element: &'a dyn Element = unsafe { &*element };
        (element.element_id() == Some(id)).then_some(element)
    }

    /// Brings the id-to-element index and the event tree up to date.
    ///
    /// Event dispatch needs both. A caller that only has to find an element by
    /// identity uses [`Self::synchronize_path_index`] and leaves the event tree
    /// to the first event that needs it.
    fn synchronize_paths(&mut self, root: &dyn Element) {
        self.synchronize_path_index(root);
        self.synchronize_event_tree(root);
    }

    /// Brings the id-to-element index up to date.
    ///
    /// This is all that resolving an element by identity needs. It marks the
    /// event tree stale whenever it rebuilds, so the tree, which costs as much
    /// as the index again, is built only for an event that is dispatched.
    fn synchronize_path_index(&mut self, root: &dyn Element) {
        let root_id = root.element_id();
        let root_address = root as *const dyn Element as *const ();
        let root_address_changed = self.indexed_root_address != Some(root_address);
        let generation = match current_event_frame() {
            Some(frame) => {
                if self.generation_checked_frame != Some(frame) {
                    self.generation_checked_frame = Some(frame);
                    let generation = root.subtree_generation();
                    self.paths_dirty = self.indexed_subtree_generation != generation
                        || self.indexed_root != root_id;
                    Some(generation)
                } else if self.indexed_root != root_id {
                    self.paths_dirty = true;
                    Some(root.subtree_generation())
                } else {
                    None
                }
            }
            None => {
                let generation = root.subtree_generation();
                self.paths_dirty = self.indexed_subtree_generation != generation
                    || self.indexed_root != root_id;
                Some(generation)
            }
        };

        // Read on every call rather than once per event frame: windows slide
        // during layout, after this epoch's first look at the tree.
        let live_children = live_children_generation();
        let live_children_changed = self.indexed_live_children != live_children;
        if root_address_changed || live_children_changed {
            self.paths_dirty = true;
        }
        if !self.paths_dirty {
            return;
        }

        let generation = generation.unwrap_or_else(|| root.subtree_generation());

        // The cached hit chain holds element addresses and goes. The hover chains
        // hold identities, are resolved against the new tree when the pointer next
        // moves, and are what tells an element the pointer has left it, so a tree
        // change must not take them: an element that is not offered the move that
        // leaves it would stay hovered.
        self.invalidate_hit_chain();
        // A patch only refreshes pointers of same-shaped replaced subtrees; a
        // changed child set needs the full walk.
        if !root_address_changed
            && !live_children_changed
            && !self.index_patching_disabled
            && self.patch_path_index(root)
        {
            self.index_work.1 += 1;
            self.indexed_subtree_generation = generation;
            self.paths_dirty = false;
            self.event_tree_stale = true;
            return;
        }
        let mut next_path_indices = HashMap::with_capacity(self.path_indices.len());
        let mut next_path_links = Vec::with_capacity(self.path_links.len());
        let mut next_indexed_elements = Vec::with_capacity(self.indexed_elements.len());
        let mut next_focus_scope = None;
        index_element_links(
            root,
            None,
            0,
            &mut next_path_links,
            &mut next_path_indices,
            &mut next_indexed_elements,
            &mut next_focus_scope,
        );
        // Commit the new ID/path index only after the full structural walk
        // succeeds. A partial walk never becomes a usable lookup table.
        self.path_indices = next_path_indices;
        self.path_links = next_path_links;
        self.indexed_elements = next_indexed_elements;
        self.focus_scope = next_focus_scope;
        self.captures
            .retain(|_, owner| self.path_indices.contains_key(owner));
        let path_indices = &self.path_indices;
        self.nested_captures.retain(|(boundary, _), owner| {
            path_indices.contains_key(boundary) && path_indices.contains_key(owner)
        });
        self.focus
            .retain_history(|owner| path_indices.contains_key(owner));
        self.indexed_subtree_generation = generation;
        self.indexed_root = root_id;
        self.index_work.0 += 1;
        self.indexed_root_address = Some(root_address);
        self.indexed_unscoped = unscoped_element_tree_generation();
        self.indexed_tree_generation = element_tree_generation();
        self.indexed_live_children = live_children;
        self.scoped_cursor = scoped_rebuild_cursor();
        self.paths_dirty = false;
        self.event_tree_stale = true;
    }

    /// Brings the event tree up to date with the path index and the layout.
    ///
    /// Requires the path index to match `root`, which
    /// [`Self::synchronize_path_index`] provides.
    fn synchronize_event_tree(&mut self, root: &dyn Element) {
        let layout_generation = layout_invalidation_generation();
        let event_layout_changed = self.indexed_layout_generation != layout_generation;
        if !self.event_tree_stale && !event_layout_changed {
            return;
        }
        if event_layout_changed {
            self.invalidate_hit_chain();
        }

        if self.event_tree_stale {
            self.event_tree = EventTree::new();
            self.event_target_by_element.clear();
            build_indexed_event_tree(
                root,
                None,
                None,
                &mut self.event_tree,
                &mut self.event_target_by_element,
                &self.path_indices,
            );
            self.event_tree_stale = false;
        } else if !self.event_tree.elements().is_empty() {
            for (element_id, target) in &self.event_target_by_element {
                let bounds = self
                    .resolve_indexed_element(root, *element_id)
                    .and_then(bounds_from_element);
                self.event_tree.update_bounds(*target, bounds);
            }
        }
        self.indexed_layout_generation = layout_generation;
    }

    /// Resolves the focus owner for this frame, notifying both sides of a
    /// change.
    ///
    /// The decision itself belongs to [`FocusManager`]: this method only walks
    /// the tree for candidates and turns the reported transition into element
    /// events. A frame in which neither the tree nor any node asked for a change
    /// stops at the manager's gate, so no traversal happens at all.
    fn synchronize_focus(&mut self, root: &dyn Element) -> EventResult {
        if self.defers_focus {
            return EventResult::ignored();
        }
        let Some(request_generation) = self
            .focus
            .begin_synchronization(self.indexed_subtree_generation, self.indexed_root)
        else {
            return EventResult::ignored();
        };

        self.focus.set_scope(self.focus_scope);
        self.collect_candidates(root);
        let target = self.focus.resolve(&self.focus_candidates);

        let result = self.transition_focus(root, target);
        self.focus.mark_synchronized(
            self.indexed_subtree_generation,
            self.indexed_root,
            request_generation,
        );
        result
    }

    /// Gathers the focusable targets focus may be given to this frame.
    ///
    /// A trapping scope confines focus by omission: the walk starts at the scope
    /// instead of the root, so every target outside it is simply never offered —
    /// neither to [`FocusManager::resolve`] nor to traversal. A scope whose
    /// element can no longer be resolved has left the tree, and the whole tree is
    /// offered again.
    fn collect_candidates(&mut self, root: &dyn Element) {
        let scope = self
            .focus_scope
            .and_then(|scope| self.resolve_owner(root, scope))
            .unwrap_or(root);
        self.focus_candidates.clear();
        collect_focus_candidates(scope, &mut self.focus_candidates);
    }

    /// Returns whether a press is allowed to change who owns focus.
    ///
    /// A press reaches whatever is under it, including the tree behind an inline
    /// dialog, so it is the one way focus could leave a trapping scope without
    /// ever being offered by [`Self::collect_candidates`]. While a scope traps,
    /// a press therefore only moves focus when it landed on a target the scope
    /// contains: a press anywhere else — the dialog's own chrome as much as the
    /// application behind it — leaves the scope's owner alone rather than
    /// blurring it, which is what a press on a dialog's title bar should do.
    ///
    /// Without a scope every press decides focus, as it always has, and the
    /// check is one comparison against `None`.
    fn press_may_move_focus(
        &mut self,
        root: &dyn Element,
        target: Option<&FocusCandidate<ElementId>>,
    ) -> bool {
        if self.focus_scope.is_none() {
            return true;
        }
        let Some(target) = target else {
            return false;
        };
        self.collect_candidates(root);
        self.focus_candidates
            .iter()
            .any(|candidate| candidate.is_attached_to(target.id, &target.node))
    }

    /// Hands focus to `target` and delivers the resulting notifications.
    ///
    /// The losing element only hears about it while the node it reported is
    /// still the one attached to its identity; a node that left the tree with
    /// its element has nothing to notify.
    fn transition_focus(
        &mut self,
        root: &dyn Element,
        target: Option<FocusCandidate<ElementId>>,
    ) -> EventResult {
        let transition = self.focus.transition(target);
        let mut result = EventResult::ignored();

        if let Some(lost) = transition.lost
            && let Some(element) = self.resolve_owner(root, lost.id)
            && element
                .focus_node()
                .is_some_and(|node| node.ptr_eq(&lost.node))
        {
            if event_callback_enabled(element) {
                result = result.merge(element.on_event(&ElementEvent::FocusLost));
            }
        }
        if let Some(gained) = transition.gained
            && let Some(element) = self.resolve_owner(root, gained.id)
        {
            if event_callback_enabled(element) {
                result = result.merge(element.on_event(&ElementEvent::FocusGained));
            }
        }
        result
    }

    fn traverse_focus(&mut self, root: &dyn Element, reverse: bool) -> Option<EventResult> {
        if self.defers_focus {
            return None;
        }
        self.collect_candidates(root);
        let target = self.focus.traverse(&self.focus_candidates, reverse)?;
        Some(self.transition_focus(root, Some(target)))
    }

    fn dispatch_to_focused(
        &self,
        root: &dyn Element,
        event: &ElementEvent,
    ) -> EventResult {
        let Some(focused) = self.focus.owner() else {
            return EventResult::ignored();
        };
        let Some(target) = self.resolve_owner(root, focused.id) else {
            return EventResult::ignored();
        };
        if !target
            .focus_node()
            .is_some_and(|node| node.ptr_eq(&focused.node))
        {
            return EventResult::ignored();
        }
        if event_callback_enabled(target) {
            target.on_event(event).without_capture_request()
        } else {
            EventResult::ignored()
        }
    }

    fn resolve_owner<'a>(
        &self,
        root: &'a dyn Element,
        owner: ElementId,
    ) -> Option<&'a dyn Element> {
        self.resolve_indexed_element(root, owner)
    }

    fn dispatch_captured(
        &mut self,
        root: &dyn Element,
        pointer: PointerKey,
        event: &ElementEvent,
    ) -> EventResult {
        let Some(owner) = self.captures.get(&pointer).copied() else {
            return EventResult::ignored();
        };
        let Some(target) = self.resolve_indexed_element(root, owner) else {
            self.captures.remove(&pointer);
            return EventResult::ignored();
        };
        if target.element_id() != Some(owner) {
            self.captures.remove(&pointer);
            return EventResult::ignored();
        }

        let result = if event_callback_enabled(target) {
            let mut context = EventDispatchContext::new(self, root, Some(owner), event.get_pointer_pos().unwrap_or_default());
            target.on_event_with_context(event, &mut context)
        } else {
            EventResult::ignored()
        };
        self.apply_capture_request(result.capture_request(), Some(owner));
        if matches!(event, ElementEvent::PointerUp(_)) {
            self.captures.remove(&pointer);
        }
        result.without_capture_request()
    }

    fn cancel_captures(&mut self, root: &dyn Element, event: &ElementEvent) -> EventResult {
        let owners: HashSet<ElementId> = self.captures.values().copied().collect();
        let mut result = EventResult::ignored();
        for owner in owners {
            let Some(target) = self.resolve_indexed_element(root, owner) else {
                continue;
            };
            if target.element_id() == Some(owner) {
                if event_callback_enabled(target) {
                    let mut context = EventDispatchContext::new(self, root, Some(owner), event.get_pointer_pos().unwrap_or_default());
                    result = result.merge(target.on_event_with_context(event, &mut context));
                }
            }
        }
        self.captures.clear();
        result
    }

    fn apply_capture_request(&mut self, request: CaptureRequest, owner: Option<ElementId>) {
        match request {
            CaptureRequest::None => {}
            CaptureRequest::Capture(pointer) => {
                if let Some(owner) = owner {
                    self.captures.insert(pointer, owner);
                }
            }
            CaptureRequest::Release(pointer) => {
                self.captures.remove(&pointer);
            }
        }
    }
}

/// Resolves a newly exposed virtualized child when the event path index still
/// describes the previous live window. This scan is only used to bound queued
/// invalidations whose IDs have not entered that index yet.
/// Finds every element of `wanted` in one walk of the tree, stopping as soon as
/// all of them are found.
fn find_structural_elements<'a>(
    root: &'a dyn Element,
    wanted: &HashSet<ElementId>,
) -> HashMap<ElementId, &'a dyn Element> {
    note_structural_search();
    let mut found = HashMap::with_capacity(wanted.len());
    let mut pending = vec![root];
    while let Some(element) = pending.pop() {
        if let Some(id) = element.element_id()
            && wanted.contains(&id)
        {
            found.insert(id, element);
            if found.len() == wanted.len() {
                break;
            }
        }
        element.structural_children(&mut |child| pending.push(child));
    }
    found
}

/// Reports whether the focus change being delivered was decided between input
/// events — by [`EventDispatcher::settle_focus`] at the end of a frame — rather
/// than by an event the user is making.
///
/// An element told [`ElementEvent::FocusGained`] can ask which it was.
#[inline]
pub fn focus_is_settling_between_events() -> bool {
    SETTLING_BETWEEN_EVENTS.with(Cell::get)
}

thread_local! {
    static SETTLING_BETWEEN_EVENTS: Cell<bool> = const { Cell::new(false) };
}

/// Marks [`EventDispatcher::settle_focus`] as running for as long as it lives.
///
/// A guard rather than a pair of calls so that a panic in a notified element
/// cannot leave the next, user-made, focus change looking settled.
struct SettlingBetweenEvents;

impl SettlingBetweenEvents {
    fn enter() -> Self {
        SETTLING_BETWEEN_EVENTS.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for SettlingBetweenEvents {
    fn drop(&mut self) {
        SETTLING_BETWEEN_EVENTS.with(|flag| flag.set(false));
    }
}

mod routing;

use routing::{
    build_indexed_event_tree, collect_focus_candidates, contains, dispatch_cached_hit_chain_inner,
    dispatch_nested_event, dispatch_routed_event, event_pointer_key,
    is_indexed_hit_test_boundary, index_element_links, RoutedEventResult,
};
pub use routing::{broadcast_event, dispatch_event, dispatch_focused_event};
#[cfg(test)]
use routing::{broadcast_event_inner, dispatch_event_inner};

#[cfg(test)]
mod tests;
