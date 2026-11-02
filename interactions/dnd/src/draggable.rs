//! Picking a widget up.
//!
//! [`Draggable`] recognizes the press-and-move that starts a drag, takes
//! ownership of the pointer so the rest of the gesture keeps reaching it, opens
//! the [`DragSession`], and asks the dispatcher to route the drag to whatever
//! is underneath.
//!
//! It recognizes that gesture itself rather than wrapping `GestureDetector`,
//! for one reason: a drag has to travel with the pointer *and* be handed to a
//! second element on the same event, and the detector's result has no way to
//! express the second half. The thresholds are the framework's own —
//! [`tap_slop`] and [`LONG_PRESS_DURATION`] — so a drag begins exactly where a
//! tap stops being a tap, for the device in hand.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_animation::AnimInstant;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_input::gesture::{LONG_PRESS_DURATION, tap_slop};
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, FollowUp, LayoutElement,
    PointerKey, Rebuildable, RequiredChild, VisitorElement, Widget,
};

use crate::overlay::{DragAxis, DragOverlay};
use crate::{DragPayload, DragSession};

/// When a press turns into a drag.
///
/// Touch and mouse want opposite answers. A mouse press that moves is
/// unambiguously a drag. A finger press that moves is much more likely to be a
/// scroll, and an enclosing [`Scrollable`] wants the same pointer — so on touch
/// the drag waits for a long press, which the user cannot perform by accident
/// while flicking a list.
///
/// [`Scrollable`]: https://docs.rs/aimer
///
/// # Examples
///
/// ```
/// use aimer_dnd::DragStartMode;
/// use aimer_events::pointer::PointerSource;
///
/// // Left to itself, each input device gets the behaviour that suits it.
/// assert_eq!(
///     DragStartMode::for_source(PointerSource::Mouse),
///     DragStartMode::Immediate
/// );
/// assert_eq!(
///     DragStartMode::for_source(PointerSource::Touch),
///     DragStartMode::LongPress
/// );
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragStartMode {
    /// The drag begins as soon as the pointer has moved past its device's slop.
    Immediate,
    /// The drag begins only once the pointer has also been held for
    /// [`LONG_PRESS_DURATION`], leaving shorter movements to whatever else
    /// wants them.
    LongPress,
}

impl DragStartMode {
    /// The mode a pointer of this kind gets when none was chosen.
    #[inline]
    pub const fn for_source(source: PointerSource) -> Self {
        match source {
            PointerSource::Mouse => Self::Immediate,
            _ => Self::LongPress,
        }
    }
}

/// A widget that can be picked up and dropped on a [`DragTarget`].
///
/// `new()` takes nothing and [`Draggable::child`] comes last: attaching the
/// child is what turns the builder into a [`Widget`].
///
/// A `Draggable` without [`Draggable::data`] carries nothing and will never be
/// accepted by any target; one without [`Draggable::feedback`] drags
/// invisibly. Both are permitted — a target that only cares *that* something is
/// being dragged is a legitimate, if unusual, thing to build — but neither is
/// what you normally want.
///
/// # Examples
///
/// ```no_run
/// use aimer_container::{Container, ZeroSizedBox};
/// use aimer_dnd::{DragAxis, Draggable};
///
/// #[derive(Clone)]
/// struct CardId(u32);
///
/// let card = Draggable::new()
///     .data(CardId(7))
///     .feedback(|| Container::new().child(ZeroSizedBox))
///     .axis(DragAxis::Vertical)
///     .child(Container::new().child(ZeroSizedBox));
/// ```
///
/// [`DragTarget`]: crate::DragTarget
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_dnd::Draggable", schema_only)]
pub struct Draggable<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_skip]
    payload: Option<Rc<dyn Fn() -> DragPayload>>,
    #[portable_skip]
    feedback: Option<Rc<dyn Fn() -> AnyWidget>>,
    #[portable_skip]
    child_when_dragging: Option<AnyWidget>,
    #[portable_skip]
    start_mode: Option<DragStartMode>,
    #[portable_skip]
    axis: DragAxis,
    #[portable_skip]
    on_drag_completed: Option<Rc<dyn Fn(bool)>>,
}

impl Default for Draggable {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Draggable {
    /// Creates an incomplete draggable that carries nothing.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            payload: None,
            feedback: None,
            child_when_dragging: None,
            start_mode: None,
            axis: DragAxis::Free,
            on_drag_completed: None,
        }
    }
}

