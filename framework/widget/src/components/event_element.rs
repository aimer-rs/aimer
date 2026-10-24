use std::collections::HashSet;
use crate::Element;
use crate::components::element::{EventDispatchContext, VisitorElement};
use crate::focus::FocusNode;
use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use aimer_widget::CALLED;

#[derive(Clone, Copy)]
struct StructuralChildPointer<'a>(*const (dyn Element + 'a));

impl PartialEq for StructuralChildPointer<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for StructuralChildPointer<'_> {}

impl Hash for StructuralChildPointer<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0 as *const ()).hash(state);
    }
}

/// Identifies one pointer independently of pointers from other input sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PointerKey {
    /// The device class that produced the pointer.
    pub source: PointerSource, // enum Mouse, Touch
    /// The source-local pointer identifier.
    pub id: u64,
}

/// Describes how an element participates in indexed event delivery.
///
/// Custom elements default to [`IndexedHitTestBoundary`](Self::IndexedHitTestBoundary),
/// which keeps their callback and custom child hit-test behavior in the index.
/// Use [`Transparent`](Self::Transparent) for wrappers that do not handle
/// events, or [`IndexedTarget`](Self::IndexedTarget) when event-child order is
/// also the correct positional hit-test order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventTreeRole {
    /// Compatibility alias for [`IndexedHitTestBoundary`](Self::IndexedHitTestBoundary).
    /// New elements should choose an explicit indexed role.
    Legacy,
    /// This element is an event target and may participate in indexed routing.
    ///
    /// It still receives its ordinary callback. Use this role only when its
    /// `event_children` order is also a correct positional hit-test order and
    /// it has no custom child hit-test boundary.
    IndexedTarget,
    /// This event target applies its custom child hit-test view during indexed
    /// positional routing.
    ///
    /// The indexed tree retains every `event_children` branch for focus and
    /// broadcast delivery, then intersects positional candidates with the
    /// children returned by `hit_test_children_at`. That hit-test view must be
    /// a subset of `event_children` in the same paint order. The element still
    /// receives its ordinary callback.
    #[default]
    IndexedHitTestBoundary,
    /// This element does not handle events; descendants may be routed through
    /// it through the index. It must not impose a custom hit-test boundary.
    Transparent,
}

impl Hash for PointerKey {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        // pack tag + id into a single u64 write instead of two separate
        // hasher calls — cheaper than write_u8 + write_u64 for most hashers,
        // since a lot of the cost is per-call, not per-byte, for small inputs.
        state.write_u64((self.id << 1) | self.source as u64);
    }
}

impl PointerKey {
    /// Creates a pointer key from its source and source-local identifier.
    #[inline]
    pub const fn new(source: PointerSource, id: u64) -> Self {
        Self { source, id }
    }
}

/// Requests a change to persistent pointer ownership after event handling.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CaptureRequest {
    /// Leaves pointer ownership unchanged.
    #[default]
    None,
    /// Routes subsequent events for this pointer to the requesting element.
    Capture(PointerKey),
    /// Releases this pointer if it is currently captured.
    Release(PointerKey),
}

/// Asks the dispatcher for one extra routed pass after the current one.
///
/// An element that holds a pointer capture is, by construction, the only
/// element that hears about the pointer — which is exactly wrong for a drag,
/// where the widget being carried owns the pointer but the widget being dropped
/// *onto* is the one that needs to know. Rather than give drag-and-drop its own
/// hit-tester, the capturing element asks for a second pass of the ordinary
/// routed dispatch, and the topmost element under the pointer receives a
/// [`ElementEvent::DragOver`] or [`ElementEvent::DragDrop`].
///
/// The request deliberately carries no position. [`EventResult`] is `Copy + Eq`
/// and a position is neither, and the dispatcher already holds the position it
/// would have carried.
///
/// # Examples
///
/// ```
/// use aimer_widget::{EventResult, FollowUp};
///
/// let moving = EventResult::consumed().with_follow_up(FollowUp::DragOver);
///
/// assert_eq!(moving.follow_up(), FollowUp::DragOver);
/// assert_eq!(EventResult::consumed().follow_up(), FollowUp::None);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FollowUp {
    /// No extra pass. This is what every handler that knows nothing about
    /// dragging reports, and it costs nothing.
    #[default]
    None,
    /// Route one [`ElementEvent::DragOver`] at the dispatch position.
    DragOver,
    /// Route one [`ElementEvent::DragDrop`] at the dispatch position, then
    /// release the capture that asked for it.
    DragDrop,
}

