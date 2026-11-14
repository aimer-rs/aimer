use std::collections::HashSet;
use crate::Element;
use crate::components::element::{EventDispatchContext, VisitorElement};
use crate::focus::FocusNode;
use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use crate::CALLED;

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
/// Elements default to [`Transparent`](Self::Transparent): they handle no events
/// and are not part of the index. An element that handles events declares
/// [`IndexedHitTestBoundary`](Self::IndexedHitTestBoundary), which keeps its
/// callback and custom child hit-test behavior in the index, or
/// [`IndexedTarget`](Self::IndexedTarget) when event-child order is also the
/// correct positional hit-test order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EventTreeRole {
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
    IndexedHitTestBoundary,
    /// This element does not handle events; descendants may be routed through
    /// it through the index. It must not impose a custom hit-test boundary.
    #[default]
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
/// use aimer_element::{EventResult, FollowUp};
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

/// The input half of an element: routing, focus, accessibility and handling.
///
/// Input reaches an element in three ways, and each walks a different view of
/// the tree. Those views are methods of [`VisitorElement`], next to
/// [`visit_children`](VisitorElement::visit_children), because they answer which
/// children an element has and not what it does with an event:
///
/// - **Positional** (a pointer at some point): the dispatcher descends through
///   [`hit_test_children`](VisitorElement::hit_test_children) and its
///   position-aware variants, topmost first, so an element that paints only
///   part of its children can keep the walk proportional to what is on screen.
/// - **Directed** (keyboard input to the focused element, or a pointer that has
///   been captured): delivered by identity, through
///   [`event_children`](VisitorElement::event_children) and
///   [`focus_children`](VisitorElement::focus_children).
/// - **Broadcast** (lifecycle notifications): every element, through
///   [`event_children`](VisitorElement::event_children).
///
/// All three default to [`VisitorElement::visit_children`], so an element whose
/// children are the same for painting and events implements none of them.
/// [`structural_children`](VisitorElement::structural_children) is the union of
/// the event and visual views, which is what focus traversal and reconciliation
/// need.
///
/// An element reacts to an event in [`on_event`](EventElement::on_event) and
/// reports the outcome as an [`EventResult`]: whether it consumed the event,
/// whether a redraw is needed, and whether it wants to capture or release the
/// pointer. The same trait carries what makes an element focusable
/// ([`focus_node`](EventElement::focus_node),
/// [`autofocus`](EventElement::autofocus),
/// [`traps_focus`](EventElement::traps_focus)) and what it tells assistive
/// technology ([`semantics`](EventElement::semantics)).
///
/// Every method has a default, so a non-interactive element implements it with
/// an empty `impl`, and costs event routing nothing: the default
/// [`EventTreeRole`] is [`Transparent`](EventTreeRole::Transparent), which keeps
/// the element out of the event index. An element that handles events declares a
/// role that registers it; see [`event_tree_role`](EventElement::event_tree_role).
pub trait EventElement: VisitorElement {
    /// Returns this element's event-routing role.
    ///
    /// The default is [`EventTreeRole::Transparent`]: an element is registered
    /// in the event tree only when it says it handles events. **An element that
    /// overrides [`on_event`](Self::on_event),
    /// [`on_event_with_context`](Self::on_event_with_context) or
    /// [`focus_node`](Self::focus_node), or that narrows hit testing through
    /// [`hit_test_children`](VisitorElement::hit_test_children) or one of its
    /// variants, must override this too**, or the dispatcher never reaches it.
    /// [`IndexedHitTestBoundary`](EventTreeRole::IndexedHitTestBoundary) is the
    /// safe choice, and [`IndexedTarget`](EventTreeRole::IndexedTarget) is the
    /// cheaper one for an element whose event children are also its hit-test
    /// order.
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
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

    /// Returns the keyboard shortcuts this element declares for its subtree.
    ///
    /// Only an element that exists to bind shortcuts overrides this. The
    /// dispatcher offers a key chord that nothing focused consumed to the scopes
    /// of the focused element's ancestors, innermost first, and then to every
    /// [window](aimer_shortcut::ShortcutScope::window) scope in the tree. It
    /// does so only while a scope exists at all, so it costs nothing otherwise.
    #[inline]
    fn shortcut_scope(&self) -> Option<&aimer_shortcut::ShortcutScope> {
        None
    }

    /// Describes this element to assistive technology, or `None` when it has
    /// nothing of its own to say.
    ///
    /// `id` is this element's identity as the accessibility tree must publish
    /// it; build the node with it so actions can be routed back. An element that
    /// returns `None` is transparent: its descendants are published under the
    /// nearest ancestor that does describe itself. Return children-free nodes
    /// here, since the collector attaches the descendants.
    ///
    /// Only called while an assistive technology is attached.
    #[inline]
    fn semantics(&self, _id: aimer_accessibility::NodeId) -> Option<aimer_accessibility::SemanticNode> {
        None
    }

    /// Performs an action requested by assistive technology on this element.
    ///
    /// Returns whether the element handled it. Only actions the element listed
    /// in [`Self::semantics`] are ever requested.
    #[inline]
    fn perform_semantic_action(&self, _action: &aimer_accessibility::SemanticAction) -> bool {
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
