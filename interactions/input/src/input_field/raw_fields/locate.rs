//! A range that something else pointed at: shown in the middle of the view, and marked for a moment.
//!
//! The request is the controller's ([`TextEditingController::locate`](crate::TextEditingController::locate)), taken by
//! the layout of the next frame, because only the layout knows where a byte is. The mark is a rectangle that starts
//! larger than the range and closes on it as it fades, which draws the eye to a place the view has just jumped to.

use std::ops::Range;
use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_cupid::draw_cmd::Rect;

use super::{Canvas, EditableGeometry, RawTextField, vertical_scroll_extent};

/// How long the mark takes to close on the range and fade.
const DURATION: Duration = Duration::from_millis(650);
/// How much larger than the range the mark starts, on each side, in logical pixels.
const GROW: f32 = 14.0;
/// How far the mark is from the range when it has closed, on each side, in logical pixels.
const SETTLED: f32 = 2.0;
/// How thick the mark's line is, in logical pixels.
const STROKE: f32 = 1.5;
/// How much of the color is inside the mark, as it begins, out of `1.0`.
const FILL: f32 = 0.16;
/// How round the mark's corners are, in logical pixels.
const RADIUS: f32 = 4.0;

/// How far along the mark is, from `0.0` as it begins to `1.0` as it ends, `elapsed` into it.
pub(super) fn phase_of(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / DURATION.as_secs_f32()).clamp(0.0, 1.0)
}

/// How far the mark is from the range, on each side, `phase` of the way: the size it began at, closing quickly at first
/// and settling slowly, which is how something zooms in.
fn spread_at(phase: f32) -> f32 {
    let eased = 1.0 - (1.0 - phase).powi(3);
    SETTLED + GROW * (1.0 - eased)
}

/// How opaque the mark is, `phase` of the way: all of it as it begins and none as it ends.
fn opacity_at(phase: f32) -> f32 {
    (1.0 - phase).clamp(0.0, 1.0)
}

impl RawTextField {
    /// Takes the request to show a range, if there is one, and ends what the jump supersedes. Called first in a frame, before
    /// anything is mirrored from the selection area into the field, which would otherwise put the caret back.
    ///
    /// The jump is the end of the press that asked for it. A force click is a deep press of a finger that is still on the
    /// pad, and a drag that goes on from it would put the caret back under the finger, in a view that has moved. A field in a
    /// selection area has the press in the area too, which mirrors what it selected into the field on every frame.
    pub(super) fn begin_locate(&self) {
        let Some(range) = self.controller.take_locate() else {
            return;
        };
        self.mouse_held.set(None);
        self.drag_origin.set(None);
        self.pending_press.borrow_mut().take();
        self.pending_click.set(None);
        self.shift_press.set(None);
        if let Some(selection) = self.area_participant() {
            if let Some(pointer) = selection.active_pointer() {
                selection.end(pointer);
            }
            selection.set_range(range.start, range.start);
        }
        self.locating.set(Some((range.start, range.end)));
        aimer_events::window::request_animation_frame();
    }

    /// Puts the range that was asked for in the middle of the view, as far as the text lets it, and begins the mark. Called
    /// by the layout of a frame, when what is scrolled to can be measured.
    pub(super) fn resolve_locate(&self, geometry: &EditableGeometry, line_height: f32, content_height: f32, is_multiline: bool) {
        self.begin_locate();
        let Some((start, end)) = self.locating.take() else {
            return;
        };
        if is_multiline {
            let value = self.controller.value();
            let total_height = geometry.content_height(line_height);
            let extent = vertical_scroll_extent(total_height, content_height);
            if extent.is_finite() {
                let (top, bottom) = geometry.reveal_span(value.grapheme_of_byte(start), line_height);
                self.scroll_y.set(((top + bottom) / 2.0 - content_height / 2.0).clamp(0.0, extent));
            }
        }
        self.pulse.set(Some((start, end, self.now())));
        self.reveal_caret.set(true);
        aimer_events::window::request_animation_frame();
    }

    /// How far along the mark is, or `None` when there is none or it is over, which is also when it is forgotten.
    pub(super) fn pulse_phase(&self) -> Option<f32> {
        let (_, _, began) = self.pulse.get()?;
        let phase = phase_of(self.now().duration_since(began));
        if phase >= 1.0 {
            self.pulse.set(None);
            return None;
        }
        Some(phase)
    }