/// The independent effects produced by an element event handler.
///
/// Consumption controls event propagation, while redraw indicates that visual
/// state changed. Keeping these effects independent allows a handler to report
/// either or both without allocating or borrowing the incoming event.
#[must_use = "event results may contain propagation or redraw requests"]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventResult {
    consumed: bool,
    needs_redraw: bool,
    capture_request: CaptureRequest,
    follow_up: FollowUp,
}

impl EventResult {
    /// Creates a result that leaves propagation and application state unchanged.
    #[inline]
    pub const fn ignored() -> Self {
        Self {
            consumed: false,
            needs_redraw: false,
            capture_request: CaptureRequest::None,
            follow_up: FollowUp::None,
        }
    }

    /// Creates a result that stops normal event propagation.
    #[inline]
    pub const fn consumed() -> Self {
        Self {
            consumed: true,
            ..Self::ignored()
        }
    }

    /// Creates a non-consuming result that requests a redraw.
    #[inline]
    pub const fn redraw() -> Self {
        Self {
            needs_redraw: true,
            ..Self::ignored()
        }
    }

    /// Returns whether normal event propagation should stop.
    #[inline]
    pub const fn is_consumed(self) -> bool {
        self.consumed
    }

    /// Returns whether handling changed visual state that must be redrawn.
    #[inline]
    pub const fn needs_redraw(self) -> bool {
        self.needs_redraw
    }

    /// Adds a redraw request while preserving all other effects.
    #[inline]
    pub const fn with_redraw(mut self) -> Self {
        self.needs_redraw = true;
        self
    }

    /// Clears the redraw flag while preserving consumption and pointer effects.
    ///
    /// A transparent routing wrapper can use this after a child has already
    /// invalidated its own retained paint. The wrapper must schedule the frame
    /// separately so its full bounds do not become another paint invalidation.
    #[inline]
    pub const fn without_redraw(mut self) -> Self {
        self.needs_redraw = false;
        self
    }

    /// Requests persistent ownership of `pointer` by the producing element.
    #[inline]
    pub const fn with_pointer_capture(mut self, pointer: PointerKey) -> Self {
        self.capture_request = CaptureRequest::Capture(pointer);
        self
    }

    /// Requests release of `pointer` after this result is processed.
    #[inline]
    pub const fn with_pointer_release(mut self, pointer: PointerKey) -> Self {
        self.capture_request = CaptureRequest::Release(pointer);
        self
    }

    /// Returns the requested pointer-ownership change.
    #[inline]
    pub const fn capture_request(self) -> CaptureRequest {
        self.capture_request
    }

    /// Asks the dispatcher to route one drag event after this one.
    ///
    /// See [`FollowUp`] for why a drag needs a second pass at all.
    #[inline]
    pub const fn with_follow_up(mut self, follow_up: FollowUp) -> Self {
        self.follow_up = follow_up;
        self
    }

    /// Returns the extra pass this result asked for.
    #[inline]
    pub const fn follow_up(self) -> FollowUp {
        self.follow_up
    }

    /// Removes the follow-up request after a dispatcher has run it.
    #[inline]
    pub const fn without_follow_up(mut self) -> Self {
        self.follow_up = FollowUp::None;
        self
    }

    /// Removes the ownership request after a dispatcher has applied it.
    ///
    /// Consumption and redraw are preserved so nested dispatchers can return
    /// ordinary effects without accidentally replaying ownership changes in a
    /// parent registry.
    #[inline]
    pub const fn without_capture_request(mut self) -> Self {
        self.capture_request = CaptureRequest::None;
        self
    }

    /// Combines effects produced along one dispatch path.
    ///
    /// Consumption and redraw are accumulated independently.
    #[inline]
    pub const fn merge(self, other: Self) -> Self {
        Self {
            consumed: self.consumed || other.consumed,
            needs_redraw: self.needs_redraw || other.needs_redraw,
            capture_request: match self.capture_request {
                CaptureRequest::None => other.capture_request,
                request => request,
            },
            follow_up: match self.follow_up {
                FollowUp::None => other.follow_up,
                follow_up => follow_up,
            },
        }
    }
}

impl From<bool> for EventResult {
    #[inline]
    fn from(consumed: bool) -> Self {
        if consumed {
            Self::consumed()
        } else {
            Self::ignored()
        }
    }
}

