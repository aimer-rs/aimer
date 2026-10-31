use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_events::element::{ElementEvent, KeyAction, TouchPhase};
use aimer_events::pointer::PointerSource;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    Element, EventDispatchContext, EventElement, EventResult, EventTreeRole, LayoutElement,
    PointerKey, VisitorElement, is_pointer_claimed,
};

use crate::ScrollAxis;
use crate::raw_scroll::{DragMode, RawScrollableContainer};
use crate::scrollable::constants::*;
use crate::scrollable::overscroll_source::OverscrollSource;
use crate::scrollable::scroll_frame::apply_scroll_frame;

const AXIS_DOMINANCE_RATIO: f32 = 1.2;

/// What a per-event handler hands back to `on_event`.
pub(super) enum Flow {
    /// Carry on to the shared tail with whether this scrollable used the event.
    Continue(bool),
    /// Stop here and return this finished result.
    Return(EventResult),
}

/// The overscroll source a pointer gesture belongs to.
///
/// Touch content drags and mouse scrollbar drags share the scroll physics, but
/// are not the same device to a target that only trusts some of them with a
/// rubber band, so the distinction is kept all the way into the scroll engine.
#[inline]
pub(super) fn drag_overscroll_source(source: PointerSource) -> OverscrollSource {
    match source {
        PointerSource::Touch => OverscrollSource::Touch,
        PointerSource::Mouse => OverscrollSource::Mouse,
    }
}

fn drag_start_threshold() -> f32 {
    DRAG_START_THRESHOLD_DP
}

pub(super) fn owns_pointer(active_pointer: Option<u64>, pointer: u64) -> bool {
    active_pointer == Some(pointer)
}

fn pending_content_drag_wins(
    axis: ScrollAxis,
    start: Vec2d,
    current: Vec2d,
    threshold: f32,
) -> bool {
    let dx = current.x - start.x;
    let dy = current.y - start.y;
    match axis {
        ScrollAxis::Vertical => dy.abs() > threshold && dy.abs() > dx.abs() * AXIS_DOMINANCE_RATIO,
        ScrollAxis::Horizontal => {
            dx.abs() > threshold && dx.abs() > dy.abs() * AXIS_DOMINANCE_RATIO
        }
    }
}

/// Whether a touch gesture is still free to become a content scroll.
///
/// A press inside a scroll view is ambiguous — it could be a tap, or the start
/// of a scroll — so the view arms a pending drag and takes the gesture from
/// whoever was under the finger as soon as it travels far enough. That is right
/// for a button and wrong for a descendant that is already dragging something of
/// its own: dragging across text to select it, or moving a selection knob. Such
/// a descendant [claims](aimer_widget::pointer_claim) the pointer, and a claimed
/// pointer is not available here, which is what lets text inside a scroll view
/// be selected at all.
///
/// A pointer capture is deliberately *not* consulted: every gesture recognizer
/// captures on press, so refusing to scroll over anything that captured would
/// make a scroll view full of buttons unscrollable.
#[inline]
pub(super) fn content_drag_allowed(pointer: PointerKey) -> bool {
    pointer.source == PointerSource::Touch && !is_pointer_claimed(pointer)
}

pub(super) fn pointer_drag_delta(
    last: Vec2d,
    current: Vec2d,
    speed_multiplier: f32,
    content_drag_just_won: bool,
) -> Vec2d {
    if content_drag_just_won {
        return Vec2d::default();
    }
    Vec2d {
        x: (current.x - last.x) * speed_multiplier,
        y: (current.y - last.y) * speed_multiplier,
    }
}

/// Keep only the component of `velocity` the container can actually scroll.
///
/// A finger never travels along a single axis, and letting the cross-axis
/// component through would seed a fling the viewport cannot show.
#[inline]
pub(super) fn axis_velocity(axis: ScrollAxis, velocity: Vec2d) -> Vec2d {
    match axis {
        ScrollAxis::Vertical => Vec2d {
            x: 0.0,
            y: velocity.y,
        },
        ScrollAxis::Horizontal => Vec2d {
            x: velocity.x,
            y: 0.0,
        },
    }
}

