//! Scroll bar hit testing against the thumb and track rectangles the bar
//! publishes.

use std::cell::Cell;

use aimer_attribute::position::Vec2d;

use super::controller::*;
use aimer_input::mouse_region::{PointerState, SharedPointerState};
use crate::scrollable::constants::*;

/// Scroll bar state published by the bar and read by input.
#[derive(Default)]
pub struct BarState {
    /// How a finger's hold on the thumb is progressing (see
    /// [`ThumbGrab`](super::thumb_grab::ThumbGrab)).
    pub(crate) grab: Cell<super::thumb_grab::ThumbGrab>,
    pub(crate) v_thumb_rect: Cell<Option<(f32, f32, f32, f32)>>,
    pub(crate) h_thumb_rect: Cell<Option<(f32, f32, f32, f32)>>,
    /// Whether the cursor was over a scrollbar thumb after the last pointer
    /// move, so the move that crosses a thumb edge — and only that move — can
    /// schedule the frame that repaints the highlight (see
    /// [`sync_thumb_hover`](ScrollState::sync_thumb_hover)).
    pub(crate) thumb_hovered: Cell<bool>,
    /// How opaque the scroll bar was painted last, from `0.0` (hidden) to `1.0`.
    ///
    /// Published by the bar as it paints. Touch reads it so a finger cannot grab
    /// a thumb that is not on screen.
    pub(crate) opacity: Cell<f32>,
    /// Where the viewport's top-left corner is in the window, in logical pixels.
    ///
    /// The bar's geometry is published relative to the viewport while pointers
    /// arrive in window coordinates, so every hit test measures from here.
    /// Kept current by the container from its own bounds.
    pub(crate) viewport_origin: Cell<Vec2d>,
    /// Whether a mouse is over the scroll bar strip, which keeps the bar showing
    /// for as long as it stays there.
    ///
    /// The cell is the one the bar's [`MouseRegion`](aimer_input::mouse_region::MouseRegion)
    /// writes, so hover is decided by the same bounds and edge detection as any
    /// other region instead of by a second hit test of the container's own.
    pub(crate) pointer: SharedPointerState,
    /// Where a thumb drag took hold: the pointer's position along the bar and
    /// the scroll offset at that moment. The drag is measured from these, not
    /// accumulated, so the thumb stays under the pointer for the whole drag.
    pub(crate) anchor: Cell<(f32, f32)>,
    pub(crate) v_track_width: Cell<f32>,
    pub(crate) h_track_width: Cell<f32>,
}

impl ScrollState {
    /// A pointer position, which arrives in window logical pixels, in the
    /// viewport-relative device pixels the bar's geometry is laid out in.
    #[inline]
    pub(crate) fn to_device(&self, p: Vec2d) -> Vec2d {
        let scale = self.last_scale.get();
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let origin = self.bar.viewport_origin.get();
        Vec2d {
            x: (p.x - origin.x) * scale,
            y: (p.y - origin.y) * scale,
        }
    }

    /// Check if a point, in logical pixels, is inside the vertical thumb rect.
    pub(crate) fn hit_test_v_thumb(&self, p: Vec2d) -> bool {
        let p = self.to_device(p);
        if let Some((x, y, w, h)) = self.bar.v_thumb_rect.get() {
            p.x >= x && p.x <= x + w && p.y >= y && p.y <= y + h
        } else {
            false
        }
    }

    /// Check if a point is inside the horizontal thumb rect.
    pub(crate) fn hit_test_h_thumb(&self, p: Vec2d) -> bool {
        let p = self.to_device(p);
        if let Some((x, y, w, h)) = self.bar.h_thumb_rect.get() {
            p.x >= x && p.x <= x + w && p.y >= y && p.y <= y + h
        } else {
            false
        }
    }

