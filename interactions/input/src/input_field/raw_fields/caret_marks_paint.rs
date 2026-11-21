// The boxes round what the caret is next to: asked of the app when the text or the caret is new, drawn behind the text for as
// long as the caret is there, and marked, closing on the range as they fade, when they are not what was drawn before.

use crate::input_field::caret_marks::{CaretMark, CaretMarks};

/// What the marks were asked for and what they said, kept so that a frame which is not a new text or a new caret asks nothing:
/// a blink and each frame of a mark are frames.
pub(crate) struct MarksAsked {
    revision: u64,
    focused: bool,
    marks: Vec<CaretMark>,
}

/// How much of its color the inside of a box has, out of `1.0`.
const BOX_FILL: f32 = 0.16;
/// How much of its color the line of a box has, out of `1.0`.
const BOX_LINE: f32 = 0.85;
/// How far a box is from its range, on each side, in logical pixels.
const BOX_SPREAD: f32 = 1.0;
/// How round the corners of a box are, in logical pixels.
const BOX_RADIUS: f32 = 3.0;
/// How thick the line of a box is, in logical pixels.
const BOX_STROKE: f32 = 1.0;

/// `rect` cut to `content`, the part of the field that shows text: a box that reaches past the text's edge, as one round a character at
/// the start of a line does, is pulled in and not cut off by the clip, which would take its side away.
fn within(rect: Rect, content: &Rect) -> Rect {
    let left = rect.x.max(content.x);
    let top = rect.y.max(content.y);
    let right = (rect.x + rect.width).min(content.x + content.width);
    let bottom = (rect.y + rect.height).min(content.y + content.height);
    Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0))
}

impl RawTextField {
    /// What is to be boxed for the caret now: nothing for a field that does not have the focus. Begins the mark when what is
    /// boxed is not what was the last time.
    fn current_marks(&self) -> Vec<CaretMark> {
        let Some(marks) = &self.caret_marks else {
            return Vec::new();
        };
        let focused = self.is_focused();
        let revision = self.controller.revision();
        let value = self.controller.value();
        let known = self
            .marks_asked
            .borrow()
            .as_ref()
            .filter(|asked| asked.revision == revision && asked.focused == focused)
            .map(|asked| asked.marks.clone());
        let marks = known.unwrap_or_else(|| {
            let found = if focused { (marks.of_caret)(&value.snapshot(), value.selection().range()) } else { Vec::new() };
            *self.marks_asked.borrow_mut() = Some(MarksAsked { revision, focused, marks: found.clone() });
            found
        });
        // What is past the end of the text is not there, and what has no text has nothing to box.
        let len = value.len();
        let marks: Vec<CaretMark> = marks
            .into_iter()
            .filter_map(|mark| {
                let end = mark.range.end.min(len);
                let start = mark.range.start.min(end);
                (start < end).then(|| CaretMark { range: start..end, color: mark.color })
            })
            .collect();
        let ranges: Vec<Range<usize>> = marks.iter().map(|mark| mark.range.clone()).collect();
        if *self.marks_seen.borrow() != ranges {
            let any = !ranges.is_empty();
            *self.marks_seen.borrow_mut() = ranges;
            self.marks_began.set(any.then(|| self.now()));
            if any {
                aimer_events::window::request_animation_frame();
            }
        }
        marks
    }

    /// How far along the mark is, from `0.0` to `1.0`, or `None` when there is none or it is over.
    pub(super) fn marks_phase(&self) -> Option<f32> {
        let began = self.marks_began.get()?;
        let phase = locate::phase_of(self.now().duration_since(began));
        if phase >= 1.0 {
            self.marks_began.set(None);
            return None;
        }
        Some(phase)
    }

    /// Draws a rounded box round each of `marks`, in its color, behind the text.
    fn paint_mark_boxes(&self, canvas: &Canvas, geometry: &EditableGeometry, scale: f32, origin: Vec2d, content: &Rect, marks: &[CaretMark]) {
        for mark in marks {
            let Some((left, top, right, bottom)) = self.range_box(geometry, mark.range.start, mark.range.end) else {
                continue;
            };
            let (red, green, blue, alpha) = mark.color.to_rgba();
            let tint = |share: f32| aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * share).round() as u8);
            canvas.fill_rect_with_border(
                within(
                    Rect::new(
                        origin.x + left / scale - BOX_SPREAD,
                        origin.y + top / scale - BOX_SPREAD,
                        (right - left) / scale + 2.0 * BOX_SPREAD,
                        (bottom - top) / scale + 2.0 * BOX_SPREAD,
                    ),
                    content,
                ),
                tint(BOX_FILL),
                [BOX_RADIUS; 4],
                BOX_STROKE,
                tint(BOX_LINE),
            );
        }
    }

    /// Draws the mark over each of `marks`, closing on it and fading, and asks for the next frame while it is going.
    fn paint_mark_pulses(&self, canvas: &Canvas, geometry: &EditableGeometry, scale: f32, origin: Vec2d, content: &Rect, marks: &[CaretMark]) {
        let Some(phase) = self.marks_phase() else {
            return;
        };
        aimer_events::window::request_animation_frame();
        for mark in marks {
            if let Some(bounds) = self.range_box(geometry, mark.range.start, mark.range.end) {
                self.paint_zoom(canvas, bounds, scale, origin, content, mark.color, phase);
            }
        }
    }
}
