use std::cell::{OnceCell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::font::{FontStyle, TextLanguage};
use aimer_cupid::text_layout::{
    CaretGeometry, SelectionRect, TextCluster, TextInteractionLayout, TextLine,
};
use aimer_style::{TextAlign, TextStyle, TextTransform};
use aimer_text::RawTextWidget;

use crate::editable_text::line_index::LineIndex;
use crate::input_field::inlay::Placed;
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
    /// Identity of the highlighter whose ranges shaped the layout; zero for none.
    pub highlighter: u64,
    /// The lines a cursor-aware highlighter styled for, so the layout is shaped
    /// again when the cursor reaches another line; `usize::MAX` for both when
    /// the highlighter does not look at the cursor or the field has no focus.
    pub active_start: usize,
    pub active_end: usize,
    /// What the embedder asked for and the boxes it was given: the layout holds a
    /// box per embed, so it is shaped again when they differ. Zero for none.
    pub embeds: u64,
    /// The hard lines a windowed layout was made for; `(0, 0)` for a layout of the whole text.
    pub window: (usize, usize),
}

/// A range of the text an embedded widget stands in for, with the size of the
/// box that was reserved for it, in physical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmbedBox {
    pub key: u64,
    pub range: Range<usize>,
    pub width: f32,
    pub height: f32,
}

/// Where an embed's box sits in the laid-out text, in physical pixels from the
/// text's own origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EmbedRect {
    pub key: u64,
    pub x: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

pub(crate) struct EditableGeometry {
    pub display: Arc<str>,
    pub text_width: f32,
    pub visual_lines: Vec<VisualLine>,
    pub interaction: Option<Rc<TextInteractionLayout>>,
    pub source_ranges: Vec<Range<usize>>,
    /// The styled ranges of `display` this layout was shaped with, which the
    /// painter must hand to its text so the glyphs it draws are the glyphs the
    /// caret and selection were measured on.
    pub ranges: Option<Rc<[aimer_text::StyledRange]>>,
    /// The boxes reserved for embedded widgets.
    pub embeds: Vec<EmbedBox>,
    /// The room left in the lines for inlays, with the labels that are drawn in it.
    pub inlays: Rc<[Placed]>,
    /// How many physical pixels a logical one was when the room for inlays was made.
    inlay_scale: f32,
    line_offsets: Vec<f32>,
    /// Where every [`GRAPHEME_STRIDE`]th grapheme of `display` starts, made the first
    /// time an offset has to be converted. See [`Self::display_byte_offset`].
    grapheme_checkpoints: OnceCell<Vec<usize>>,
    /// How many of the layout's clusters, from the first, run in order. See
    /// [`Self::cluster_starting_at`].
    ordered_clusters: OnceCell<usize>,
    /// The text widget this layout was measured with, and the style and alignment it
    /// was made for. See [`Self::display_widget_for`].
    display_widget: Option<(TextStyle, TextAlign, Rc<RawTextWidget>)>,
    /// Set when this geometry is of some of the lines of a long text and not of all of
    /// it: see [`WindowInfo`].
    pub window: Option<WindowInfo>,
}

/// What a windowed geometry knows of the text around the lines it was laid out for.
///
/// Such a geometry's `display`, `interaction` and `visual_lines` are of the window's
/// lines alone, but everything it answers about an offset or a height is in the
/// coordinates of the whole text: the grapheme offsets are those of the whole text and
/// the heights are those of the lines stacked, those not laid out at their estimates.
#[derive(Clone)]
pub(crate) struct WindowInfo {
    pub index: Rc<LineIndex>,
    /// The hard lines laid out.
    pub lines: Range<usize>,
    /// Where the window's text starts in the whole text, in bytes and in graphemes.
    pub byte_base: usize,
    pub grapheme_base: usize,
    /// How many graphemes the window holds.
    pub grapheme_len: usize,
    /// The height of everything above the window.
    pub y_base: f32,
}