    /// Which thumb a pointer at `p` takes hold of, if any.
    ///
    /// A mouse has to be on the thumb itself. A finger gets a generous margin
    /// around it.
    ///
    /// The margin is there whether or not the bar is showing, because reaching
    /// for a thumb that has faded out is how it is brought back. Taking hold of
    /// it is another matter: that waits until the bar is on screen (see
    /// [`poll_thumb_hold`](Self::poll_thumb_hold)).
    pub(crate) fn thumb_at(&self, p: Vec2d, touch: bool) -> Option<DragMode> {
        if !touch {
            return if self.hit_test_v_thumb(p) {
                Some(DragMode::VerticalScrollbar)
            } else if self.hit_test_h_thumb(p) {
                Some(DragMode::HorizontalScrollbar)
            } else {
                None
            };
        }
        let p = self.to_device(p);
        let scale = self.last_scale.get().max(1.0);
        let (cross, along) = (
            SCROLLBAR_TOUCH_SLOP_CROSS * scale,
            SCROLLBAR_TOUCH_SLOP_ALONG * scale,
        );
        let within = |rect: Option<(f32, f32, f32, f32)>, cross_x: bool| {
            rect.is_some_and(|rect| {
                touch_zone_overshoot(p, rect, cross_x, cross, along) == Vec2d::ZERO
            })
        };
        if within(self.bar.v_thumb_rect.get(), true) {
            Some(DragMode::VerticalScrollbar)
        } else if within(self.bar.h_thumb_rect.get(), false) {
            Some(DragMode::HorizontalScrollbar)
        } else {
            None
        }
    }

    /// Says in words why a touch at `p` (window logical pixels) is not on the
    /// thumb, for the log: how far outside the grab zone it landed, on which
    /// side, against the margins that apply. Uses the same zone as
    /// [`thumb_at`](Self::thumb_at), so the two cannot disagree.
    pub(crate) fn describe_touch_miss(&self, p: Vec2d) -> String {
        let vertical = matches!(self.axis, crate::scrollable::ScrollAxis::Vertical);
        let rect = if vertical {
            self.bar.v_thumb_rect.get()
        } else {
            self.bar.h_thumb_rect.get()
        };
        let Some(rect) = rect else {
            return "no thumb is published for this axis (the content fits, or the bar has not been laid out)"
                .to_string();
        };
        let scale = self.last_scale.get().max(1.0);
        let device = self.to_device(p);
        let (cross, along) = (
            SCROLLBAR_TOUCH_SLOP_CROSS * scale,
            SCROLLBAR_TOUCH_SLOP_ALONG * scale,
        );
        let o = touch_zone_overshoot(device, rect, vertical, cross, along);
        // Across the bar is x for a vertical thumb and y for a horizontal one.
        let (across, along_by) = if vertical { (o.x, o.y) } else { (o.y, o.x) };
        let (across_words, along_words) = if vertical {
            (("left of", "right of"), ("above", "below"))
        } else {
            (("above", "below"), ("left of", "right of"))
        };
        let side = |overshoot: f32, words: (&str, &str), margin: f32, what: &str| {
            if overshoot == 0.0 {
                format!("inside the zone {what}")
            } else {
                format!(
                    "{:.1} dp {} the zone {what} (zone margin {:.0} dp)",
                    overshoot.abs() / scale,
                    if overshoot < 0.0 { words.0 } else { words.1 },
                    margin / scale
                )
            }
        };
        format!(
            "{}; {}",
            side(across, across_words, cross, "across the bar"),
            side(along_by, along_words, along, "along it")
        )
    }

    /// Whether a mouse is over the scroll bar strip, as its `MouseRegion`
    /// last decided.
    #[inline]
    pub(crate) fn scroll_bar_hovered(&self) -> bool {
        matches!(self.bar.pointer.get(), PointerState::Inside)
    }

    /// Reconciles the remembered hover state of a mouse at `p` with the thumb,
    /// and reports whether it changed.
    ///
    /// Only the thumb highlight is decided here: whether the bar is showing is
    /// the strip region's business. The highlight is resolved at draw time from
    /// what is stored, and pointer moves no longer buy a frame each, so the move
    /// that carries the cursor across a thumb edge is detected here and the
    /// caller schedules the single frame that follows from it.
    #[inline]
    pub(crate) fn sync_thumb_hover(&self, p: Vec2d) -> bool {
        let hovered = self.hit_test_v_thumb(p) || self.hit_test_h_thumb(p);
        self.bar.thumb_hovered.replace(hovered) != hovered
    }

    /// Forgets that a mouse was over the thumb, for when the cursor has left.
    /// Reports whether it had been.
    #[inline]
    pub(crate) fn clear_thumb_hover(&self) -> bool {
        self.bar.thumb_hovered.replace(false)
    }

