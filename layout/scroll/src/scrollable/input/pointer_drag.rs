//! A pointer moving over the scrollable: a drag in progress, a pending drag
//! deciding whether it is a scroll, or a mouse hovering.

use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::PointerSource;
use aimer_utils::AnimInstant;
use aimer_widget::{Element, EventResult, PointerKey};

use super::thumb_grab::HoldTrigger;
use super::handle_scroll::{Flow, active_axis_changed, axis_velocity, pointer_drag_delta};
use crate::ScrollAxis;
use crate::raw_scroll::{DragMode, RawScrollableContainer};
use crate::scrollable::constants::*;

impl<E: Element> RawScrollableContainer<E> {
    /// Handles a `PointerMove`.
    ///
    /// While a content or thumb drag is active it moves the offset and feeds
    /// the velocity sampler; while a press is pending it only promotes the
    /// press to a content drag once `pending_content_drag_won`; otherwise a
    /// mouse move just refreshes the thumb hover.
    pub(super) fn on_pointer_move(
        &self,
        event: &ElementEvent,
        child_result: EventResult,
        pending_content_drag_won: bool,
    ) -> Flow {
        let ElementEvent::PointerMove(pointer) = event else {
            return Flow::Continue(false);
        };
        let p = &pointer.pos;
        // Ignore moves from non-primary fingers.
        if self.ctrl.active_touch_id.get().is_some()
            && self.ctrl.active_touch_id.get() != Some(pointer.id)
        {
            // info!("[scroll] MOVE REJECTED — non-primary finger active={:?} got={}",
            // self.ctrl.active_touch_id.get(), id);
            return Flow::Return(child_result);
        }

        // A finger that rests on the thumb takes it, even if the frame that
        // would have noticed has not run yet; one that has travelled off was
        // swiping, and the hold is dropped for the content to have it.
        if self.ctrl.thumb_holding() && self.ctrl.follow_thumb_hold(*p) {
            self.settle_thumb_hold(AnimInstant::now(), HoldTrigger::Move);
        }

        let mut mode = self.ctrl.drag_mode.get();
        #[allow(clippy::collapsible_if)]
        if mode == DragMode::Pending {
            if self.ctrl.last_pointer_pos.get().is_some() {
                if pending_content_drag_won {
                    mode = DragMode::Content;
                    self.ctrl.drag_mode.set(DragMode::Content);
                } else {
                    return Flow::Return(child_result);
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
                        let (from, anchored) = self.ctrl.bar.anchor.get();
                        offset.y = anchored
                            - (self.ctrl.to_device(*p).y - from)
                                * self.ctrl.v_scroll_multiplier.get();
                    }
                    DragMode::HorizontalScrollbar => {
                        let (from, anchored) = self.ctrl.bar.anchor.get();
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
                    return Flow::Return(child_result);
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
            return Flow::Return(child_result.merge(EventResult::consumed().with_redraw()));
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
        Flow::Continue(false)
    }
}
