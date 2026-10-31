//! The mouse's side of the scroll bar: what a click and a drag on its strip do.
//!
//! The bar strip is wrapped in a `GestureDetector`, so a press is recognised as
//! a tap or a drag by the same slop and release rules as any other widget, and
//! what that means for the bar is decided here, in one place:
//!
//! * a tap on the track glides to that spot, the thumb centring on the click;
//! * a drag that starts on the thumb carries it, staying under the pointer;
//! * a drag that starts on the track first jumps the thumb there, then carries it.
//!
//! Touch does not come through here. A finger gets a generous margin around the
//! thumb that reaches beyond the strip, which a detector bounded by the strip
//! cannot see, so touch stays with the container.

use aimer_attribute::position::Vec2d;
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_input::gesture::GestureEvent;

use crate::ScrollAxis;
use crate::scrollable::controller::{DragMode, ScrollState};
use crate::scrollable::overscroll_source::OverscrollSource;

/// What a pointer position is over on the active scroll bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarHit {
    Thumb,
    Track,
}

/// The active bar's thumb along its axis, in viewport-relative device pixels.
struct BarAxis {
    vertical: bool,
    thumb_start: f32,
    thumb_length: f32,
    multiplier: f32,
}

impl ScrollState {
    fn bar_axis(&self) -> Option<BarAxis> {
        let vertical = matches!(self.axis, ScrollAxis::Vertical);
        let (start, length, multiplier) = if vertical {
            let (_, y, _, h) = self.v_thumb_rect.get()?;
            (y, h, self.v_scroll_multiplier.get())
        } else {
            let (x, _, w, _) = self.h_thumb_rect.get()?;
            (x, w, self.h_scroll_multiplier.get())
        };
        Some(BarAxis {
            vertical,
            thumb_start: start,
            thumb_length: length,
            multiplier,
        })
    }

    /// Whether `p` (window logical pixels) is on the active bar's thumb or track.
    pub(crate) fn bar_hit(&self, p: Vec2d) -> Option<BarHit> {
        let (viewport_w, viewport_h) = self.cached_viewport.get();
        let vertical = matches!(self.axis, ScrollAxis::Vertical);
        let (thumb, track) = if vertical {
            (
                self.hit_test_v_thumb(p),
                self.hit_test_v_track(p, viewport_w, viewport_h, self.cached_v_track_width.get()),
            )
        } else {
            (
                self.hit_test_h_thumb(p),
                self.hit_test_h_track(p, viewport_w, viewport_h, self.cached_h_track_width.get()),
            )
        };
        if thumb {
            Some(BarHit::Thumb)
        } else if track {
            Some(BarHit::Track)
        } else {
            None
        }
    }

    /// The internal offset that puts the thumb's centre on `p` along the bar,
    /// not yet clamped to the content.
    fn thumb_centred_offset(&self, p: Vec2d) -> Option<Vec2d> {
        let bar = self.bar_axis()?;
        let device = self.to_device(p);
        let along = if bar.vertical { device.y } else { device.x };
        let mut offset = self.scroll_offset.get();
        let shift = (along - bar.thumb_length / 2.0 - bar.thumb_start) * bar.multiplier;
        if bar.vertical {
            offset.y -= shift;
        } else {
            offset.x -= shift;
        }
        Some(offset)
    }

    /// Glides to the spot `p` on the track, the thumb ending up centred there.
    fn scroll_smoothly_to(&self, p: Vec2d) {
        let Some(target) = self.thumb_centred_offset(p) else {
            return;
        };
        // The content cannot travel past its ends, so neither does the push.
        let target = self.clamp_offset(target);
        let current = self.scroll_offset.get();
        self.glide_by(
            Vec2d {
                x: target.x - current.x,
                y: target.y - current.y,
            },
            false,
        );
    }

