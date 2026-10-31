//! The end of a pointer interaction: the lift, or a cancel.

use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_utils::AnimInstant;
use aimer_widget::{Element, EventResult, PointerKey};

use super::handle_scroll::{axis_velocity, dispatch_child_event, owns_pointer};
use crate::raw_scroll::{DragMode, RawScrollableContainer};
use crate::scrollable::constants::*;
use crate::scrollable::state::thumb_grab::ThumbGrab;

impl<E: Element> RawScrollableContainer<E> {
    /// Ends the interaction a `PointerUp` or `Cancel` closes: lets the child
    /// see it, arms the release fling from the drag's velocity, and drops the
    /// pointer ownership taken on press.
    ///
    /// `mode_before` is the drag mode the interaction was in when the event
    /// arrived, and `pos` the position the child is addressed at.
    pub(super) fn on_pointer_release(
        &self,
        event: &ElementEvent,
        pos: Vec2d,
        mode_before: DragMode,
    ) -> EventResult {
        let mut child_result = EventResult::ignored();
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
        match self.ctrl.end_thumb_grab() {
            ThumbGrab::Holding { since, .. } => thumb_log!(
                "finger lifted before the hold completed: down {} ms, needs {SCROLLBAR_THUMB_HOLD_MS} ms",
                now.duration_since(since).as_millis()
            ),
            ThumbGrab::Armed { since } => thumb_log!(
                "thumb let go after being held {} ms",
                now.duration_since(since).as_millis()
            ),
            ThumbGrab::Idle => {}
        }
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
        match event {
            ElementEvent::PointerUp(pointer) => {
                result.with_pointer_release(PointerKey::new(pointer.source, pointer.id))
            }
            _ => result,
        }
    }
}
