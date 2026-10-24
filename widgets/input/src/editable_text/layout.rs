use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::font::{FontStyle, TextLanguage};
use aimer_cupid::text_layout::{CaretGeometry, SelectionRect, TextInteractionLayout};
use aimer_style::{TextAlign, TextTransform};
use unicode_segmentation::UnicodeSegmentation;

/// The text presented to the renderer and the source byte ranges each
/// displayed grapheme represents.
pub(crate) struct DisplayText {
    pub display: Arc<str>,
    pub source_ranges: Vec<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EditableGeometryKey {
    pub revision: u64,
    pub font_size_bits: u32,
    pub width_bits: u32,
    pub obscure: bool,
    pub wrap: bool,
    pub font_family: aimer_cupid::font::FontFamily,
    pub font_style: FontStyle,
    pub font_weight: u16,
    pub text_transform: TextTransform,
    pub letter_spacing_bits: u32,
    pub word_spacing_bits: u32,
    pub language: Option<TextLanguage>,
}

pub(crate) struct EditableGeometry {
    pub display: Arc<str>,
    pub text_width: f32,
    pub visual_lines: Vec<VisualLine>,
    pub interaction: Option<Rc<TextInteractionLayout>>,
    pub source_ranges: Vec<Range<usize>>,
    line_offsets: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VisualLine {
    pub byte_start: usize,
    pub byte_end: usize,
    pub grapheme_start: usize,
    pub grapheme_end: usize,
    pub width: f32,
}

impl EditableGeometry {
    /// Creates field geometry around the source-aware layout produced by the
    /// text renderer. `source_ranges` maps each displayed grapheme to the
    /// corresponding UTF-8 range in the controller's text; it is different
    /// from the display text for secure fields and while an IME preedit is
    /// removed from the committed presentation.
    pub(crate) fn from_interaction(
        display: Arc<str>,
        source_ranges: Vec<Range<usize>>,
        mut interaction: TextInteractionLayout,
        content_width: f32,
        text_align: TextAlign,
    ) -> Self {
        let line_offsets = interaction
            .lines
            .iter()
            .map(|line| horizontal_alignment_offset(text_align, content_width, line.width))
            .collect::<Vec<_>>();
        for cluster in &mut interaction.clusters {
            if let Some(offset) = line_offsets.get(cluster.line_index) {
                cluster.start_x += *offset;
                cluster.end_x += *offset;
            }
        }
        if interaction.text.is_empty() {
            interaction.origin_x += line_offsets.first().copied().unwrap_or(0.0);
        }
        // Paragraph-backed layouts use zero as the line-top origin while the
        // shared selection geometry treats `origin_y` as the first baseline.
        // Normalize both paths before the layout is retained so containment,
        // caret handles, and the field's own hit testing agree on the same
        // line box.
        if let Some(baseline) = interaction.lines.first().map(|line| line.baseline) {
            interaction.origin_y = baseline;
        }
        // Selection geometry uses the layout's width for hard-break hit areas
        // and for the ambient participant's containment check. The field owns
        // the whole content viewport even when its current line is shorter.
        interaction.metrics.width = interaction.metrics.width.max(content_width);

        let text_width = interaction
            .lines
            .iter()
            .map(|line| line.width)
            .fold(0.0, f32::max);
        let visual_lines = visual_lines_from_interaction(&display, &interaction);
        Self {
            display,
            text_width,
            visual_lines,
            interaction: Some(Rc::new(interaction)),
            source_ranges,
            line_offsets,
        }
    }

    /// Converts a displayed grapheme offset into the byte offset consumed by
    /// the canonical interaction layout.
    #[inline]
    pub(crate) fn display_byte_offset(&self, offset: usize) -> usize {
        self.display
            .grapheme_indices(true)
            .nth(offset)
            .map_or(self.display.len(), |(byte, _)| byte)
    }

    /// Returns the shaped caret for a displayed grapheme offset. If the
    /// shaper fused adjacent graphemes into a ligature, the nearest legal
    /// cluster edge is used so the caret never cuts through the painted glyph.
    pub(crate) fn caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        let layout = self.interaction.as_ref()?;
        let byte = self.display_byte_offset(offset);