#[inline]
pub(super) fn active_axis_changed(axis: ScrollAxis, before: Vec2d, after: Vec2d) -> bool {
    match axis {
        ScrollAxis::Vertical => before.y != after.y,
        ScrollAxis::Horizontal => before.x != after.x,
    }
}

/// Whether this scrollable's axis matches the dominant direction of a
/// diagonal wheel or trackpad frame. A horizontal code block must not consume
/// the tiny horizontal drift that commonly accompanies a primarily vertical
/// scroll intended for its ancestor.
#[inline]
fn scroll_axis_matches_intent(axis: ScrollAxis, delta: Vec2d) -> bool {
    match axis {
        ScrollAxis::Vertical => delta.y.abs() > delta.x.abs() * AXIS_DOMINANCE_RATIO,
        ScrollAxis::Horizontal => delta.x.abs() > delta.y.abs() * AXIS_DOMINANCE_RATIO,
    }
}

fn child_dispatch_position(event: &ElementEvent, cursor: Vec2d) -> Vec2d {
    event.get_pointer_pos().unwrap_or(cursor)
}

fn event_pointer_key(event: &ElementEvent) -> Option<PointerKey> {
    match event {
        ElementEvent::PointerDown(pointer)
        | ElementEvent::PointerUp(pointer)
        | ElementEvent::PointerMove(pointer) => Some(PointerKey::new(pointer.source, pointer.id)),
        ElementEvent::PointerExited(source, id) => Some(PointerKey::new(*source, *id)),
        _ => None,
    }
}

fn child_route_allowed(inside: bool, active_drag: bool, child_captured: bool) -> bool {
    inside || active_drag || child_captured
}

pub(super) fn dispatch_child_event<E: Element>(
    scrollable: &RawScrollableContainer<E>,
    pos: Vec2d,
    event: &ElementEvent,
) -> EventResult {
    let pointer = event_pointer_key(event);
    let was_captured =
        pointer.is_some_and(|pointer| scrollable.event_dispatcher.borrow().is_captured(pointer));
    // The child is part of the enclosing tree's focus, not a focus of its own: a
    // field inside a scroll is typed into through whoever the enclosing
    // dispatcher says owns the keyboard. See `on_event_with_context`.
    let result = {
        let mut dispatcher = scrollable.event_dispatcher.borrow_mut();
        dispatcher.defer_focus_to_parent();
        dispatcher.dispatch(&scrollable.child, pos, event)
    };
    let is_captured =
        pointer.is_some_and(|pointer| scrollable.event_dispatcher.borrow().is_captured(pointer));
    match (pointer, was_captured, is_captured) {
        (Some(pointer), false, true) => result.with_pointer_capture(pointer),
        (Some(pointer), true, false) => result.with_pointer_release(pointer),
        _ => result,
    }
}

impl<E: Element> RawScrollableContainer<E> {
    /// Tells the scroll state where the viewport is in the window, so the bar's
    /// viewport-relative geometry can be compared with window-space pointers.
    #[inline]
    pub(crate) fn sync_viewport_origin(&self) {
        if let Some(bounds) = self.bounds.get_bounds() {
            self.ctrl.bar.viewport_origin.set(Vec2d {
                x: bounds.x,
                y: bounds.y,
            });
        }
    }