    /// Draws the mark over the range, closing on it and fading, and asks for the next frame while it is going.
    pub(super) fn paint_pulse(&self, canvas: &Canvas, geometry: &EditableGeometry, scale: f32, text_origin: Vec2d, content: &Rect) {
        let (Some(phase), Some((start, end, _))) = (self.pulse_phase(), self.pulse.get()) else {
            return;
        };
        aimer_events::window::request_animation_frame();
        let value = self.controller.value();
        let (start, end) = (start.min(value.len()), end.min(value.len()));
        let Some(bounds) = self.range_box(geometry, start, end) else {
            return;
        };
        let color = self.locate_color.or(self.caret_color.map(Into::into)).unwrap_or(self.text_style.color);
        self.paint_zoom(canvas, bounds, scale, text_origin, content, color, phase);
    }

    /// The box of all of the bytes `start` to `end` of the text, as `(left, top, right, bottom)` in physical pixels from the
    /// text's origin: a name is on one line, and a range that is not is shown by the box round it. `None` where there is
    /// nothing of it to show.
    pub(super) fn range_box(&self, geometry: &EditableGeometry, start: usize, end: usize) -> Option<(f32, f32, f32, f32)> {
        let value = self.controller.value();
        let (start, end) = (start.min(value.len()), end.min(value.len()));
        let rects = geometry.selection_rects(value.grapheme_of_byte(start), value.grapheme_of_byte(end));
        let first = rects.first()?;
        let (mut left, mut top, mut right, mut bottom) = (first.x, first.y, first.x + first.width, first.y + first.height);
        for rect in &rects {
            left = left.min(rect.x);
            top = top.min(rect.y);
            right = right.max(rect.x + rect.width);
            bottom = bottom.max(rect.y + rect.height);
        }
        Some((left, top, right, bottom))
    }

    /// Draws the mark round `bounds`, `phase` of the way through it, in `color`: a line that is larger than the box and closes on
    /// it as it fades, and the same color inside it, so faint that the text shows through.
    ///
    /// The mark is kept inside `content`, the part of the field that shows text: a range at the start of a line is not far enough from the
    /// edge for the mark that begins larger than it, which the clip would cut.
    pub(super) fn paint_zoom(&self, canvas: &Canvas, bounds: (f32, f32, f32, f32), scale: f32, text_origin: Vec2d, content: &Rect, color: aimer_widget::base::Color, phase: f32) {
        let (left, top, right, bottom) = bounds;
        let spread = spread_at(phase);
        let (red, green, blue, alpha) = color.to_rgba();
        let opacity = opacity_at(phase);
        let line = aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * opacity).round() as u8);
        let fill = aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * FILL * opacity).round() as u8);
        canvas.fill_rect_with_border(
            super::within(
                Rect::new(
                    text_origin.x + left / scale - spread,
                    text_origin.y + top / scale - spread,
                    (right - left) / scale + 2.0 * spread,
                    (bottom - top) / scale + 2.0 * spread,
                ),
                content,
            ),
            fill,
            [RADIUS + spread / 2.0; 4],
            STROKE,
            line,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_begins_large_and_opaque_and_ends_settled_and_gone() {
        assert_eq!(phase_of(Duration::ZERO), 0.0);
        assert_eq!(phase_of(DURATION), 1.0);
        assert_eq!(phase_of(DURATION * 9), 1.0, "it does not go on");
        assert_eq!(spread_at(0.0), SETTLED + GROW);
        assert_eq!(spread_at(1.0), SETTLED);
        assert_eq!(opacity_at(0.0), 1.0);
        assert_eq!(opacity_at(1.0), 0.0);
    }

    #[test]
    fn the_mark_closes_fast_at_first_and_settles_slowly_and_fades_steadily() {
        // Closing: the first half of the time covers more than half of the way.
        assert!(spread_at(0.5) - SETTLED < (GROW) / 2.0);
        let mut last = f32::MAX;
        for step in 0..=10 {
            let spread = spread_at(step as f32 / 10.0);
            assert!(spread <= last, "it only closes");
            last = spread;
        }
        assert!((opacity_at(0.25) - 0.75).abs() < 1e-6);
    }
}