        // The canonical shaper gives a hard-break cluster to the line it
        // terminates. When the cursor is immediately after a trailing break,
        // that cluster's end is also the start of the following empty line;
        // prefer the empty line so a TextArea keeps the insertion point where
        // the next character will be entered.
        if let Some((line_index, next_line)) = layout
            .lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.hard_break && line.text_range.end == byte)
            .and_then(|(line_index, _)| {
                layout
                    .lines
                    .get(line_index + 1)
                    .map(|line| (line_index, line))
            })
            && !layout
                .clusters
                .iter()
                .any(|cluster| cluster.text_range.start == byte)
        {
            return Some(CaretGeometry {
                offset: byte,
                line_index: line_index + 1,
                x: layout.origin_x
                    + self
                        .line_offsets
                        .get(line_index + 1)
                        .copied()
                        .unwrap_or(0.0),
                y: next_line.baseline - next_line.ascent - next_line.line_gap * 0.5,
                width: 0.0,
                height: (next_line.ascent - next_line.descent + next_line.line_gap).max(0.0),
            });
        }

        if let Some(caret) = layout.caret_geometry(byte) {
            return Some(caret);
        }
        let cluster = layout
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.start < byte && byte < cluster.text_range.end)?;
        let edge = if byte - cluster.text_range.start <= cluster.text_range.end - byte {
            cluster.text_range.start
        } else {
            cluster.text_range.end
        };
        layout.caret_geometry(edge)
    }

    /// Maps a content-local point through the canonical shaped layout.
    #[inline]
    pub(crate) fn hit_test(&self, x: f32, y: f32) -> Option<usize> {
        let layout = self.interaction.as_ref()?;
        let byte = layout.hit_test(x, y)?;
        Some(grapheme_count(
            &self.display[..byte.min(self.display.len())],
        ))
    }

    /// Returns canonical selection rectangles for displayed grapheme offsets.
    #[inline]
    pub(crate) fn selection_rects(&self, start: usize, end: usize) -> Vec<SelectionRect> {
        let Some(layout) = self.interaction.as_ref() else {
            return Vec::new();
        };
        layout.selection_rects(self.display_byte_offset(start)..self.display_byte_offset(end))
    }

    /// Maps a displayed grapheme range to its source byte range.
    pub(crate) fn source_range_for_display_range(
        &self,
        start: usize,
        end: usize,
    ) -> Option<Range<usize>> {
        let start = start.min(self.source_ranges.len());
        let end = end.min(self.source_ranges.len());
        if start >= end {
            return None;
        }
        let first = self.source_ranges.get(start)?;
        let last = self.source_ranges.get(end - 1)?;
        Some(first.start.min(last.start)..first.end.max(last.end))
    }

    /// Maps one canonical display cluster to the controller's source range.
    pub(crate) fn source_range_for_display_bytes(
        &self,
        range: &Range<usize>,
    ) -> Option<Range<usize>> {
        let start = grapheme_count(&self.display[..range.start.min(self.display.len())]);
        let end = grapheme_count(&self.display[..range.end.min(self.display.len())]);
        self.source_range_for_display_range(start, end)
    }

    /// Returns the visual line containing an editing cursor offset.
    #[inline]
    pub(crate) fn line_for_offset(&self, offset: usize) -> usize {
        self.caret_geometry(offset).map_or_else(
            || self.visual_lines.len().saturating_sub(1),
            |caret| caret.line_index,
        )
    }
}

#[inline]
fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

fn horizontal_alignment_offset(align: TextAlign, content_width: f32, line_width: f32) -> f32 {
    let remaining = (content_width - line_width).max(0.0);
    match align {
        TextAlign::TopCenter | TextAlign::MidCenter | TextAlign::BotCenter => remaining / 2.0,
        TextAlign::TopRight | TextAlign::MidRight | TextAlign::BotRight => remaining,
        _ => 0.0,
    }
}