impl<W> Draggable<W> {
    /// Sets the value a drop delivers.
    ///
    /// The value is cloned once per drag, so the same widget can be picked up
    /// again after a drop that consumed the previous copy.
    #[inline]
    pub fn data<T: Clone + 'static>(mut self, data: T) -> Self {
        self.payload = Some(Rc::new(move || DragPayload::new(data.clone())));
        self
    }

    /// Sets what is painted under the pointer while dragging.
    ///
    /// The widget is built on the first frame of the drag, not when this is
    /// called, so a closure that reads application state sees the state at
    /// pick-up time.
    #[inline]
    pub fn feedback<F, V>(mut self, feedback: F) -> Self
    where
        F: Fn() -> V + 'static,
        V: Widget + 'static,
    {
        self.feedback = Some(Rc::new(move || feedback().boxed()));
        self
    }

    /// Replaces the child in place while the drag is in flight.
    ///
    /// Typically a dimmed or outlined copy, so the space the item came from
    /// stays visible instead of collapsing under the layout.
    #[inline]
    pub fn child_when_dragging<V: Widget + 'static>(mut self, child: V) -> Self {
        self.child_when_dragging = Some(child.boxed());
        self
    }

    /// Overrides when a press becomes a drag.
    ///
    /// The default depends on the pointer — see [`DragStartMode::for_source`].
    #[inline]
    pub fn start_on(mut self, mode: DragStartMode) -> Self {
        self.start_mode = Some(mode);
        self
    }

    /// Restricts which way the feedback may move.
    #[inline]
    pub fn axis(mut self, axis: DragAxis) -> Self {
        self.axis = axis;
        self
    }

    /// Runs when the drag ends, with whether a target accepted it.
    #[inline]
    pub fn on_drag_completed<F: Fn(bool) + 'static>(mut self, on_drag_completed: F) -> Self {
        self.on_drag_completed = Some(Rc::new(on_drag_completed));
        self
    }

    /// Attaches the widget that can be picked up, completing the builder.
    #[inline]
    pub fn child<V: Widget>(self, child: V) -> Draggable<V> {
        Draggable {
            child,
            payload: self.payload,
            feedback: self.feedback,
            child_when_dragging: self.child_when_dragging,
            start_mode: self.start_mode,
            axis: self.axis,
            on_drag_completed: self.on_drag_completed,
        }
    }
}

impl<W: Widget + 'static> Widget for Draggable<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawDraggable {
            child_when_dragging: self.child_when_dragging.map(|child| child.to_element(ctx)),
            child: self.child.to_element(ctx),
            payload: self.payload.clone(),
            feedback: self.feedback.clone(),
            start_mode: self.start_mode,
            axis: self.axis,
            on_drag_completed: self.on_drag_completed.clone(),
            bounds: InteractionBounds::new(),
            press: RefCell::new(None),
            dragging: Cell::new(false),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Draggable"
    }
}

/// What is known about a press that has not yet become a drag.
struct Press {
    pointer: PointerKey,
    at: Vec2d,
    when: AnimInstant,
}

struct RawDraggable {
    child: AnyElement,
    child_when_dragging: Option<AnyElement>,
    payload: Option<Rc<dyn Fn() -> DragPayload>>,
    feedback: Option<Rc<dyn Fn() -> AnyWidget>>,
    start_mode: Option<DragStartMode>,
    axis: DragAxis,
    on_drag_completed: Option<Rc<dyn Fn(bool)>>,
    bounds: InteractionBounds,
    press: RefCell<Option<Press>>,
    dragging: Cell<bool>,
}

impl RawDraggable {
    /// Whether the press has travelled and waited far enough to be a drag.
    fn should_start(&self, press: &Press, pos: Vec2d) -> bool {
        let dx = pos.x - press.at.x;
        let dy = pos.y - press.at.y;
        // Per device: a finger is allowed eighteen pixels of wobble, a mouse
        // barely one. Using the touch tolerance for a mouse — which this did
        // until the slop became per-source — meant a card had to be dragged
        // almost two centimetres before it would pick up.
        if (dx * dx + dy * dy).sqrt() < tap_slop(press.pointer.source) {
            return false;
        }
        let mode = self
            .start_mode
            .unwrap_or_else(|| DragStartMode::for_source(press.pointer.source));
        match mode {
            DragStartMode::Immediate => true,
            DragStartMode::LongPress => {
                AnimInstant::now().duration_since(press.when) >= LONG_PRESS_DURATION
            }
        }
    }