// Event capabilities
pub trait EventElement: VisitorElement {
    /// Returns this element's event-routing role.
    ///
    /// Custom implementations default to conservative indexed hit testing.
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedHitTestBoundary
    }

    /// Returns the focus handle attached to this element, if it is focusable.
    fn focus_node(&self) -> Option<&FocusNode> {
        None
    }

    /// Returns whether this element should request focus on first attachment.
    fn autofocus(&self) -> bool {
        false
    }

    /// Returns whether this element confines keyboard focus to its subtree.
    ///
    /// A trapping element is a *focus scope*: while it is in the tree, only the
    /// focusable targets inside it are offered to the focus manager, so `Tab`
    /// cycles within it and nothing outside it can be given focus. That is what
    /// a dialog rendered inline needs — a target that is never offered can
    /// neither be reached nor focused, which is the whole of the confinement.
    ///
    /// The innermost trapping element wins, and the owner displaced when the
    /// scope appeared is restored once it leaves the tree. Overlays that own
    /// their own dispatch root, such as `aimer_modal`, confine focus with a
    /// [`FocusTrap`](crate::focus::FocusTrap) instead, because their content is
    /// not part of the tree they cover.
    fn traps_focus(&self) -> bool {
        false
    }

    /// Called when a pointer event hits this element.
    /// Return the independent effects produced while handling it.
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    /// Called by routed pointer dispatch with the owning dispatcher's shared
    /// capture and path state.
    ///
    /// Most elements only handle their own event and use the default. A
    /// routing boundary that owns a child outside its ordinary event-child
    /// view can override this method and call
    /// [`EventDispatchContext::dispatch_child`] without allocating a nested
    /// dispatcher and path index.
    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        _context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        self.on_event(event)
    }

    /// Visit children for event dispatch. By default delegates to
    /// `visit_children`. Override this when `visit_children` is not
    /// implemented (e.g. because the element handles its own child
    /// rendering) but events still need to reach the children.
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_children(visitor);
    }

    /// Visit the children that make up this element's structural tree.
    ///
    /// Focus traversal and reconciliation need the union of the event and
    /// visual child views. Elements whose two views have one canonical child
    /// source can override this method to avoid walking and de-duplicating
    /// that source twice. An override must visit every child exposed by either
    /// [`Self::event_children`] or [`VisitorElement::visit_children`] exactly
    /// once, in structural order.
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // CALLED.update(|c| c + 1);
        let mut children: SmallVec<[&'a dyn Element; 8]> = SmallVec::new();
        self.event_children(&mut |child| children.push(child));
        let event_child_count = children.len();
        let mut visual_index = 0;
        let mut seen: Option<HashSet<StructuralChildPointer<'a>>> = None;
        self.visit_children(&mut |child| {
            if visual_index < event_child_count
                && std::ptr::eq(children[visual_index], child)
            {
                visual_index += 1;
                return;
            }

            visual_index += 1;
            let child_pointer = StructuralChildPointer(std::ptr::from_ref(child));
            let already_seen = if let Some(seen) = seen.as_mut() {
                !seen.insert(child_pointer)
            } else if children.len() >= 16 {
                let mut pointers: HashSet<StructuralChildPointer<'a>> = children
                    .iter()
                    .map(|existing| StructuralChildPointer(std::ptr::from_ref(*existing)))
                    .collect();
                let already_seen = !pointers.insert(child_pointer);
                seen = Some(pointers);
                already_seen
            } else {
                children
                    .iter()
                    .any(|existing| std::ptr::eq(*existing, child))
            };
            if !already_seen {
                children.push(child);
            }
        });
        for child in children {
            visitor(child);
        }
    }

    /// Visit children that can participate in focus traversal.
    ///
    /// This defaults to the full structural tree because most elements use the
    /// same children for painting, events, reconciliation, and focus. A visual
    /// overlay that must remain part of reconciliation but cannot own focus can
    /// override this to omit that overlay while leaving
    /// [`EventElement::structural_children`] complete.
    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.structural_children(visitor);
    }

    /// Visit children that a pointer could plausibly hit.
    ///
    /// Position-based dispatch uses this hook instead of
    /// [`EventElement::event_children`], which lets an element that paints only
    /// part of its children — a clipped or virtualized list — keep the
    /// hit-test walk proportional to what is on screen. Focus-directed and
    /// broadcast delivery deliberately keep using `event_children`, so a child
    /// that was never painted still receives keyboard input and lifecycle
    /// notifications.
    ///
    /// The default forwards to `event_children`; overriding it is purely an
    /// optimization and must never hide a child that was actually drawn.
    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.event_children(visitor);
    }

    /// Visit hit-test children in reverse order for callers that need a
    /// topmost-first stream.
    ///
    /// The default collects the ordinary hit-test view and reverses it, which
    /// preserves the existing topmost-first behavior for custom elements. Large
    /// child containers can override this to walk their retained storage in
    /// reverse directly and avoid a temporary child buffer.
    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let mut children: SmallVec<[&'a dyn Element; 32]> = SmallVec::new();
        self.hit_test_children(&mut |child| children.push(child));
        for child in children.into_iter().rev() {
            visitor(child);
        }
    }

    /// Visit hit-test children that could contain a pointer at `pos`.
    ///
    /// The default delegates to the existing position-independent hook, so
    /// current element implementations retain their behavior. Containers with
    /// a retained spatial index may override this to skip known-outside
    /// children without affecting focus or broadcast traversal.
    #[inline]
    fn hit_test_children_at<'a>(&'a self, _pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.hit_test_children(visitor);
    }

    /// Visit position-aware hit-test children in routed topmost-first order for
    /// callers that request a reverse stream directly.
    ///
    /// The routed dispatcher uses [`EventElement::hit_test_children_at`] with
    /// one shared reverse scratch stack, so it does not call this default
    /// buffer. Direct callers still get the historical behavior, and large
    /// containers can override this method to query retained bounds directly.
    #[inline]
    fn hit_test_children_at_reversed<'a>(
        &'a self,
        _pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.hit_test_children_reversed(visitor);
    }

    /// Returns whether this element can expose overlapping pointer-hit
    /// children whose hover side effects must all be observed on every move.
    ///
    /// The routed dispatcher may cache a single hit chain for an uncaptured,
    /// non-consuming pointer move. A container that can paint several
    /// overlapping hit targets — a stack of layers is the usual example —
    /// must return `true` so a newly entered layer is not missed while the
    /// cached chain remains inside another layer. The default is appropriate
    /// for ordinary non-overlapping containers and leaves existing event
    /// behavior unchanged.
    #[inline]
    fn has_overlapping_hit_targets(&self) -> bool {
        false
    }

    /// Returns whether this element hides everything painted behind it at
    /// `pos` from the pointer.
    ///
    /// `pos` is already known to lie within the element's bounds. A layered
    /// parent such as a `Stack` asks this of the layers above a point, through
    /// [`crate::pointer_occluded_at`], to decide whether the layers below it may
    /// still see the pointer there. An element that paints a background over its
    /// box answers `true`. The default is `false`, which leaves a pointer free to
    /// reach whatever is behind.
    #[inline]
    fn occludes_pointer_at(&self, _pos: Vec2d) -> bool {
        false
    }

    /// Visits the children that [`crate::pointer_occluded_at`] searches for an
    /// occluder at `pos`.
    ///
    /// The default is the hit-test view, so a spatially indexed container keeps
    /// the search as narrow as its routing. An element that routes its child by
    /// hand and so hides it from that view should visit the child here.
    #[inline]
    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.hit_test_children_at(pos, visitor);
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::pointer::PointerSource;

    use super::{CaptureRequest, EventResult, PointerKey};

    #[test]
    fn event_result_combines_consumption_and_redraw() {
        let result = EventResult::consumed().with_redraw();

        assert!(result.is_consumed());
        assert!(result.needs_redraw());
    }

    #[test]
    fn merge_preserves_independent_flags() {
        let consumed = EventResult::consumed();
        let redraw = EventResult::redraw();

        let result = consumed.merge(redraw);

        assert!(result.is_consumed());
        assert!(result.needs_redraw());
    }

    #[test]
    fn ignored_event_result_has_no_effect() {
        assert_eq!(EventResult::ignored(), EventResult::default());
        assert!(!EventResult::ignored().is_consumed());
        assert!(!EventResult::ignored().needs_redraw());
    }

    #[test]
    fn capture_effect_preserves_consumption_and_redraw() {
        let pointer = PointerKey::new(PointerSource::Touch, 4);
        let result = EventResult::consumed()
            .with_redraw()
            .with_pointer_capture(pointer);

        assert!(result.is_consumed());
        assert!(result.needs_redraw());
        assert_eq!(result.capture_request(), CaptureRequest::Capture(pointer));
    }

    #[test]
    fn removing_forwarded_redraw_preserves_other_event_effects() {
        let pointer = PointerKey::new(PointerSource::Touch, 9);
        let result = EventResult::consumed()
            .with_redraw()
            .with_pointer_capture(pointer)
            .without_redraw();

        assert!(result.is_consumed());
        assert!(!result.needs_redraw());
        assert_eq!(result.capture_request(), CaptureRequest::Capture(pointer));
    }

    #[test]
    fn first_capture_effect_wins_when_results_merge() {
        let deepest = PointerKey::new(PointerSource::Touch, 1);
        let ancestor = PointerKey::new(PointerSource::Mouse, 1);

        let result = EventResult::ignored()
            .with_pointer_capture(deepest)
            .merge(EventResult::ignored().with_pointer_release(ancestor));

        assert_eq!(result.capture_request(), CaptureRequest::Capture(deepest));
    }
}