fn visual_lines_from_interaction(
    display: &str,
    interaction: &TextInteractionLayout,
) -> Vec<VisualLine> {
    let mut lines = interaction
        .lines
        .iter()
        .map(|line| {
            let byte_start = line.text_range.start.min(display.len());
            let mut byte_end = line.text_range.end.min(display.len());
            if line.hard_break {
                if display[byte_start..byte_end].ends_with("\r\n") {
                    byte_end = byte_end.saturating_sub(2);
                } else if display[byte_start..byte_end].ends_with('\n') {
                    byte_end = byte_end.saturating_sub(1);
                }
            }
            VisualLine {
                byte_start,
                byte_end,
                grapheme_start: grapheme_count(&display[..byte_start]),
                grapheme_end: grapheme_count(&display[..byte_end]),
                width: line.width,
            }
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push(VisualLine {
            byte_start: 0,
            byte_end: 0,
            grapheme_start: 0,
            grapheme_end: 0,
            width: 0.0,
        });
    }
    lines
}

pub(crate) fn wrap_visual_lines(
    text: &str,
    max_width: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> Vec<VisualLine> {
    let mut lines = Vec::new();
    let mut byte_start = 0;
    let mut grapheme_start = 0;
    let mut byte_end = 0;
    let mut grapheme_end = 0;
    let mut width = 0.0;

    for (byte, grapheme) in text.grapheme_indices(true) {
        if grapheme == "\n" {
            lines.push(VisualLine {
                byte_start,
                byte_end: byte,
                grapheme_start,
                grapheme_end,
                width,
            });
            byte_start = byte + grapheme.len();
            grapheme_start = grapheme_end + 1;
            byte_end = byte_start;
            grapheme_end = grapheme_start;
            width = 0.0;
            continue;
        }

        let grapheme_width = measure(grapheme);
        if width > 0.0 && width + grapheme_width > max_width {
            lines.push(VisualLine {
                byte_start,
                byte_end,
                grapheme_start,
                grapheme_end,
                width,
            });
            byte_start = byte;
            grapheme_start = grapheme_end;
            width = 0.0;
        }
        width += grapheme_width;
        byte_end = byte + grapheme.len();
        grapheme_end += 1;
    }
    lines.push(VisualLine {
        byte_start,
        byte_end,
        grapheme_start,
        grapheme_end,
        width,
    });
    lines
}

pub(crate) fn vertical_target(lines: &[VisualLine], current: usize, direction: isize) -> usize {
    let Some(line_index) = lines
        .iter()
        .position(|line| current >= line.grapheme_start && current <= line.grapheme_end)
    else {
        return current;
    };
    let target_index = line_index.saturating_add_signed(direction);
    let Some(target) = lines.get(target_index) else {
        return current;
    };
    let column = current.saturating_sub(lines[line_index].grapheme_start);
    target.grapheme_start + column.min(target.grapheme_end - target.grapheme_start)
}

#[derive(Default)]
pub(crate) struct EditableGeometryCache {
    cached: RefCell<Option<(EditableGeometryKey, Rc<EditableGeometry>)>>,
}

impl EditableGeometryCache {
    pub(crate) fn resolve(
        &self,
        key: EditableGeometryKey,
        build: impl FnOnce() -> EditableGeometry,
    ) -> Rc<EditableGeometry> {
        if let Some((cached_key, geometry)) = self.cached.borrow().as_ref()
            && *cached_key == key
        {
            return geometry.clone();
        }

        let geometry = Rc::new(build());
        self.cached.replace(Some((key, geometry.clone())));
        geometry
    }

    pub(crate) fn latest(&self) -> Option<Rc<EditableGeometry>> {
        self.cached
            .borrow()
            .as_ref()
            .map(|(_, geometry)| geometry.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::{EditableGeometry, EditableGeometryCache, EditableGeometryKey};
    use super::{vertical_target, wrap_visual_lines};

    #[test]
    fn unchanged_geometry_is_reused_and_revisions_invalidate_it() {
        let cache = EditableGeometryCache::default();
        let builds = Cell::new(0);
        let key = EditableGeometryKey {
            revision: 3,
            font_size_bits: 14.0f32.to_bits(),
            width_bits: 200.0f32.to_bits(),
            obscure: false,
            wrap: false,
            font_family: aimer_cupid::font::FontFamily::SANS_SERIF,
            font_style: aimer_cupid::font::FontStyle::Normal,
            font_weight: 400,
            text_transform: aimer_style::TextTransform::None,
            letter_spacing_bits: 0,
            word_spacing_bits: 0,
            language: None,
        };
        let build = || {
            builds.set(builds.get() + 1);
            EditableGeometry {
                display: Arc::from("hello"),
                text_width: 40.0,
                visual_lines: Vec::new(),
                interaction: None,
                source_ranges: Vec::new(),
                line_offsets: Vec::new(),
            }
        };

        let first = cache.resolve(key, build);
        let second = cache.resolve(key, build);
        let changed = cache.resolve(EditableGeometryKey { revision: 4, ..key }, build);

        assert!(std::rc::Rc::ptr_eq(&first, &second));
        assert!(!std::rc::Rc::ptr_eq(&second, &changed));
        assert_eq!(builds.get(), 2);
    }

    #[test]
    fn visual_lines_wrap_softly_preserve_hard_breaks_and_move_vertically() {
        let lines = wrap_visual_lines("abcd\nef", 2.0, |_| 1.0);

        assert_eq!(
            lines
                .iter()
                .map(|line| (line.grapheme_start, line.grapheme_end))
                .collect::<Vec<_>>(),
            vec![(0, 2), (2, 4), (5, 7)]
        );
        assert_eq!(vertical_target(&lines, 3, -1), 1);
        assert_eq!(vertical_target(&lines, 1, 1), 3);
        assert_eq!(vertical_target(&lines, 3, 1), 6);
    }
}