    /// Opens the session and puts the feedback on screen.
    ///
    /// The drag is anchored at the *press*, not at the point that crossed the
    /// slop: that is where the user thinks they took hold of the card, and it
    /// is where a refused drop has to return to.
    fn begin(&self, press: &Press) -> bool {
        let Some(payload) = self.payload.as_ref() else {
            return false;
        };
        if !DragSession::begin(press.pointer, payload(), press.at) {
            return false;
        }
        // The card is being carried, so the pointer is its: see
        // [`aimer_widget::pointer_claim`]. Released with the pointer.
        aimer_widget::claim_pointer(press.pointer);

        let corner = self
            .bounds
            .pos_start_end()
            .map_or(press.at, |(start, _)| start);
        let grab_offset = Vec2d {
            x: press.at.x - corner.x,
            y: press.at.y - corner.y,
        };
        DragOverlay::show(self.feedback.clone(), grab_offset, press.at, self.axis);
        self.dragging.set(true);
        true
    }

    /// Ends the drag without a drop: a cancelled gesture, or a window that lost
    /// focus mid-flight.
    fn abandon(&self, pointer: Option<PointerKey>) {
        let owns_pointer = self
            .press
            .borrow()
            .as_ref()
            .is_some_and(|press| pointer.is_none_or(|pointer| press.pointer == pointer));
        if !owns_pointer {
            return;
        }
        if let Some(press) = self.press.borrow_mut().take()
            && self.dragging.replace(false)
        {
            DragSession::cancel(press.pointer);
            DragOverlay::spring_back();
            if let Some(completed) = self.on_drag_completed.as_ref() {
                completed(false);
            }
        }
    }
}

impl VisitorElement for RawDraggable {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
        if let Some(child) = self.child_when_dragging.as_ref() {
            visitor(child.as_ref());
        }
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        visitor(0, self.visible_child());
    }

    fn debug_name(&self) -> &'static str {
        "Draggable"
    }
}

impl EventElement for RawDraggable {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(info) => {
                if !self.bounds.is_inside(info.x(), info.y()) {
                    return EventResult::ignored();
                }
                // The public session intentionally supports one active drag at
                // a time. Do not capture a second pointer only to strand it
                // until release, and do not overwrite a pending press on this
                // element.
                if DragSession::is_active() || self.press.borrow().is_some() {
                    return EventResult::ignored();
                }
                let pointer = PointerKey::new(info.source, info.id);
                *self.press.borrow_mut() = Some(Press {
                    pointer,
                    at: info.pos,
                    when: AnimInstant::now(),
                });
                // Ownership is taken now, not when the drag starts: once the
                // pointer has left these bounds there is no second chance to
                // ask for it, and a press that turns out to be a tap gives it
                // straight back on release.
                EventResult::ignored().with_pointer_capture(pointer)
            }

            ElementEvent::PointerMove(info) => {
                let pointer = PointerKey::new(info.source, info.id);
                let mut press = self.press.borrow_mut();
                let Some(press) = press.as_mut().filter(|press| press.pointer == pointer) else {
                    return EventResult::ignored();
                };

                if !self.dragging.get() {
                    if !self.should_start(press, info.pos) {
                        return EventResult::ignored();
                    }
                    if !self.begin(press) {
                        return EventResult::ignored();
                    }
                }

                DragSession::update(pointer, info.pos);
                EventResult::consumed()
                    .with_redraw()
                    .with_follow_up(FollowUp::DragOver)
            }

            ElementEvent::PointerUp(info) => {
                let pointer = PointerKey::new(info.source, info.id);
                let taken = self
                    .press
                    .borrow_mut()
                    .take_if(|press| press.pointer == pointer)
                    .is_some();
                if !taken || !self.dragging.replace(false) {
                    // The dispatcher releases on PointerUp as a safety net,
                    // while this explicit request also makes direct element
                    // adapters and lost-state tests observe the cleanup.
                    return EventResult::ignored().with_pointer_release(pointer);
                }

                // The drop itself is delivered by the follow-up pass that this
                // result asks for; whether anybody took the payload is only
                // knowable afterwards, so the overlay settles it on the frame
                // the drop requested.
                DragOverlay::resolve_on_next_frame(self.on_drag_completed.clone());
                EventResult::consumed()
                    .with_redraw()
                    .with_follow_up(FollowUp::DragDrop)
            }