    /// Offers a mouse event to the active scroll bar, and says whether the bar
    /// took it.
    ///
    /// The bar strip holds a hover region and a gesture detector, but is not part
    /// of the content subtree `event_dispatcher` is rooted at, so it has its own
    /// dispatcher. Every mouse move reaches it, since a region needs to see the
    /// cursor leave as much as arrive. A press reaches it only on the bar's
    /// thumb or track, where it belongs to the bar and not to the content under
    /// it. From then on the bar owns the pointer until it lets go: a drag that
    /// leaves the viewport keeps following, and the release lands on the bar.
    ///
    /// Returns `Some` with the result to hand back when the bar owns the event.
    fn route_to_scroll_bar(&self, event: &ElementEvent, cursor: Vec2d) -> Option<EventResult> {
        let pointer = match event {
            ElementEvent::PointerDown(info)
            | ElementEvent::PointerUp(info)
            | ElementEvent::PointerMove(info)
                if info.source == PointerSource::Mouse =>
            {
                Some(PointerKey::new(info.source, info.id))
            }
            ElementEvent::PointerExited(PointerSource::Mouse, id) => {
                Some(PointerKey::new(PointerSource::Mouse, *id))
            }
            ElementEvent::Cancel => None,
            _ => return None,
        };
        let bar = match self.ctrl.axis {
            ScrollAxis::Vertical => &self.vertical_scroll_bar,
            ScrollAxis::Horizontal => &self.horizontal_scroll_bar,
        }
        .as_ref()?;
        // A press beside the bar is the content's.
        if matches!(event, ElementEvent::PointerDown(_)) && self.ctrl.bar_hit(cursor).is_none() {
            return None;
        }
        let mut dispatcher = self.bar_dispatcher.borrow_mut();
        let was_captured = pointer.is_some_and(|pointer| dispatcher.is_captured(pointer));
        if matches!(event, ElementEvent::Cancel) && dispatcher.capture_count() == 0 {
            return None;
        }
        let result = dispatcher.dispatch(bar, cursor, event);
        let is_captured = pointer.is_some_and(|pointer| dispatcher.is_captured(pointer));
        drop(dispatcher);

        let result = match (pointer, was_captured, is_captured) {
            (Some(pointer), false, true) => result.with_pointer_capture(pointer),
            (Some(pointer), true, false) => result.with_pointer_release(pointer),
            _ => result,
        };
        let owns = was_captured
            || is_captured
            || matches!(event, ElementEvent::Cancel)
            || (matches!(event, ElementEvent::PointerDown(_)) && result.is_consumed());
        owns.then_some(result)
    }

    /// Everything `on_event` does for a pointer that is no longer over the
    /// content: a panel's grab band over the scrollable's edge is the usual
    /// case.
    fn on_event_outside_content(&self, event: &ElementEvent, cursor: Vec2d) -> EventResult {
        if self.ctrl.clear_thumb_hover() {
            self.ctrl.request_animation_frame();
        }
        // The pointer is still over this element as far as the enclosing
        // tree is concerned, but no longer over the content: a panel's grab
        // band over the scrollable's edge is the usual case. Nothing
        // further reaches the child dispatcher, so whatever it was
        // hovering has to be told here, or it stays lit until the pointer
        // happens to come back.
        if let ElementEvent::PointerMove(info) = event {
            let pointer = PointerKey::new(info.source, info.id);
            if self.event_dispatcher.borrow().is_hovering(pointer) {
                let exit = ElementEvent::PointerExited(info.source, info.id);
                return dispatch_child_event(
                    self,
                    child_dispatch_position(&exit, cursor),
                    &exit,
                )
                .without_capture_request();
            }
        }
        EventResult::ignored()
    }

    /// Lets the child see a press or move, and decides whether a pending
    /// press has become a content drag.
    ///
    /// Returns what the child did, and whether the pending drag won.
    fn dispatch_pending(
        &self,
        event: &ElementEvent,
        pos: Vec2d,
        mode_before: DragMode,
    ) -> (EventResult, bool) {
        let mut pending_content_drag_won = false;
        let mut child_result = EventResult::ignored();
        if mode_before == DragMode::None || mode_before == DragMode::Pending {
            child_result = child_result.merge(dispatch_child_event(self, pos, event));
            if let ElementEvent::PointerMove(pointer) = event
                && mode_before == DragMode::Pending
                && content_drag_allowed(PointerKey::new(pointer.source, pointer.id))
                && !(child_result.is_consumed() && child_result.needs_redraw())
                && self
                    .ctrl
                    .active_touch_id
                    .get()
                    .is_none_or(|active| active == pointer.id)
            {
                pending_content_drag_won = self.ctrl.last_pointer_pos.get().is_some_and(|start| {
                    pending_content_drag_wins(
                        self.ctrl.axis,
                        start,
                        pointer.pos,
                        drag_start_threshold(),
                    )
                });
                if pending_content_drag_won {
                    let pointer_key = PointerKey::new(pointer.source, pointer.id);
                    child_result = child_result.merge(
                        self.event_dispatcher
                            .borrow_mut()
                            .cancel_pointer(&self.child, pointer_key),
                    );
                }
            }
        }
        (child_result, pending_content_drag_won)
    }