    /// Check if a point is inside the vertical scrollbar *track* but outside
    /// the thumb.
    pub(crate) fn hit_test_v_track(
        &self,
        p: Vec2d,
        viewport_w: f32,
        viewport_h: f32,
        track_width: f32,
    ) -> bool {
        let p = self.to_device(p);
        if let Some((_tx, y, _tw, h)) = self.bar.v_thumb_rect.get() {
            let track_left = self
                .scroll_bar_placement
                .cross_offset(viewport_w, track_width);
            let in_track_x = p.x >= track_left && p.x <= track_left + track_width;
            let in_track_y = p.y >= 0.0 && p.y <= viewport_h;
            let on_thumb = p.y >= y && p.y <= y + h;
            in_track_x && in_track_y && !on_thumb
        } else {
            false
        }
    }

    /// Check if a point is inside the horizontal scrollbar *track* but outside
    /// the thumb.
    pub(crate) fn hit_test_h_track(
        &self,
        p: Vec2d,
        viewport_w: f32,
        viewport_h: f32,
        track_width: f32,
    ) -> bool {
        let p = self.to_device(p);
        if let Some((x, _ty, w, _th)) = self.bar.h_thumb_rect.get() {
            let track_top = self
                .scroll_bar_placement
                .cross_offset(viewport_h, track_width);
            let in_track_y = p.y >= track_top && p.y <= track_top + track_width;
            let in_track_x = p.x >= 0.0 && p.x <= viewport_w;
            let on_thumb = p.x >= x && p.x <= x + w;
            in_track_y && in_track_x && !on_thumb
        } else {
            false
        }
    }

}

/// How far `p` lies outside the touch zone around `thumb`, per axis: zero on an
/// axis it is inside, negative short of the zone's start, positive past its end.
///
/// The zone is the thumb grown by `cross` on the sides across the bar and by
/// `along` on the ends. `cross_is_x` is true for a vertical thumb, whose
/// across-the-bar direction is x.
pub(crate) fn touch_zone_overshoot(
    p: Vec2d,
    thumb: (f32, f32, f32, f32),
    cross_is_x: bool,
    cross: f32,
    along: f32,
) -> Vec2d {
    let (x, y, w, h) = thumb;
    let (slop_x, slop_y) = if cross_is_x { (cross, along) } else { (along, cross) };
    let axis = |v: f32, start: f32, len: f32, slop: f32| {
        let (low, high) = (start - slop, start + len + slop);
        if v < low {
            v - low
        } else if v > high {
            v - high
        } else {
            0.0
        }
    };
    Vec2d {
        x: axis(p.x, x, w, slop_x),
        y: axis(p.y, y, h, slop_y),
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::position::Vec2d;

    use super::touch_zone_overshoot;

    /// A vertical thumb: 10 wide, 40 tall, at (100, 50).
    const THUMB: (f32, f32, f32, f32) = (100.0, 50.0, 10.0, 40.0);

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    #[test]
    fn a_point_inside_the_zone_overshoots_nothing() {
        // 20 across (x) and 8 along (y) around the thumb, edges included.
        for p in [at(105.0, 70.0), at(80.0, 42.0), at(130.0, 98.0)] {
            assert_eq!(touch_zone_overshoot(p, THUMB, true, 20.0, 8.0), Vec2d::ZERO, "{p:?}");
        }
    }

    #[test]
    fn a_point_past_the_zone_says_by_how_much_and_on_which_axis() {
        // 45 left of the thumb's left edge: 25 past the 20 margin, across the bar.
        let p = touch_zone_overshoot(at(55.0, 70.0), THUMB, true, 20.0, 8.0);
        assert_eq!(p, at(-25.0, 0.0));
        // 30 below the thumb's bottom (90): 22 past the 8 margin, along the bar.
        let p = touch_zone_overshoot(at(105.0, 120.0), THUMB, true, 20.0, 8.0);
        assert_eq!(p, at(0.0, 22.0));
        // Both at once, on the near corner.
        let p = touch_zone_overshoot(at(140.0, 30.0), THUMB, true, 20.0, 8.0);
        assert_eq!(p, at(10.0, -12.0));
    }

    #[test]
    fn a_horizontal_thumb_swaps_which_margin_is_which() {
        // The same rectangle read as a horizontal thumb: the long margin is
        // vertical, across the bar.
        let p = touch_zone_overshoot(at(105.0, 15.0), THUMB, false, 20.0, 8.0);
        assert_eq!(p, at(0.0, -15.0), "35 above the top edge, 20 allowed");
        let p = touch_zone_overshoot(at(85.0, 70.0), THUMB, false, 20.0, 8.0);
        assert_eq!(p, at(-7.0, 0.0), "15 left of the left edge, 8 allowed");
    }
}