            ElementEvent::PointerExited(source, id) => {
                let pointer = PointerKey::new(*source, *id);
                self.abandon(Some(pointer));
                EventResult::ignored().with_pointer_release(pointer)
            }

            ElementEvent::Cancel => {
                self.abandon(None);
                EventResult::ignored()
            }

            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.visible_child());
    }
}

impl RawDraggable {
    /// The child that is currently on screen.
    fn visible_child(&self) -> &dyn Element {
        match (self.dragging.get(), self.child_when_dragging.as_ref()) {
            (true, Some(child)) => child.as_ref(),
            _ => self.child.as_ref(),
        }
    }
}

impl LayoutElement for RawDraggable {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.visible_child().layout(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.visible_child().computed_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawDraggable {
    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.visible_child().computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Draggable")
    }

    fn update(&self, ctx: &BuildContext) {
        let child = self.visible_child();
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        child.update(ctx);
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        let child = self.visible_child();
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        child.sync_paint_geometry(ctx);
        true
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        let child = self.visible_child();
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        child.update(ctx);
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        std::ptr::eq(child, self.visible_child()).then(|| ctx.clone())
    }
}

impl Rebuildable for RawDraggable {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Claims the gesture the element being replaced was in the middle of.
    ///
    /// A drag outlives the element that started it. The first drop target the
    /// pointer reaches highlights itself, that rebuild replaces every element
    /// the target built — including this one — and a replacement that started
    /// from nothing would drop the press on the floor: the moves that follow
    /// would find no gesture to continue, the release would ask for no drop, and
    /// the feedback would hang in mid-air above a session nothing can close.
    ///
    /// So the gesture is *moved* out of `old`, which reconciliation drops
    /// immediately afterwards, leaving exactly one element carrying the drag.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };

        *self.press.borrow_mut() = old.press.borrow_mut().take();
        self.dragging.set(old.dragging.replace(false));
    }
}

#[cfg(test)]
mod tests {
    use aimer_container::{Container, ZeroSizedBox};
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_widget::CaptureRequest;

    use super::*;
    use crate::test_support::headless_context;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct CardId(u32);