    /// Scrolls so the thumb is centred on `p` at once, clamped to the content.
    /// Starts a scroll session and leaves it open for the caller.
    ///
    /// A drag that starts on the track has to have the thumb under the pointer
    /// from its first update, so it cannot wait for an animation.
    fn centre_thumb_on(&self, p: Vec2d) {
        let Some(offset) = self.thumb_centred_offset(p) else {
            return;
        };
        // A click is a deliberate jump: nothing that was in flight carries on.
        self.cancel_fling();
        self.pointer_velocity.set(Vec2d::ZERO);
        self.spring_velocity.set(Vec2d::ZERO);
        self.release_overscroll_recovery();
        self.reset_overscroll_peak();
        self.begin_scroll();
        self.set_scroll_offset(self.clamp_offset(offset));
        self.record_input_event();
        self.request_animation_frame();
    }

    fn begin_bar_drag(&self, p: Vec2d) {
        let Some(hit) = self.bar_hit(p) else {
            return;
        };
        if hit == BarHit::Track {
            self.centre_thumb_on(p);
        }
        let Some(bar) = self.bar_axis() else {
            return;
        };
        let device = self.to_device(p);
        let offset = self.scroll_offset.get();
        // The thumb is measured from where it was taken, so it stays under the
        // pointer however the drag goes.
        self.scroll_bar_anchor.set(if bar.vertical {
            (device.y, offset.y)
        } else {
            (device.x, offset.x)
        });
        self.drag_mode.set(if bar.vertical {
            DragMode::VerticalScrollbar
        } else {
            DragMode::HorizontalScrollbar
        });
        // A bar drag is a mouse interaction: it takes over from any wheel or
        // trackpad gesture instead of inheriting its recovery state.
        self.set_overscroll_source(OverscrollSource::Mouse);
        self.release_overscroll_recovery();
        self.begin_device_contact(false);
        self.reset_overscroll_peak();
        self.cancel_fling();
        self.pointer_velocity.set(Vec2d::ZERO);
        self.begin_scroll();
        self.record_input_event();
        self.request_animation_frame();
    }

    fn update_bar_drag(&self, p: Vec2d) {
        let mode = self.drag_mode.get();
        if !matches!(
            mode,
            DragMode::VerticalScrollbar | DragMode::HorizontalScrollbar
        ) {
            return;
        }
        let (from, anchored) = self.scroll_bar_anchor.get();
        let device = self.to_device(p);
        let mut offset = self.scroll_offset.get();
        if mode == DragMode::VerticalScrollbar {
            offset.y = anchored - (device.y - from) * self.v_scroll_multiplier.get();
        } else {
            offset.x = anchored - (device.x - from) * self.h_scroll_multiplier.get();
        }
        // The bar ends where the content does.
        self.set_scroll_offset(self.clamp_offset(offset));
        self.record_input_event();
        self.request_animation_frame();
    }

    fn end_bar_drag(&self) {
        if !matches!(
            self.drag_mode.get(),
            DragMode::VerticalScrollbar | DragMode::HorizontalScrollbar
        ) {
            return;
        }
        self.drag_mode.set(DragMode::None);
        self.release_overscroll_recovery();
        self.begin_device_contact(false);
        self.last_frame_time.set(Some(aimer_utils::AnimInstant::now()));
        self.request_animation_frame();
    }

    /// Reacts to one gesture recognised on the bar strip.
    pub(crate) fn on_bar_gesture(&self, event: GestureEvent) {
        match event {
            GestureEvent::Tap { pointer }
                if pointer.source == PointerSource::Mouse
                    && pointer.button == PointerButton::Primary =>
            {
                if self.bar_hit(pointer.pos) == Some(BarHit::Track) {
                    self.scroll_smoothly_to(pointer.pos);
                }
            }
            GestureEvent::DragStart { pointer } if pointer.source == PointerSource::Mouse => {
                self.begin_bar_drag(pointer.pos);
            }
            GestureEvent::DragUpdate { pointer, .. } if pointer.source == PointerSource::Mouse => {
                self.update_bar_drag(pointer.pos);
            }
            GestureEvent::DragEnd { .. } | GestureEvent::DragCancel => self.end_bar_drag(),
            _ => {}
        }
    }
}
