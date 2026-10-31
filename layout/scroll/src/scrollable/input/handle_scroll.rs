use aimer_animation::curves;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_events::pointer::PointerSource;
use aimer_utils::AnimInstant;
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

/// The overscroll source a pointer gesture belongs to.
///
/// Touch content drags and mouse scrollbar drags share the scroll physics, but
/// are not the same device to a target that only trusts some of them with a
/// rubber band, so the distinction is kept all the way into the scroll engine.
#[inline]
fn drag_overscroll_source(source: PointerSource) -> OverscrollSource {
    match source {
        PointerSource::Touch => OverscrollSource::Touch,
        PointerSource::Mouse => OverscrollSource::Mouse,
    }
}

fn drag_start_threshold() -> f32 {
    DRAG_START_THRESHOLD_DP
}

fn owns_pointer(active_pointer: Option<u64>, pointer: u64) -> bool {
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
fn content_drag_allowed(pointer: PointerKey) -> bool {
    pointer.source == PointerSource::Touch && !is_pointer_claimed(pointer)
}

fn pointer_drag_delta(
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
fn axis_velocity(axis: ScrollAxis, velocity: Vec2d) -> Vec2d {
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
fn active_axis_changed(axis: ScrollAxis, before: Vec2d, after: Vec2d) -> bool {
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

fn dispatch_child_event<E: Element>(
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
            self.ctrl.viewport_origin.set(Vec2d {
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
            return EventResult::ignored();
        }

        let pos = child_dispatch_position(event, cursor);

        let mode_before = self.ctrl.drag_mode.get();
        let mut pending_content_drag_won = false;
        let mut child_result = EventResult::ignored();

        if matches!(event, ElementEvent::PointerUp(_) | ElementEvent::Cancel) {
            if let ElementEvent::PointerUp(pointer) = event
                && self
                    .ctrl
                    .active_touch_id
                    .get()
                    .is_some_and(|active| active != pointer.id)
            {
                return EventResult::ignored();
            }
            let owned_pointer = match event {
                ElementEvent::PointerUp(pointer) => {
                    owns_pointer(self.ctrl.active_touch_id.get(), pointer.id)
                }
                ElementEvent::Cancel => {
                    self.ctrl.active_touch_id.get().is_some() || mode_before != DragMode::None
                }
                _ => false,
            };
            if matches!(
                mode_before,
                DragMode::VerticalScrollbar | DragMode::HorizontalScrollbar
            ) {
                match event {
                    ElementEvent::PointerUp(pointer) => {
                        child_result =
                            child_result.merge(self.event_dispatcher.borrow_mut().cancel_pointer(
                                &self.child,
                                PointerKey::new(pointer.source, pointer.id),
                            ));
                    }
                    ElementEvent::Cancel => {
                        child_result = child_result.merge(dispatch_child_event(self, pos, event));
                    }
                    _ => {}
                }
            } else if matches!(mode_before, DragMode::None | DragMode::Pending)
                && matches!(event, ElementEvent::PointerUp(_))
            {
                child_result = child_result.merge(dispatch_child_event(self, pos, event));
            }

            let now = AnimInstant::now();
            // info!("[scroll] PointerUp mode_before={:?} drag_mode={:?}", mode_before,
            // self.ctrl.drag_mode.get());
            if let Some(last_time) = self.ctrl.last_event_time.get() {
                let elapsed = now.duration_since(last_time).as_millis();
                if elapsed > VELOCITY_RESET_IDLE_MS {
                    // info!("[scroll] FLING CLEARED — idle too long ({}ms > {}ms threshold)",
                    // elapsed, VELOCITY_RESET_IDLE_MS);
                    self.ctrl.pointer_velocity.set(Vec2d::default());
                    self.ctrl.clear_velocity_history();
                    self.ctrl.cancel_fling();
                } else {
                    // The gesture ended between two samples, so whatever the
                    // finger did since the last one has not been measured yet
                    // — including doing nothing at all, which is precisely how
                    // a drag that was deliberately brought to a stop before
                    // the lift looks. Closing that slice is what keeps such a
                    // release from inheriting the speed of the swing that
                    // preceded it.
                    if mode_before == DragMode::Content
                        && let Some((velocity, sample_dt)) = self.ctrl.flush_drag_velocity(now)
                    {
                        self.ctrl.push_velocity(
                            axis_velocity(self.ctrl.axis, velocity),
                            sample_dt,
                            now,
                        );
                    }
                    let max_v = MAX_SCROLL_VELOCITY * self.ctrl.last_scale.get();
                    let raw = self.ctrl.smoothed_velocity(now);
                    let sv = Vec2d {
                        x: (raw.x * RELEASE_VELOCITY_GAIN).clamp(-max_v, max_v),
                        y: (raw.y * RELEASE_VELOCITY_GAIN).clamp(-max_v, max_v),
                    };
                    // info!("[scroll] FLING ARMED elapsed={}ms raw=({:.2},{:.2}) gain=({:.2},{:.2})
                    // max_v={:.0}", elapsed, raw.x, raw.y, sv.x, sv.y, max_v);
                    self.ctrl.cancel_fling();
                    self.ctrl.pointer_velocity.set(sv);
                }
            }

            self.ctrl.last_frame_time.set(Some(now));
            self.ctrl.drag_mode.set(DragMode::None);
            self.ctrl.last_pointer_pos.set(None);
            // A pointer interaction owns the edge from here on, so a wheel or
            // trackpad gesture can no longer hold the stretch: without this a
            // contact hold whose lift was never reported would freeze it.
            self.ctrl.release_overscroll_recovery();
            self.ctrl.begin_device_contact(false);
            match event {
                ElementEvent::PointerUp(pointer) => {
                    if self.ctrl.active_touch_id.get() == Some(pointer.id) {
                        self.ctrl.active_touch_id.set(None);
                    }
                }
                _ => self.ctrl.active_touch_id.set(None),
            }
            self.ctrl.request_animation_frame();
            let result = child_result.merge(EventResult::from(owned_pointer).with_redraw());
            return match event {
                ElementEvent::PointerUp(pointer) => {
                    result.with_pointer_release(PointerKey::new(pointer.source, pointer.id))
                }
                _ => result,
            };
        }

        // ── Pending scrolls are decided after the child gets this move. ──
        // A nested scrollable may win the same axis-dominant gesture, claim
        // the pointer, and leave no gesture for this ancestor. If it is at a
        // hard edge it returns the pointer to us, enabling edge handoff.
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

        let we_consumed = match event {
            ElementEvent::Scroll {
                delta,
                kind,
                phase,
                is_direct_manipulation,
            } => {
                // A nested scrollable gets first refusal. It may consume a
                // diagonal frame on its own axis while leaving the cross-axis
                // component untouched; forwarding the original event after
                // that would make the ancestor scroll at the same time. A
                // passive opaque wrapper can also consume Scroll to occlude a
                // lower stack layer, but it must not hide this ancestor when
                // no descendant actually moved.
                if child_result.is_consumed() && child_result.needs_redraw() {
                    return child_result;
                }
                if !scroll_axis_matches_intent(self.ctrl.axis, *delta) {
                    return child_result;
                }
                let moved =
                    apply_scroll_frame(&self.ctrl, *delta, *kind, *phase, *is_direct_manipulation);
                if moved {
                    // Whoever stood in front of this scroller, a navigator watching
                    // for a sideways swipe, has to be able to tell that the content
                    // moved: a retained child reports every result as the same.
                    aimer_events::pointer::note_scroll_used();
                    self.ctrl.begin_scroll();
                    self.ctrl.record_input_event();
                    self.ctrl.request_animation_frame();
                }
                moved
            }
            ElementEvent::PointerDown(pointer) => {
                let p = &pointer.pos;
                // A mouse press that reached here is not on the bar, which took
                // its own in `route_to_scroll_bar`: it is the content's.
                if pointer.source == PointerSource::Mouse {
                    return child_result;
                }
                if let Some(prev_id) = self.ctrl.active_touch_id.get()
                    && prev_id != pointer.id
                {
                    let stale = self.ctrl.last_event_time.get().is_none_or(|t| {
                        AnimInstant::now().duration_since(t).as_millis() > STALE_TOUCH_THRESHOLD_MS
                    });
                    if stale {
                        // info!("[scroll] DOWN stale touch cleared prev_id={}", prev_id);
                        self.ctrl.active_touch_id.set(None);
                        self.ctrl.drag_mode.set(DragMode::None);
                        self.ctrl.last_pointer_pos.set(None);
                    } else {
                        // info!("[scroll] DOWN REJECTED — secondary finger prev_id={} new_id={}",
                        // prev_id, id);
                        return child_result.merge(EventResult::consumed());
                    }
                }
                self.ctrl.active_touch_id.set(Some(pointer.id));
                // A touch contact or mouse scrollbar interaction takes over from
                // any wheel/trackpad gesture, so it must not inherit that
                // gesture's recovery state nor be judged by its input source.
                self.ctrl
                    .set_overscroll_source(drag_overscroll_source(pointer.source));
                self.ctrl.release_overscroll_recovery();
                self.ctrl.begin_device_contact(false);
                self.ctrl.reset_overscroll_peak();
                // info!("[scroll] PointerDown id={} pos=({:.1},{:.1})", id, p.x, p.y);

                let mut mode = DragMode::Pending;
                if let Some(thumb) = self
                    .ctrl
                    .thumb_at(*p, pointer.source != PointerSource::Mouse)
                {
                    mode = thumb;
                    // Remember where the thumb was taken, so that it stays under
                    // the pointer however the drag goes.
                    let device = self.ctrl.to_device(*p);
                    let along = if thumb == DragMode::VerticalScrollbar {
                        (device.y, self.ctrl.scroll_offset.get().y)
                    } else {
                        (device.x, self.ctrl.scroll_offset.get().x)
                    };
                    self.ctrl.scroll_bar_anchor.set(along);
                    // A finger on the thumb is not a swipe, whatever is waiting
                    // to read it as one.
                    aimer_widget::claim_pointer(PointerKey::new(pointer.source, pointer.id));
                }

                self.ctrl.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
                self.ctrl.clear_velocity_history();
                // Reset the velocity-sampling accumulator so a fresh gesture
                // doesn't inherit stale coalesced delta / timing.
                self.ctrl.vel_accum.set(Vec2d { x: 0.0, y: 0.0 });
                self.ctrl.vel_sample_time.set(None);
                // A fresh touch/click stops the in-flight release fling.
                self.ctrl.cancel_fling();
                self.ctrl.momentum_start_time.set(None);

                if mode == DragMode::Pending
                    && !content_drag_allowed(PointerKey::new(pointer.source, pointer.id))
                {
                    // A descendant is already dragging with this pointer — text
                    // being selected, a selection knob being moved. Arming a
                    // pending drag would end that gesture the moment the finger
                    // passed the threshold, so the press is left entirely to the
                    // child and this scrollable forgets the pointer.
                    self.ctrl.drag_mode.set(DragMode::None);
                    self.ctrl.last_pointer_pos.set(None);
                    self.ctrl.active_touch_id.set(None);
                    return child_result;
                }

                self.ctrl.drag_mode.set(mode);
                self.ctrl.last_pointer_pos.set(Some(*p));
                true
            }
            ElementEvent::PointerMove(pointer) => {
                let p = &pointer.pos;
                // Ignore moves from non-primary fingers.
                if self.ctrl.active_touch_id.get().is_some()
                    && self.ctrl.active_touch_id.get() != Some(pointer.id)
                {
                    // info!("[scroll] MOVE REJECTED — non-primary finger active={:?} got={}",
                    // self.ctrl.active_touch_id.get(), id);
                    return child_result;
                }

                let mut mode = self.ctrl.drag_mode.get();
                #[allow(clippy::collapsible_if)]
                if mode == DragMode::Pending {
                    if self.ctrl.last_pointer_pos.get().is_some() {
                        if pending_content_drag_won {
                            mode = DragMode::Content;
                            self.ctrl.drag_mode.set(DragMode::Content);
                        } else {
                            return child_result;
                        }
                    }
                }

                if mode != DragMode::None && mode != DragMode::Pending {
                    // The content (or a scrollbar thumb) is actively being dragged
                    // — the start of a scroll session. Edge-triggered, so repeated
                    // moves within the same drag don't re-fire.
                    self.ctrl.begin_scroll();
                    if let Some(last) = self.ctrl.last_pointer_pos.get() {
                        let speed_multiplier = self.ctrl.speed_multiplier;
                        let delta = pointer_drag_delta(
                            last,
                            *p,
                            speed_multiplier,
                            pending_content_drag_won,
                        );
                        let dx = delta.x;
                        let dy = delta.y;

                        let now = AnimInstant::now();
                        self.ctrl.last_event_time.set(Some(now));
                        let velocity_delta = match mode {
                            DragMode::Content => axis_velocity(self.ctrl.axis, delta),
                            _ => Vec2d::ZERO,
                        };
                        if let Some((raw_velocity, sample_dt)) = self.ctrl.accumulate_drag_velocity(
                            velocity_delta.x,
                            velocity_delta.y,
                            now,
                        ) {
                            let mut new_velocity = match mode {
                                DragMode::Content => axis_velocity(self.ctrl.axis, raw_velocity),
                                _ => Vec2d { x: 0.0, y: 0.0 },
                            };

                            let mut old_velocity = self.ctrl.pointer_velocity.get();
                            let reversed_x = new_velocity.x * old_velocity.x < 0.0;
                            let reversed_y = new_velocity.y * old_velocity.y < 0.0;
                            if reversed_x || reversed_y {
                                self.ctrl.clear_velocity_history();
                                if reversed_x {
                                    old_velocity.x = 0.0;
                                }
                                if reversed_y {
                                    old_velocity.y = 0.0;
                                }
                            }

                            self.ctrl.push_velocity(new_velocity, sample_dt, now);

                            let blend_factor = (sample_dt / DRAG_BLEND_WINDOW).min(1.0);
                            let blend_new =
                                (DRAG_BLEND_BASE * (1.0 - blend_factor) + blend_factor).min(1.0);
                            let blend_old = 1.0 - blend_new;

                            new_velocity.x =
                                old_velocity.x * blend_old + new_velocity.x * blend_new;
                            new_velocity.y =
                                old_velocity.y * blend_old + new_velocity.y * blend_new;

                            self.ctrl.pointer_velocity.set(new_velocity);
                        }

                        let offset_before = self.ctrl.scroll_offset.get();
                        let mut offset = offset_before;

                        match mode {
                            DragMode::Content => {
                                // Same rubber band the wheel / trackpad path
                                // uses: resistance grows with the stretch, so a
                                // finger held past the edge stops pulling the
                                // content further out.
                                let step =
                                    self.ctrl.resisted_overscroll_delta(match self.ctrl.axis {
                                        ScrollAxis::Vertical => Vec2d { x: 0.0, y: dy },
                                        ScrollAxis::Horizontal => Vec2d { x: dx, y: 0.0 },
                                    });
                                offset.x += step.x;
                                offset.y += step.y;
                            }
                            // The thumb is measured from where it was taken, so
                            // it stays under the pointer instead of lagging it.
                            DragMode::VerticalScrollbar => {
                                let (from, anchored) = self.ctrl.scroll_bar_anchor.get();
                                offset.y = anchored
                                    - (self.ctrl.to_device(*p).y - from)
                                        * self.ctrl.v_scroll_multiplier.get();
                            }
                            DragMode::HorizontalScrollbar => {
                                let (from, anchored) = self.ctrl.scroll_bar_anchor.get();
                                offset.x = anchored
                                    - (self.ctrl.to_device(*p).x - from)
                                        * self.ctrl.h_scroll_multiplier.get();
                            }
                            _ => {}
                        }

                        // The bar ends where the content does: only a drag of the
                        // content itself stretches past an edge.
                        if !self.ctrl.bouncy() || mode != DragMode::Content {
                            offset = self.ctrl.clamp_offset(offset);
                        }
                        self.ctrl.set_scroll_offset(offset);
                        let moved = active_axis_changed(
                            self.ctrl.axis,
                            offset_before,
                            self.ctrl.scroll_offset.get(),
                        );
                        if mode == DragMode::Content && !moved && !pending_content_drag_won {
                            // The inner scrollable reached a hard edge. Let
                            // the ancestor inspect this same move instead of
                            // consuming a gesture that made no progress.
                            self.ctrl.pointer_velocity.set(Vec2d::ZERO);
                            self.ctrl.clear_velocity_history();
                            self.ctrl.drag_mode.set(DragMode::Pending);
                            self.ctrl.last_pointer_pos.set(Some(*p));
                            // The gesture is no longer this scroller's, so what it
                            // claimed while it moved is given back to whoever
                            // takes it next.
                            aimer_widget::release_pointer(PointerKey::new(
                                pointer.source,
                                pointer.id,
                            ));
                            return child_result;
                        }
                        if mode == DragMode::Content && moved {
                            // The content is moving under this finger. Saying so
                            // lets an ancestor that arbitrates touches, such as a
                            // navigator that pops on a sideways swipe, leave the
                            // gesture alone instead of inferring it.
                            aimer_widget::claim_pointer(PointerKey::new(
                                pointer.source,
                                pointer.id,
                            ));
                        }
                        self.ctrl.record_input_event();
                    }
                    self.ctrl.last_pointer_pos.set(Some(*p));
                    self.ctrl.request_animation_frame();
                    return child_result.merge(EventResult::consumed().with_redraw());
                }

                // The thumb highlight is resolved from the cursor at draw
                // time, and hover moves no longer buy a frame each — so the
                // move that carries the cursor across a thumb edge schedules
                // the one frame the highlight needs, and every other move
                // schedules nothing.
                // Only a mouse hovers: a finger that has lifted leaves no cursor
                // behind to hold the bar.
                if pointer.source == PointerSource::Mouse {
                    // Only the bar itself shows the bar: a mouse over the
                    // content must not, so a move asks for a frame solely when
                    // it crosses the edge of the thumb or track.
                    if self.ctrl.sync_thumb_hover(*p) {
                        self.ctrl.request_animation_frame();
                    }
                }
                false
            }
            // A held key arrives as a press followed by the platform's repeats, and
            // each one is another step: the glide builds on itself, so holding
            // scrolls continuously.
            ElementEvent::KeyInput {
                key,
                action: KeyAction::Pressed | KeyAction::Repeat,
                ..
            } => {
                if child_result.is_consumed() {
                    return child_result;
                }
                let scale = self.ctrl.last_scale.get();
                let (vp_w, vp_h) = self.ctrl.cached_viewport.get();
                let line = KEYBOARD_SCROLL_STEP * scale;
                let page_v = vp_h * KEYBOARD_PAGE_FRACTION;
                let page_h = vp_w * KEYBOARD_PAGE_FRACTION;

                // Steps glide: a push that carries the content on and settles.
                // Home and End name a destination, so they ease to it.
                let max = self.ctrl.cached_max_scroll.get();
                let vertical = matches!(self.ctrl.axis, ScrollAxis::Vertical);
                let (step, destination) = match (&self.ctrl.axis, key) {
                    (ScrollAxis::Vertical, NamedKey::ArrowUp) => (Some(Vec2d { x: 0.0, y: line }), None),
                    (ScrollAxis::Vertical, NamedKey::ArrowDown) => (Some(Vec2d { x: 0.0, y: -line }), None),
                    (ScrollAxis::Vertical, NamedKey::PageUp) => (Some(Vec2d { x: 0.0, y: page_v }), None),
                    (ScrollAxis::Vertical, NamedKey::PageDown) => (Some(Vec2d { x: 0.0, y: -page_v }), None),
                    (ScrollAxis::Horizontal, NamedKey::ArrowLeft) => (Some(Vec2d { x: line, y: 0.0 }), None),
                    (ScrollAxis::Horizontal, NamedKey::ArrowRight) => (Some(Vec2d { x: -line, y: 0.0 }), None),
                    (ScrollAxis::Horizontal, NamedKey::PageUp) => (Some(Vec2d { x: page_h, y: 0.0 }), None),
                    (ScrollAxis::Horizontal, NamedKey::PageDown) => (Some(Vec2d { x: -page_h, y: 0.0 }), None),
                    // Top: offset 0. Bottom: -max_scroll.
                    (_, NamedKey::Home) => (None, Some(Vec2d::ZERO)),
                    (_, NamedKey::End) => (
                        None,
                        Some(if vertical {
                            Vec2d { x: 0.0, y: -max.y }
                        } else {
                            Vec2d { x: -max.x, y: 0.0 }
                        }),
                    ),
                    _ => (None, None),
                };

                if let Some(step) = step {
                    self.ctrl.set_overscroll_source(OverscrollSource::Keyboard);
                    self.ctrl.glide_by(step, true);
                    true
                } else if let Some(destination) = destination {
                    self.ctrl.set_overscroll_source(OverscrollSource::Keyboard);
                    self.ctrl.clear_velocity_history();
                    self.ctrl.animate_scroll_to(
                        destination,
                        KEYBOARD_SCROLL_DURATION_S,
                        curves::EASE_OUT,
                    );
                    true
                } else {
                    false
                }
            }
            // PointerUp/Cancel is handled early above (before child dispatch),
            // so it never reaches this match.
            ElementEvent::PointerUp(pointer) => {
                // Release primary-finger lock.
                if self.ctrl.active_touch_id.get() == Some(pointer.id) {
                    self.ctrl.active_touch_id.set(None);
                }
                false
            }
            ElementEvent::Cancel => {
                self.ctrl.active_touch_id.set(None);
                false
            }
            ElementEvent::PointerExited(_, _)
            | ElementEvent::CharInput { .. }
            | ElementEvent::KeyInput { .. }
            | ElementEvent::ImePreedit { .. } => false,

            _ => false,
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