    /// Whether the content currently rests past an edge of its scroll range.
    #[inline]
    fn is_stretched(&self) -> bool {
        let stretch = self.ctrl.overscroll_distance(self.ctrl.scroll_offset.get());
        stretch.x != 0.0 || stretch.y != 0.0
    }

    /// Handles a wheel or trackpad frame the content did not already use.
    fn on_scroll_frame(&self, event: &ElementEvent, child_result: EventResult) -> Flow {
        let ElementEvent::Scroll {
            delta,
            kind,
            phase,
            is_direct_manipulation,
        } = event
        else {
            return Flow::Continue(false);
        };
        // A nested scrollable gets first refusal. It may consume a
        // diagonal frame on its own axis while leaving the cross-axis
        // component untouched; forwarding the original event after
        // that would make the ancestor scroll at the same time. A
        // passive opaque wrapper can also consume Scroll to occlude a
        // lower stack layer, but it must not hide this ancestor when
        // no descendant actually moved.
        if child_result.is_consumed() && child_result.needs_redraw() {
            return Flow::Return(child_result);
        }
        // The frame that ends a gesture is let through even when it carries
        // nothing for this axis. The platform closes a gesture whose distance
        // was already delivered with an empty frame, and a little sideways
        // drift can ride on the last one; neither scrolls anything, but the
        // scroller must hear that the fingers lifted or the stretch they were
        // holding is never let go.
        let ends_gesture = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        let delta = if scroll_axis_matches_intent(self.ctrl.axis, *delta) {
            *delta
        } else if ends_gesture {
            Vec2d::ZERO
        } else {
            return Flow::Return(child_result);
        };
        let moved = apply_scroll_frame(&self.ctrl, delta, *kind, *phase, *is_direct_manipulation);
        if !moved && ends_gesture && self.is_stretched() {
            // Nothing moved, but letting go is what allows the edge to spring
            // back, and that is driven by frames.
            self.ctrl.request_animation_frame();
        }
        if moved {
            // Whoever stood in front of this scroller, a navigator watching
            // for a sideways swipe, has to be able to tell that the content
            // moved: a retained child reports every result as the same.
            aimer_events::pointer::note_scroll_used();
            self.ctrl.begin_scroll();
            self.ctrl.record_input_event();
            self.ctrl.request_animation_frame();
        }
        Flow::Continue(moved)
    }
}

