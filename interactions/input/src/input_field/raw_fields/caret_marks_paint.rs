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
const BOX_FILL: f32 = 0.32;
/// How much of its color the line of a box has, out of `1.0`.
const BOX_LINE: f32 = 1.0;
/// How far a box is from its range, on each side, in logical pixels.
const BOX_SPREAD: f32 = 1.0;
/// How round the corners of a box are, in logical pixels.
const BOX_RADIUS: f32 = 3.0;
/// How thick the line of a box is, in logical pixels.
const BOX_STROKE: f32 = 1.0;
/// What progress of the mark pulse the transition to settled opacity starts at.
const TRANSITION_START: f32 = 0.6;
/// The opacity of the mark box after it settles, out of `1.0`.
const SETTLED_OPACITY: f32 = 0.3;

/// Contrasting ink color for the scaled glyph drawn inside a solid mark pill (dark ink on bright
/// highlight colors, crisp white on dark colors), with `alpha` applied.
fn contrast_ink(red: u8, green: u8, blue: u8, alpha: u8) -> aimer_cupid::utilities::Color {
    let luma = 0.299 * (red as f32) + 0.587 * (green as f32) + 0.114 * (blue as f32);
    if luma >= 140.0 {
        aimer_cupid::utilities::Color::rgba8(18, 18, 22, alpha)
    } else {
        aimer_cupid::utilities::Color::rgba8(255, 255, 255, alpha)
    }
}

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
        let edited = self
            .marks_value
            .borrow()
            .as_ref()
            .is_some_and(|prev| !value.same_text(prev));
        *self.marks_value.borrow_mut() = Some(value.clone());
        if edited {
            self.marks_began.set(None);
            *self.marks_seen.borrow_mut() = ranges;
        } else if *self.marks_seen.borrow() != ranges {
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

    /// Draws a rounded box round each of `marks`, in its color at settled opacity, behind the text.
    /// Only drawn when the mark pulse is not active (e.g. after animation ends, or while typing).
    fn paint_mark_boxes(&self, canvas: &Canvas, geometry: &EditableGeometry, scale: f32, origin: Vec2d, content: &Rect, marks: &[CaretMark]) {
        if self.marks_phase().is_some() {
            return;
        }
        for mark in marks {
            let Some((left, top, right, bottom)) = self.range_box(geometry, mark.range.start, mark.range.end) else {
                continue;
            };
            let (red, green, blue, alpha) = mark.color.to_rgba();
            let tint = |share: f32| aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * share * SETTLED_OPACITY).round() as u8);
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

    /// Draws the mark over each of `marks`, closing on it as a solid pop-and-shrink pill with the
    /// marked glyph scaled to match the pulsing box, transitioning to settled opacity starting at
    /// 60% progress.
    #[allow(clippy::too_many_arguments)]
    fn paint_mark_pulses(
        &self,
        canvas: &Canvas,
        ctx: &BuildContext,
        geometry: &EditableGeometry,
        style: &TextStyle,
        scale: f32,
        origin: Vec2d,
        content: &Rect,
        marks: &[CaretMark],
    ) {
        let Some(phase) = self.marks_phase() else {
            return;
        };
        aimer_events::window::request_animation_frame();
        const GROW: f32 = 14.0;
        let fade = if phase < TRANSITION_START {
            0.0
        } else {
            ((phase - TRANSITION_START) / (1.0 - TRANSITION_START)).clamp(0.0, 1.0)
        };
        let border_opacity = 1.0 - (1.0 - SETTLED_OPACITY) * fade;
        // Solid opaque pill while popped out so the underlying unscaled glyph does not show
        // through, smoothly dissolving to the settled translucent fill as the pill lands.
        let fill_opacity = 1.0 - (1.0 - BOX_FILL * SETTLED_OPACITY) * fade;
        let glyph_opacity = 1.0 - fade;
        let eased = 1.0 - (1.0 - phase).powi(3);
        let spread = BOX_SPREAD + GROW * (1.0 - eased);
        let radius = BOX_RADIUS + (spread - BOX_SPREAD) * 0.5;
        let base_font_size = self.scaled_font_size(style, scale) / scale;
        let value = self.controller.value();

        struct PulseItem<'a> {
            mark: &'a CaretMark,
            cell_y: f32,
            cell_w: f32,
            box_rect: Rect,
            glyph_scale: f32,
        }

        let mut items: Vec<PulseItem<'_>> = Vec::with_capacity(marks.len());
        for mark in marks {
            let Some((left, top, right, bottom)) = self.range_box(geometry, mark.range.start, mark.range.end) else {
                continue;
            };
            let cell_w = (right - left) / scale;
            let cell_h = (bottom - top) / scale;
            let cell_y = origin.y + top / scale;
            let box_rect = within(
                Rect::new(
                    origin.x + left / scale - spread,
                    cell_y - spread,
                    cell_w + 2.0 * spread,
                    cell_h + 2.0 * spread,
                ),
                content,
            );
            let settled_h = (cell_h + 2.0 * BOX_SPREAD).max(1.0);
            let glyph_scale = (box_rect.height / settled_h).max(1.0);
            items.push(PulseItem { mark, cell_y, cell_w, box_rect, glyph_scale });
        }

        // When two marks share a line and sit close together (e.g. `{}` or `()`), nudge their
        // pulsing pills apart horizontally so the scaled glyphs sit cleanly side-by-side and slide
        // smoothly back to their resting cells as `glyph_scale` returns to `1.0`.
        for i in 0..items.len() {
            for j in (i + 1)..items.len() {
                if (items[i].cell_y - items[j].cell_y).abs() >= 1.0 {
                    continue;
                }
                let cx_i = items[i].box_rect.x + items[i].box_rect.width * 0.5;
                let cx_j = items[j].box_rect.x + items[j].box_rect.width * 0.5;
                let min_gap = 0.5 * (items[i].cell_w * items[i].glyph_scale + items[j].cell_w * items[j].glyph_scale);
                let dx = (cx_j - cx_i).abs();
                if dx < min_gap {
                    let push = (min_gap - dx) * 0.5;
                    let (left_idx, right_idx) = if cx_i <= cx_j { (i, j) } else { (j, i) };
                    items[left_idx].box_rect.x -= push;
                    items[left_idx].box_rect = within(items[left_idx].box_rect, content);
                    items[right_idx].box_rect.x += push;
                    items[right_idx].box_rect = within(items[right_idx].box_rect, content);
                }
            }
        }

        // Pass 1: draw all pulsing pills first so no pill paints over a neighbouring brace glyph.
        for item in &items {
            let (red, green, blue, alpha) = item.mark.color.to_rgba();
            let line = aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * BOX_LINE * border_opacity).round() as u8);
            let fill = aimer_cupid::utilities::Color::rgba8(red, green, blue, (alpha as f32 * fill_opacity).round() as u8);
            canvas.fill_rect_with_border(item.box_rect, fill, [radius; 4], BOX_STROKE, line);
        }

        // Pass 2: draw all scaled brace glyphs on top of all pills.
        let weight = style.font_weight.numeric().max(600);
        for item in &items {
            let (red, green, blue, alpha) = item.mark.color.to_rgba();
            let glyph_alpha = (alpha as f32 * glyph_opacity).round() as u8;
            if glyph_alpha == 0 {
                continue;
            }
            let start = value.floor_to_boundary(item.mark.range.start);
            let end = value.ceil_to_boundary(item.mark.range.end);
            if start >= end {
                continue;
            }
            let slice = value.slice(start..end);
            if slice.is_empty() || slice.contains('\n') {
                continue;
            }
            let scaled_font_size = base_font_size * item.glyph_scale;
            // `DrawCommand::DrawText` positions text by its baseline Y. Center the
            // glyph's ascent..descent box vertically inside `box_rect`.
            let metrics = ctx.canvas.measure_text_metrics_styled(
                slice.as_ref(),
                scaled_font_size * scale,
                0.0,
                style.font_family,
                style.font_style,
                weight,
            );
            let baseline_offset = if metrics.ascent > 0.0 {
                (metrics.ascent + metrics.descent) * 0.5 / scale
            } else {
                scaled_font_size * 0.35
            };
            let baseline_y = item.box_rect.y + item.box_rect.height * 0.5 + baseline_offset;
            canvas.draw_text_styled(
                std::sync::Arc::<str>::from(slice.as_ref()),
                aimer_cupid::utilities::Vec2d::new(item.box_rect.x, baseline_y),
                scaled_font_size,
                contrast_ink(red, green, blue, glyph_alpha),
                Some(item.box_rect.width),
                Some(item.box_rect.height),
                aimer_cupid::text_pipeline::TextOverflowMode::Clip,
                aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Center,
                style.font_family,
                style.font_style,
                weight,
                None,
                true,
            );
        }
    }
}
