//! A touch landing on the scrollable, or a mouse press on its content.

use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_utils::AnimInstant;
use aimer_widget::{Element, EventResult, PointerKey};

use super::handle_scroll::{Flow, content_drag_allowed, drag_overscroll_source};
use crate::raw_scroll::{DragMode, RawScrollableContainer};
use crate::scrollable::constants::*;

impl<E: Element> RawScrollableContainer<E> {
    /// Handles a `PointerDown`: takes the touch, stops any fling in flight,
    /// and decides whether the press grabbed a scroll bar thumb or only armed
    /// a pending content drag.
    ///
    /// `child_result` is what the content already did with the same event.
    pub(super) fn on_pointer_down(&self, event: &ElementEvent, child_result: EventResult) -> Flow {
        let ElementEvent::PointerDown(pointer) = event else {
            return Flow::Continue(false);
        };
        let p = &pointer.pos;
        // A mouse press that reached here is not on the bar, which took
        // its own in `route_to_scroll_bar`: it is the content's.
        if pointer.source == PointerSource::Mouse {
            return Flow::Return(child_result);
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
                return Flow::Return(child_result.merge(EventResult::consumed()));
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

        // Whatever grab the last finger had is over.
        self.ctrl.end_thumb_grab();
        let on_thumb = self
            .ctrl
            .thumb_at(*p, pointer.source != PointerSource::Mouse);
        thumb_log!(
            "press id={} at ({:.1}, {:.1}) bar opacity {:.2} (needed to take the thumb: {:.2}) thumb v={:?} h={:?} -> {}",
            pointer.id,
            p.x,
            p.y,
            self.ctrl.bar.opacity.get(),
            SCROLLBAR_TOUCH_MIN_OPACITY,
            self.ctrl.bar.v_thumb_rect.get(),
            self.ctrl.bar.h_thumb_rect.get(),
            match on_thumb {
                Some(thumb) => format!(
                    "on the {thumb:?} thumb, hold started (needs {SCROLLBAR_THUMB_HOLD_MS} ms){}",
                    if self.ctrl.bar.opacity.get() < SCROLLBAR_TOUCH_MIN_OPACITY {
                        ", bar is hidden: showing it first"
                    } else {
                        ""
                    }
                ),
                None => format!(
                    "not on the thumb, a content press: {}",
                    self.ctrl.describe_touch_miss(*p)
                ),
            }
        );
        if let Some(thumb) = on_thumb {
            // A fingertip near the edge of a page is usually after the page,
            // not the bar, so the thumb is not taken on contact. This starts a
            // hold; only a finger that rests takes the thumb (see
            // `settle_thumb_hold`), and one that swipes first scrolls the
            // content. The press stays a pending content drag meanwhile.
            self.ctrl.begin_thumb_hold(thumb, *p, AnimInstant::now());
            self.warm_thumb_feedback();
            self.ctrl.request_animation_frame();
        }

        self.ctrl.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        self.ctrl.clear_velocity_history();
        // Reset the velocity-sampling accumulator so a fresh gesture
        // doesn't inherit stale coalesced delta / timing.
        self.ctrl.drag_velocity.accum.set(Vec2d { x: 0.0, y: 0.0 });
        self.ctrl.drag_velocity.sample_time.set(None);
        // A fresh touch/click stops the in-flight release fling.
        self.ctrl.cancel_fling();
        self.ctrl.fling.momentum_start.set(None);

        if !content_drag_allowed(PointerKey::new(pointer.source, pointer.id)) {
            // A descendant is already dragging with this pointer — text
            // being selected, a selection knob being moved. Arming a
            // pending drag would end that gesture the moment the finger
            // passed the threshold, so the press is left entirely to the
            // child and this scrollable forgets the pointer.
            self.ctrl.end_thumb_grab();
            self.ctrl.drag_mode.set(DragMode::None);
            self.ctrl.last_pointer_pos.set(None);
            self.ctrl.active_touch_id.set(None);
            return Flow::Return(child_result);
        }

        self.ctrl.drag_mode.set(DragMode::Pending);
        self.ctrl.last_pointer_pos.set(Some(*p));
        Flow::Continue(true)
    }
}