impl<E: Element> EventElement for RawScrollableContainer<E> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    /// Hands the focus target a press found inside the content to the enclosing
    /// dispatcher, which is the one text and key events are delivered through.
    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        let result = self.on_event(event);
        if matches!(event, ElementEvent::PointerDown(_)) {
            context.adopt_deferred_focus(&mut self.event_dispatcher.borrow_mut());
        }
        result
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if let Some(cursor_pos) = event.get_pointer_pos() {
            self.ctrl.cursor_pos.set(Some(cursor_pos));
        }

        self.sync_viewport_origin();

        let cursor = match self.ctrl.cursor_pos.get() {
            Some(cursor) => cursor,
            None if matches!(event, ElementEvent::Cancel) => Vec2d::default(),
            None => return EventResult::ignored(),
        };

        if let Some(result) = self.route_to_scroll_bar(event, cursor) {
            return result;
        }

        // A mouse that has left is no longer on the thumb.
        if matches!(event, ElementEvent::PointerExited(PointerSource::Mouse, _))
            && self.ctrl.clear_thumb_hover()
        {
            self.ctrl.request_animation_frame();
        }

        let active_drag = self.ctrl.drag_mode.get() != DragMode::None;
        let child_captured = event_pointer_key(event)
            .is_some_and(|pointer| self.event_dispatcher.borrow().is_captured(pointer))
            || (matches!(event, ElementEvent::Cancel)
                && self.event_dispatcher.borrow().capture_count() > 0);

        let inside = self.bounds.is_inside(cursor.x, cursor.y);
        if !child_route_allowed(inside, active_drag, child_captured) {
            return self.on_event_outside_content(event, cursor);
        }

        let pos = child_dispatch_position(event, cursor);

        let mode_before = self.ctrl.drag_mode.get();
        if matches!(event, ElementEvent::PointerUp(_) | ElementEvent::Cancel) {
            return self.on_pointer_release(event, pos, mode_before);
        }

        // ── Pending scrolls are decided after the child gets this move. ──
        let (child_result, pending_content_drag_won) =
            self.dispatch_pending(event, pos, mode_before);

        let handled = match event {
            ElementEvent::Scroll { .. } => self.on_scroll_frame(event, child_result),
            ElementEvent::PointerDown(_) => self.on_pointer_down(event, child_result),
            ElementEvent::PointerMove(_) => {
                self.on_pointer_move(event, child_result, pending_content_drag_won)
            }
            ElementEvent::KeyInput {
                action: KeyAction::Pressed | KeyAction::Repeat,
                ..
            } => self.on_key_press(event, child_result),
            // PointerUp/Cancel is handled early above (before child dispatch),
            // so it never reaches this match.
            ElementEvent::PointerUp(pointer) => {
                // Release primary-finger lock.
                if self.ctrl.active_touch_id.get() == Some(pointer.id) {
                    self.ctrl.active_touch_id.set(None);
                }
                Flow::Continue(false)
            }
            ElementEvent::Cancel => {
                self.ctrl.active_touch_id.set(None);
                Flow::Continue(false)
            }
            _ => Flow::Continue(false),
        };
        let we_consumed = match handled {
            Flow::Continue(consumed) => consumed,
            Flow::Return(result) => return result,
        };

        let own_result = match event {
            ElementEvent::Scroll { .. } if we_consumed => EventResult::consumed().with_redraw(),
            _ => EventResult::from(we_consumed),
        };
        let result = child_result.merge(own_result);
        match event {
            ElementEvent::PointerDown(pointer) if we_consumed => {
                result.with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
            }
            ElementEvent::PointerMove(pointer) if pending_content_drag_won => {
                result.with_pointer_capture(PointerKey::new(pointer.source, pointer.id))
            }
            _ => result,
        }
    }

    fn event_children<'a>(&'a self, _: &mut dyn FnMut(&'a dyn Element)) {}

    /// Event dispatch is handled by the nested dispatcher, but focus and
    /// reconciliation still need the child and live scroll bars from the
    /// visual structural view.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_children(visitor);
    }
}

impl<E: Element> VisitorElement for RawScrollableContainer<E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
        if let Some(vertical_scroll_bar) = &self.vertical_scroll_bar {
            visitor(vertical_scroll_bar);
        }
        if let Some(horizontal_scroll_bar) = &self.horizontal_scroll_bar {
            visitor(horizontal_scroll_bar);
        }
    }

    fn debug_name(&self) -> &'static str {
        "RawScrollableContainer"
    }
}

impl<E: Element> LayoutElement for RawScrollableContainer<E> {
    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.layout_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.cached_content_size(ctx)
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::Vec2d;
    use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
    use aimer_events::pointer::PointerSource;
    use aimer_widget::PointerKey;

    use super::{
        child_dispatch_position, child_route_allowed, content_drag_allowed, drag_start_threshold,
        owns_pointer, pending_content_drag_wins, pointer_drag_delta,
    };
    use crate::ScrollAxis;

    #[test]
    fn move_exactly_at_drag_threshold_remains_pending() {
        assert!(!pending_content_drag_wins(
            ScrollAxis::Vertical,
            Vec2d::default(),
            Vec2d { x: 0.0, y: 10.0 },
            10.0,
        ));
    }