impl WindowInfo {
    /// Whether the caret at grapheme offset `offset` is in a line that was laid out.
    ///
    /// The offset one past the window's last grapheme is the start of the next hard line,
    /// which was not laid out, unless the window ends with the text.
    pub(crate) fn lays_out_caret(&self, offset: usize) -> bool {
        let end = self.grapheme_base + self.grapheme_len;
        offset >= self.grapheme_base && (offset < end || (offset == end && self.lines.end == self.index.len()))
    }
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
            ranges: None,
            embeds: Vec::new(),
            inlays: Rc::from(Vec::new()),
            inlay_scale: 1.0,
            line_offsets,
            grapheme_checkpoints: OnceCell::new(),
            ordered_clusters: OnceCell::new(),
            display_widget: None,
            window: None,
        }
    }

    /// A geometry of the hard lines `lines` of `index`'s text, laid out as `interaction`
    /// (see [`windowed::lay_out_lines`](crate::editable_text::windowed::lay_out_lines)).
    pub(crate) fn windowed(
        index: Rc<LineIndex>,
        lines: Range<usize>,
        interaction: TextInteractionLayout,
        content_width: f32,
        text_align: TextAlign,
    ) -> Self {
        let bytes = index.line_bytes(lines.start).start..index.line_bytes(lines.end - 1).end;
        let display: Arc<str> = Arc::from(&*index.text().slice(bytes.clone()));
        let grapheme_base = index.graphemes_before(lines.start);
        let grapheme_len = index.graphemes_before(lines.end) - grapheme_base;
        let y_base = index.top(lines.start) as f32;
        let mut geometry =
            Self::from_interaction(display, Vec::new(), interaction, content_width, text_align);
        // The lines the field moves between by grapheme offset are in the offsets of the
        // whole text.
        for line in &mut geometry.visual_lines {
            line.byte_start += bytes.start;
            line.byte_end += bytes.start;
            line.grapheme_start += grapheme_base;
            line.grapheme_end += grapheme_base;
        }
        geometry.window = Some(WindowInfo {
            index,
            lines,
            byte_base: bytes.start,
            grapheme_base,
            grapheme_len,
            y_base,
        });
        geometry
    }

    /// Records the styled ranges the layout was shaped with.
    #[inline]
    /// Keeps the text widget this layout was measured with.
    ///
    /// The widget has already laid the whole text out, and so has what paints the
    /// text from this geometry if it is handed the same one. A second widget made
    /// for the painting would lay the same text out again.
    pub(crate) fn with_display_widget(
        mut self,
        style: TextStyle,
        align: TextAlign,
        widget: Rc<RawTextWidget>,
    ) -> Self {
        self.display_widget = Some((style, align, widget));
        self
    }

    /// The widget this layout was measured with, if it is the one wanted for `style`
    /// and `align`.
    pub(crate) fn display_widget_for(
        &self,
        style: &TextStyle,
        align: TextAlign,
    ) -> Option<Rc<RawTextWidget>> {
        self.display_widget
            .as_ref()
            .filter(|(kept_style, kept_align, _)| kept_style == style && *kept_align == align)
            .map(|(_, _, widget)| Rc::clone(widget))
    }

    pub(crate) fn with_ranges(mut self, ranges: Option<Rc<[aimer_text::StyledRange]>>) -> Self {
        self.ranges = ranges;
        self
    }

    /// Records the boxes reserved for embedded widgets.
    #[inline]
    pub(crate) fn with_embeds(mut self, embeds: Vec<EmbedBox>) -> Self {
        self.embeds = embeds;
        self
    }

    /// Records the room that was left for inlays.
    #[inline]
    pub(crate) fn with_inlays(mut self, inlays: Rc<[Placed]>, scale: f32) -> Self {
        self.inlays = inlays;
        self.inlay_scale = scale;
        self
    }

    /// Puts a caret that is where an inlay goes before the inlay. The room is spread after the character before it, so a caret
    /// that is at the start of what follows is after the room; but the insertion point is where the text is, and the inlay is a note
    /// beside it.
    fn before_inlay(&self, mut caret: CaretGeometry) -> CaretGeometry {
        if let Ok(found) = self.inlays.binary_search_by_key(&caret.offset, |place| place.at)
            && !self.inlays[found].eol
            && !self.inlays[found].word
        {
            caret.x -= self.inlays[found].gap * self.inlay_scale;
        }
        caret
    }

    /// Where the room of `place` begins, in physical pixels from the origin of what was laid out, which for a window of a long text
    /// is the window's: how far along its line, and the baseline of the line. `scale` is how many physical pixels a logical one is, which is what the room was asked for in.
    ///
    /// The room is spread by the letter spacing after the character before `place.at`, so it ends where the cluster of that
    /// character does. None where that character is not laid out, which is a text that is windowed and not there.
    pub(crate) fn inlay_origin(&self, place: &Placed, scale: f32) -> Option<(f32, f32)> {
        let layout = self.interaction.as_ref()?;
        let byte_base = self.window.as_ref().map_or(0, |window| window.byte_base);
        let local = place.at.checked_sub(byte_base)?;
        // The room of a space is the space's own, and begins where it does, which is where the caret before it is.
        if place.word {
            let cluster = self.cluster_starting_at(layout, local)?;
            return (!cluster.level.is_rtl()).then_some((cluster.start_x, cluster.start_y));
        }
        let cluster = self.cluster_ending_at(layout, local)?;
        if cluster.level.is_rtl() {
            return None;
        }
        // At the end of a line the labels are after the last character, where the room is not spread.
        let room = if place.eol { 0.0 } else { place.gap * scale };
        Some((cluster.end_x - room, cluster.end_y))
    }

    /// Where each embed's box was laid out, from the cluster that stands for its
    /// range: the box sits on the baseline of its line, `height` above it.
    pub(crate) fn embed_rects(&self) -> Vec<EmbedRect> {
        let Some(layout) = self.interaction.as_ref() else {
            return Vec::new();
        };
        self.embeds
            .iter()
            .filter_map(|embed| {
                // The cluster a front-to-back search finds, bisected: an embed is looked for in the
                // layout of every frame, among all the clusters of the window.
                let cluster = self.cluster_starting_at(layout, embed.range.start)?;
                Some(EmbedRect {
                    key: embed.key,
                    x: cluster.start_x.min(cluster.end_x),
                    top: cluster.start_y - embed.height + self.window.as_ref().map_or(0.0, |window| window.y_base),
                    width: embed.width,
                    height: embed.height,
                })
            })
            .collect()
    }

    /// Converts a displayed grapheme offset into the byte offset consumed by
    /// the canonical interaction layout.
    #[inline]
    ///
    /// Walks from the nearest of the sampled grapheme starts, and not from the start
    /// of the text, so the cost does not grow with the note.
    pub(crate) fn display_byte_offset(&self, offset: usize) -> usize {
        let checkpoints = self.grapheme_checkpoints();
        let Some(&from) = checkpoints.get(offset / GRAPHEME_STRIDE) else {
            return self.display.len();
        };
        self.display[from..]
            .grapheme_indices(true)
            .nth(offset % GRAPHEME_STRIDE)
            .map_or(self.display.len(), |(byte, _)| from + byte)
    }

    /// How many graphemes of the display text come before `byte`: the count of
    /// `display[..byte]`, found from the nearest sampled grapheme start.
    pub(crate) fn display_grapheme_offset(&self, byte: usize) -> usize {
        let byte = byte.min(self.display.len());
        let checkpoints = self.grapheme_checkpoints();
        let at = checkpoints.partition_point(|&start| start <= byte);
        let Some(index) = at.checked_sub(1) else {
            return 0;
        };
        index * GRAPHEME_STRIDE + grapheme_count(&self.display[checkpoints[index]..byte])
    }

    fn grapheme_checkpoints(&self) -> &[usize] {
        self.grapheme_checkpoints.get_or_init(|| {
            if is_one_byte_graphemes(&self.display) {
                return (0..self.display.len()).step_by(GRAPHEME_STRIDE).collect();
            }
            self.display
                .grapheme_indices(true)
                .step_by(GRAPHEME_STRIDE)
                .map(|(byte, _)| byte)
                .collect()
        })
    }

    /// The first of the layout's clusters that starts at `byte`.
    ///
    /// The clusters run in text order, which lets the search be a bisection, except
    /// for the line breaks some layouts append after them all. Only the ordered
    /// first part is bisected and the rest, a cluster per line, is searched in turn.
    /// This is the cluster a front-to-back search would find, but not at the cost
    /// of every cluster in the note each time the caret is placed.
    fn cluster_starting_at<'a>(
        &self,
        layout: &'a TextInteractionLayout,
        byte: usize,
    ) -> Option<&'a TextCluster> {
        let ordered = self.ordered_cluster_count(layout);
        let (head, tail) = layout.clusters.split_at(ordered);
        let first = head.partition_point(|cluster| cluster.text_range.start < byte);
        head.get(first)
            .filter(|cluster| cluster.text_range.start == byte)
            .or_else(|| tail.iter().find(|cluster| cluster.text_range.start == byte))
    }

    /// The first of the layout's clusters that ends at `byte`; see
    /// [`Self::cluster_starting_at`].
    fn cluster_ending_at<'a>(
        &self,
        layout: &'a TextInteractionLayout,
        byte: usize,
    ) -> Option<&'a TextCluster> {
        let ordered = self.ordered_cluster_count(layout);
        let (head, tail) = layout.clusters.split_at(ordered);
        let first = head.partition_point(|cluster| cluster.text_range.end < byte);
        head.get(first)
            .filter(|cluster| cluster.text_range.end == byte)
            .or_else(|| tail.iter().find(|cluster| cluster.text_range.end == byte))
    }

    /// How many clusters, from the first, have starts and ends that never go back.
    fn ordered_cluster_count(&self, layout: &TextInteractionLayout) -> usize {
        *self.ordered_clusters.get_or_init(|| {
            layout
                .clusters
                .windows(2)
                .position(|pair| {
                    pair[1].text_range.start < pair[0].text_range.start
                        || pair[1].text_range.end < pair[0].text_range.end
                })
                .map_or(layout.clusters.len(), |at| at + 1)
        })
    }

    /// Returns the shaped caret for a displayed grapheme offset. If the
    /// shaper fused adjacent graphemes into a ligature, the nearest legal
    /// cluster edge is used so the caret never cuts through the painted glyph.
    pub(crate) fn caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        let caret = self.shaped_caret_geometry(offset)?;
        Some(if self.inlays.is_empty() { caret } else { self.before_inlay(caret) })
    }

    /// [`caret_geometry`](Self::caret_geometry) where it is as the layout shaped it.
    fn shaped_caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        let Some(window) = self.window.as_ref() else {
            return self.local_caret_geometry(offset);
        };
        let layout = self.interaction.as_ref()?;
        if window.lays_out_caret(offset) {
            let mut caret = self.local_caret_geometry(offset - window.grapheme_base)?;
            caret.y += window.y_base;
            caret.offset += window.byte_base;
            return Some(caret);
        }
        // Out of the window: where the line is, by its estimate. Nothing is drawn there,
        // and whoever scrolls to it finds the line laid out and asks again.
        let (line, _) = window.index.line_of_grapheme(offset);
        let before = offset < window.grapheme_base;
        Some(CaretGeometry {
            offset: window.index.line_bytes(line).start,
            line_index: if before { 0 } else { layout.lines.len().saturating_sub(1) },
            x: layout.origin_x,
            y: window.index.top(line) as f32,
            width: 0.0,
            height: window.index.metrics().line_height,
        })
    }

    /// [`caret_geometry`](Self::caret_geometry) for an offset in the laid-out text, in
    /// that text's own coordinates.
    fn local_caret_geometry(&self, offset: usize) -> Option<CaretGeometry> {
        let layout = self.interaction.as_ref()?;
        let byte = self.display_byte_offset(offset);
        let layout = &**layout;

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
            && self.cluster_starting_at(layout, byte).is_none()
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

        if byte <= layout.text.len() && layout.text.is_char_boundary(byte) {
            if let Some(cluster) = self.cluster_starting_at(layout, byte) {
                return Some(layout.caret_geometry_at_cluster(cluster, byte, true));
            }
            if let Some(cluster) = self.cluster_ending_at(layout, byte) {
                return Some(layout.caret_geometry_at_cluster(cluster, byte, false));
            }
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
        let Some(window) = self.window.as_ref() else {
            let byte = layout.hit_test(x, y)?;
            return Some(self.display_grapheme_offset(byte));
        };
        // A point above or below the window is at its first or last line.
        let byte = layout.hit_test(x, y - window.y_base)?;
        Some(window.grapheme_base + self.display_grapheme_offset(byte))
    }

    /// Returns canonical selection rectangles for displayed grapheme offsets.
    #[inline]
    pub(crate) fn selection_rects(&self, start: usize, end: usize) -> Vec<SelectionRect> {
        let Some(layout) = self.interaction.as_ref() else {
            return Vec::new();
        };
        let Some(window) = self.window.as_ref() else {
            return layout
                .selection_rects(self.display_byte_offset(start)..self.display_byte_offset(end));
        };
        // Only the part of the selection in the window has been laid out; the rest is
        // not on screen.
        let low = window.grapheme_base;
        let high = window.grapheme_base + window.grapheme_len;
        let (start, end) = (start.clamp(low, high) - low, end.clamp(low, high) - low);
        if start >= end {
            return Vec::new();
        }
        let mut rects =
            layout.selection_rects(self.display_byte_offset(start)..self.display_byte_offset(end));
        for rect in &mut rects {
            rect.y += window.y_base;
        }
        rects
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
        let start = self.display_grapheme_offset(range.start);
        let end = self.display_grapheme_offset(range.end);
        self.source_range_for_display_range(start, end)
    }

    /// The height the laid-out lines stack to.
    ///
    /// Lines differ in height when ranges change the font size, so this is the
    /// paragraph's own height and never `lines * line_height`, which counts
    /// every line as tall as the tallest. `line_height` is that tallest line,
    /// used only when the text has no layout to measure.
    #[inline]
    pub(crate) fn content_height(&self, line_height: f32) -> f32 {
        if let (Some(window), Some(layout)) = (self.window.as_ref(), self.interaction.as_ref()) {
            // The lines stacked, those that have not been laid out as they are estimated, and the
            // gap that a layout adds once: the largest of any line's, as the whole text's does.
            return window.index.total_height() as f32 + window.index.line_gap().max(layout.metrics.line_gap);
        }
        self.interaction
            .as_ref()
            .map(|layout| layout.metrics.height + layout.metrics.line_gap)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or_else(|| self.visual_lines.len().max(1) as f32 * line_height)
    }

    /// The top and bottom of visual line `line`, in the coordinates of the
    /// stacked lines. A line without a layout is placed `line_height` apart.
    pub(crate) fn line_span(&self, line: usize, line_height: f32) -> (f32, f32) {
        let Some(layout) = self.interaction.as_ref() else {
            return (line as f32 * line_height, (line + 1) as f32 * line_height);
        };
        let y_base = self.window.as_ref().map_or(0.0, |window| window.y_base);
        let top_of = |line: &TextLine| line.baseline - line.ascent;
        let Some(this) = layout.lines.get(line) else {
            return (line as f32 * line_height, (line + 1) as f32 * line_height);
        };
        let bottom = match (layout.lines.get(line + 1), self.window.as_ref()) {
            (Some(next), _) => top_of(next),
            // The last line laid out ends where the window does, and the last of the whole
            // text ends where the text does, as an unwindowed one does.
            (None, Some(window)) => {
                let gap = if window.lines.end == window.index.len() { layout.metrics.line_gap } else { 0.0 };
                layout.metrics.height + gap
            }
            (None, None) => self.content_height(line_height),
        };
        (top_of(this) + y_base, bottom + y_base)
    }

    /// The top and bottom of the line the caret is on, which is what scrolling to the
    /// caret needs. For a caret out of a windowed geometry's window this is where the
    /// line is by its estimate.
    pub(crate) fn reveal_span(&self, offset: usize, line_height: f32) -> (f32, f32) {
        if let Some(window) = self.window.as_ref()
            && !window.lays_out_caret(offset)
        {
            let (line, _) = window.index.line_of_grapheme(offset);
            let top = window.index.top(line) as f32;
            return (top, top + window.index.height(line));
        }
        self.line_span(self.line_for_offset(offset), line_height)
    }

    /// How many graphemes the whole text has.
    pub(crate) fn grapheme_count(&self) -> usize {
        match self.window.as_ref() {
            Some(window) => window.index.total_graphemes(),
            None => grapheme_count(&self.display),
        }
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

/// The lookups as they were before they were indexed: a search from the start of the
/// text each time. Kept to check that the indexed ones answer the same.
#[cfg(test)]
impl EditableGeometry {
    pub(crate) fn display_byte_offset_scanning(&self, offset: usize) -> usize {
        self.display
            .grapheme_indices(true)
            .nth(offset)
            .map_or(self.display.len(), |(byte, _)| byte)
    }

    pub(crate) fn display_grapheme_offset_scanning(&self, byte: usize) -> usize {
        grapheme_count(&self.display[..byte.min(self.display.len())])
    }

    pub(crate) fn caret_geometry_scanning(&self, offset: usize) -> Option<CaretGeometry> {
        let layout = self.interaction.as_ref()?;
        let byte = self.display_byte_offset_scanning(offset);
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
}

/// How many graphemes apart the sampled grapheme starts of a geometry are.
const GRAPHEME_STRIDE: usize = 64;

/// Whether every grapheme of `text` is a single byte, which is so of ASCII but for
/// a carriage return before a line feed, the one ASCII grapheme of two bytes.
///
/// Finding the graphemes of a long text costs a walk over it, and for such text
/// the answer is already known: the count before a byte is the byte.
#[inline]
pub(crate) fn is_one_byte_graphemes(text: &str) -> bool {
    text.is_ascii() && !text.as_bytes().contains(&b'\r')
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

/// How many graphemes come before each of `offsets`, the count of `text[..offset]`
/// for every one of them.
///
/// Offsets that never go backwards, as the lines of a layout do, are answered in
/// one walk over `text`. Counting each prefix from the start instead costs a pass
/// over the text per offset, which for a long note with thousands of lines is
/// billions of steps.
fn grapheme_counts_before(text: &str, offsets: &[usize]) -> Vec<usize> {
    if is_one_byte_graphemes(text) {
        return offsets.to_vec();
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return offsets
            .iter()
            .map(|&offset| grapheme_count(&text[..offset]))
            .collect();
    }
    let mut starts = text.grapheme_indices(true).map(|(byte, _)| byte).peekable();
    let mut before = 0;
    offsets
        .iter()
        .map(|&offset| {
            while starts.next_if(|&byte| byte < offset).is_some() {
                before += 1;
            }
            before
        })
        .collect()
}

fn visual_lines_from_interaction(
    display: &str,
    interaction: &TextInteractionLayout,
) -> Vec<VisualLine> {
    let spans = interaction
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
            (byte_start, byte_end, line.width)
        })
        .collect::<Vec<_>>();
    let offsets = spans
        .iter()
        .flat_map(|&(start, end, _)| [start, end])
        .collect::<Vec<_>>();
    let counts = grapheme_counts_before(display, &offsets);
    let mut lines = spans
        .iter()
        .enumerate()
        .map(|(index, &(byte_start, byte_end, width))| VisualLine {
            byte_start,
            byte_end,
            grapheme_start: counts[index * 2],
            grapheme_end: counts[index * 2 + 1],
            width,
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
    // A move of many lines (a page) stops at the first or last one.
    let target_index = line_index.saturating_add_signed(direction).min(lines.len() - 1);
    if target_index == line_index {
        return current;
    }
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

    /// The geometry made for `key`, if it is the one the cache holds.
    pub(crate) fn get(&self, key: &EditableGeometryKey) -> Option<Rc<EditableGeometry>> {
        self.cached
            .borrow()
            .as_ref()
            .filter(|(cached_key, _)| cached_key == key)
            .map(|(_, geometry)| geometry.clone())
    }

    /// Keeps `geometry` as the one made for `key`.
    pub(crate) fn store(&self, key: EditableGeometryKey, geometry: EditableGeometry) -> Rc<EditableGeometry> {
        let geometry = Rc::new(geometry);
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
    use super::{grapheme_count, grapheme_counts_before, vertical_target, wrap_visual_lines};

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
            highlighter: 0,
            active_start: usize::MAX,
            active_end: usize::MAX,
            embeds: 0,
            window: (0, 0),
        };
        let build = || {
            builds.set(builds.get() + 1);
            EditableGeometry {
                display: Arc::from("hello"),
                text_width: 40.0,
                visual_lines: Vec::new(),
                interaction: None,
                source_ranges: Vec::new(),
                ranges: None,
                embeds: Vec::new(),
                inlays: std::rc::Rc::from(Vec::new()),
                inlay_scale: 1.0,
                line_offsets: Vec::new(),
                grapheme_checkpoints: std::cell::OnceCell::new(),
                ordered_clusters: std::cell::OnceCell::new(),
                display_widget: None,
                window: None,
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
    fn grapheme_counts_before_match_counting_each_prefix() {
        // A family emoji is one grapheme of several scalars; "e\u{301}" is two scalars
        // and one grapheme; "\r\n" is one grapheme.
        let text = "ab\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}e\u{301}\r\ncd\n\n\u{4F60}\u{597D}";
        let boundaries: Vec<usize> = text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([text.len()])
            .collect();

        let ascending = boundaries.clone();
        let expected: Vec<usize> = ascending
            .iter()
            .map(|&offset| grapheme_count(&text[..offset]))
            .collect();
        assert_eq!(grapheme_counts_before(text, &ascending), expected);

        let mut repeated = vec![0, 0, 2, 2, text.len(), text.len()];
        let expected: Vec<usize> = repeated
            .iter()
            .map(|&offset| grapheme_count(&text[..offset]))
            .collect();
        assert_eq!(grapheme_counts_before(text, &repeated), expected);

        // Offsets that go backwards fall back to counting each prefix.
        repeated.reverse();
        repeated.push(1);
        let expected: Vec<usize> = repeated
            .iter()
            .map(|&offset| grapheme_count(&text[..offset]))
            .collect();
        assert_eq!(grapheme_counts_before(text, &repeated), expected);
        assert!(grapheme_counts_before("", &[]).is_empty());
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