    fn mouse(x: f32, y: f32) -> PointerInfo {
        PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary)
    }

    fn down(x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerDown(mouse(x, y))
    }

    fn moved(x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerMove(mouse(x, y))
    }

    fn lifted(x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerUp(mouse(x, y))
    }

    /// Comfortably past the tolerance for whichever device is being simulated.
    fn past_slop(source: PointerSource) -> f32 {
        tap_slop(source) + 4.0
    }

    /// A card sitting at the origin, big enough to be pressed inside.
    fn card_element(ctx: &BuildContext) -> AnyElement {
        let element = Draggable::new()
            .data(CardId(7))
            .child(Container::new().width(120).height(60).child(ZeroSizedBox))
            .to_element(ctx);
        element.layout(ctx);
        element
    }

    /// Every test shares the session and the overlay, so each starts from
    /// nothing.
    fn fresh() {
        DragSession::cancel_any();
        DragOverlay::hide();
    }

    #[test]
    fn a_mouse_drags_at_once_and_a_finger_waits() {
        assert_eq!(
            DragStartMode::for_source(PointerSource::Mouse),
            DragStartMode::Immediate
        );
        assert_eq!(
            DragStartMode::for_source(PointerSource::Touch),
            DragStartMode::LongPress
        );
    }

    #[test]
    fn a_press_that_travels_opens_the_session() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);

        let _ = card.on_event(&down(10.0, 10.0));
        let result = card.on_event(&moved(10.0 + past_slop(PointerSource::Mouse), 12.0));

        assert!(result.is_consumed());
        assert_eq!(result.follow_up(), FollowUp::DragOver);
        assert_eq!(DragSession::with_payload(|id: &CardId| id.0), Some(7));

        fresh();
    }

    /// A card that is being carried owns the gesture. An ancestor that arbitrates
    /// touches, such as a navigator that pops on a sideways swipe, must be able to
    /// tell, so the pointer is claimed the moment the card is picked up.
    #[test]
    fn picking_a_card_up_claims_the_pointer_and_pressing_it_does_not() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        let _ = card.on_event(&down(10.0, 10.0));
        assert!(!aimer_widget::is_pointer_claimed(pointer), "a press is not yet a drag");
        let _ = card.on_event(&moved(10.0 + past_slop(PointerSource::Mouse), 12.0));
        assert!(aimer_widget::is_pointer_claimed(pointer), "the card was picked up");

        aimer_widget::release_pointer(pointer);
        fresh();
    }

    // The mouse tolerance used to be the *touch* tolerance, so a card had to be
    // dragged eighteen logical pixels — nearly two centimetres — before it would
    // pick up, while a five-pixel drag did nothing at all.
    #[test]
    fn a_short_mouse_drag_is_enough_to_pick_a_card_up() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);

        let _ = card.on_event(&down(10.0, 10.0));
        let result = card.on_event(&moved(15.0, 10.0));

        assert!(
            DragSession::is_active(),
            "5 px of deliberate mouse travel is a drag"
        );
        assert_eq!(result.follow_up(), FollowUp::DragOver);

        fresh();
    }

    // The same movement by finger is still a tap: a contact patch centimetres
    // wide rolls as it presses, so a card must not leap out from under it.
    #[test]
    fn the_same_short_travel_by_finger_does_not_pick_a_card_up() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);
        let finger = PointerInfo::touch(Vec2d { x: 10.0, y: 10.0 }, 0);

        let _ = card.on_event(&ElementEvent::PointerDown(finger));
        let _ = card.on_event(&ElementEvent::PointerMove(
            finger.at(Vec2d { x: 15.0, y: 10.0 }),
        ));

        assert!(!DragSession::is_active(), "5 px of finger wobble is a tap");

        fresh();
    }

    /// A drop target highlights itself the moment a drag arrives, and that
    /// rebuild replaces every element it built — including the card being
    /// carried. The replacement has to keep carrying it, or the drag freezes in
    /// mid-air with the session still open and nothing left to close it.
    #[test]
    fn a_drag_survives_the_rebuild_that_replaces_the_card() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);

        let _ = card.on_event(&down(10.0, 10.0));
        let _ = card.on_event(&moved(10.0 + past_slop(PointerSource::Mouse), 12.0));
        assert!(DragSession::is_active(), "the drag has begun");

        // Exactly what reconciliation does with the element a rebuild produced.
        let rebuilt = card_element(&ctx);
        rebuilt.adopt_runtime_state_from(card.as_ref());
        drop(card);

        let result = rebuilt.on_event(&moved(200.0, 180.0));

        assert!(result.is_consumed(), "the replacement still owns the drag");
        assert_eq!(result.follow_up(), FollowUp::DragOver);
        assert_eq!(
            DragSession::position(),
            Some(Vec2d { x: 200.0, y: 180.0 }),
            "the feedback follows the pointer across the rebuild"
        );

        let release = rebuilt.on_event(&lifted(200.0, 180.0));

        assert_eq!(
            release.follow_up(),
            FollowUp::DragDrop,
            "releasing still asks for the drop to be routed"
        );

        fresh();
    }

    #[test]
    fn a_second_pointer_is_not_captured_while_another_drag_is_active() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);
        let other_pointer = PointerKey::new(PointerSource::Touch, 4);

        assert!(DragSession::begin(
            other_pointer,
            DragPayload::new(CardId(8)),
            Vec2d::default()
        ));
        let result = card.on_event(&down(10.0, 10.0));

        assert_eq!(result.capture_request(), CaptureRequest::None);
        assert_eq!(DragSession::pointer(), Some(other_pointer));
        fresh();
    }

    #[test]
    fn a_lost_pointer_releases_capture_and_cancels_the_drag() {
        fresh();
        let ctx = headless_context(400.0, 400.0);
        let card = card_element(&ctx);
        let pointer = PointerKey::new(PointerSource::Mouse, 0);

        let _ = card.on_event(&down(10.0, 10.0));
        let _ = card.on_event(&moved(15.0, 10.0));
        assert!(DragSession::is_active());

        let result = card.on_event(&ElementEvent::PointerExited(pointer.source, pointer.id));

        assert_eq!(result.capture_request(), CaptureRequest::Release(pointer));
        assert!(!DragSession::is_active());
        fresh();
    }

    #[test]
    fn draggable_publishes_a_derived_required_child_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <Draggable<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::exactly(1)
        );
    }
}