    #[test]
    fn drag_threshold_stays_in_logical_pixels_at_high_display_scale() {
        assert_eq!(drag_start_threshold(), 10.0);
    }

    #[test]
    fn scrollable_only_owns_its_active_pointer() {
        assert!(owns_pointer(Some(7), 7));
        assert!(!owns_pointer(Some(7), 8));
        assert!(!owns_pointer(None, 7));
    }

    #[test]
    fn axis_dominant_move_above_threshold_wins_scrolling() {
        assert!(pending_content_drag_wins(
            ScrollAxis::Vertical,
            Vec2d::default(),
            Vec2d { x: 2.0, y: 10.01 },
            10.0,
        ));
        assert!(pending_content_drag_wins(
            ScrollAxis::Horizontal,
            Vec2d::default(),
            Vec2d { x: -10.01, y: 2.0 },
            10.0,
        ));
    }

    #[test]
    fn equal_diagonal_move_does_not_win_scrolling() {
        assert!(!pending_content_drag_wins(
            ScrollAxis::Vertical,
            Vec2d::default(),
            Vec2d { x: 12.0, y: 12.0 },
            10.0,
        ));
    }

    #[test]
    fn near_diagonal_text_selection_does_not_win_scrolling() {
        assert!(!pending_content_drag_wins(
            ScrollAxis::Vertical,
            Vec2d::default(),
            Vec2d { x: 149.0, y: 149.5 },
            10.0,
        ));
        assert!(!pending_content_drag_wins(
            ScrollAxis::Horizontal,
            Vec2d::default(),
            Vec2d { x: 149.5, y: 149.0 },
            10.0,
        ));
    }

    #[test]
    fn cross_axis_dominant_move_does_not_win_scrolling() {
        assert!(!pending_content_drag_wins(
            ScrollAxis::Vertical,
            Vec2d::default(),
            Vec2d { x: 15.0, y: 12.0 },
            10.0,
        ));
    }

    #[test]
    fn winning_move_establishes_scroll_origin_without_changing_offset() {
        let delta = pointer_drag_delta(
            Vec2d { x: 20.0, y: 110.0 },
            Vec2d { x: 20.0, y: 150.0 },
            1.0,
            true,
        );

        assert_eq!(delta.x, 0.0);
        assert_eq!(delta.y, 0.0);
    }

    #[test]
    fn scroll_child_hit_testing_uses_cached_cursor_instead_of_delta() {
        let cursor = Vec2d { x: 50.0, y: 60.0 };
        let event = ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -120.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        };

        let position = child_dispatch_position(&event, cursor);
        assert_eq!(position.x, cursor.x);
        assert_eq!(position.y, cursor.y);
    }

    #[test]
    fn child_outside_viewport_requires_an_active_capture() {
        assert!(!child_route_allowed(false, false, false));
        assert!(child_route_allowed(false, false, true));
    }

    #[test]
    fn child_inside_viewport_uses_normal_hit_testing() {
        assert!(child_route_allowed(true, false, false));
    }

    #[test]
    fn a_pointer_claimed_by_a_descendant_cannot_become_a_scroll() {
        let pointer = PointerKey::new(PointerSource::Touch, 7);
        assert!(content_drag_allowed(pointer));

        aimer_widget::claim_pointer(pointer);

        assert!(
            !content_drag_allowed(pointer),
            "a text being selected owns this gesture; stealing it would end the selection"
        );
        assert!(
            content_drag_allowed(PointerKey::new(PointerSource::Touch, 8)),
            "another finger is unaffected"
        );

        aimer_widget::release_pointer(pointer);
        assert!(
            content_drag_allowed(pointer),
            "the gesture is over, so the next drag of that pointer scrolls again"
        );
    }

    #[test]
    fn a_mouse_pointer_cannot_start_a_content_scroll_drag() {
        assert!(!content_drag_allowed(PointerKey::new(
            PointerSource::Mouse,
            0
        )));
    }
}
