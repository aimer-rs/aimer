//! Laying out only some of the lines of a long text.
//!
//! The lines of a long note that are not near the viewport need no layout: nothing
//! shows them, and their height can be estimated ([`LineIndex`]). So the field lays out
//! a window of whole hard lines around the viewport and nothing else, and what it
//! cost to open, to edit and to hold stops depending on how long the note is.
//!
//! A hard line wraps and stacks independently of the others, so the layout of a window
//! is the layouts of its lines, each laid out as a text of its own and moved into place
//! (the same thing the chunked paragraph does for rich text). That makes every line's
//! layout one the shaping cache already holds when the line has been seen before.

use std::ops::Range;
use std::rc::Rc;

use aimer_cupid::text_layout::{
    ParagraphMetrics, TextCluster, TextInteractionLayout, TextLine, TextWritingMode,
};

use crate::editable_text::line_index::LineIndex;

/// How much text, in bytes, a field must hold before it lays out a window of it and not
/// all of it. Below this the layout of the whole text is cheap, and it is exact.
pub(crate) const WINDOWED_MIN_BYTES: usize = 64 * 1024;

/// Lays out one hard line, with its line feed, as a text of its own: what the field
/// would have laid out for the whole text, restricted to that line.
pub(crate) type LayoutLine<'a> = &'a dyn Fn(&str) -> Rc<TextInteractionLayout>;

/// Lays out the lines of `lines` that have not been, so the index knows their heights.
/// A layout of a line is one the shaping cache keeps, so laying them out again to
/// assemble a window costs little.
pub(crate) fn measure_lines(index: &LineIndex, lines: Range<usize>, layout_line: LayoutLine<'_>) {
    for line in lines {
        if index.is_exact(line) {
            continue;
        }
        // One line is a window of one.
        let _ = lay_out_lines(index, line..line + 1, layout_line);
    }
}

/// Teaches the index the heights of the hard lines `lines` as a layout of their text gave
/// them. The layout is of rich text, whose lines differ in height and some of whose source
/// lines (the inside of an embedded widget) have no line of their own, so a hard line's
/// height is how far the line below it starts from where it does, whatever lines there are
/// between.
pub(crate) fn refine_from_layout(index: &LineIndex, lines: Range<usize>, layout: &TextInteractionLayout) {
    let window_start = index.line_bytes(lines.start).start;
    let top = |line: &TextLine| line.baseline - line.ascent - line.line_gap * 0.5;
    // The first visual line that starts at or after each hard line's start.
    let mut visual = 0;
    let mut first_visual = Vec::with_capacity(lines.len());
    for line in lines.clone() {
        let start = index.line_bytes(line).start - window_start;
        while visual < layout.lines.len() && layout.lines[visual].text_range.start < start {
            visual += 1;
        }
        first_visual.push(visual);
    }
    // A window that does not end the text ends in a line feed, and the layout of text that
    // ends in one has an empty line after it, which is the next hard line's, not this one's.
    let window_text_len = index.line_bytes(lines.end - 1).end - window_start;
    let phantom = lines.end < index.len()
        && layout
            .lines
            .last()
            .is_some_and(|last| last.text_range.is_empty() && last.text_range.start >= window_text_len);
    let window_height = match (phantom, layout.lines.last()) {
        (true, Some(last)) => top(last),
        _ => layout.metrics.height,
    };
    index.observe_line_gap(layout.metrics.line_gap);
    for (offset, line) in lines.enumerate() {
        let from = first_visual[offset];
        let top_of = |visual: usize| layout.lines.get(visual).map_or(window_height, top);
        let next = first_visual.get(offset + 1).map_or(window_height, |&visual| top_of(visual));
        index.refine(line, (next - top_of(from)).max(0.0));
    }
}

/// The layout of the hard lines `lines` of `index`'s text, as the layout of a text that
/// is those lines alone: the first line's top is the top of the layout. What it learns
/// of the lines (their exact heights, how wide a grapheme is) goes into the index.
///
/// Every line but the last ends in a line feed, and a layout of a text that ends in one
/// has an empty line after it. That line is the next hard line's, so it is left out
/// here.
///
/// A carriage return before the line feed is not a break to the shaper, and a full
/// layout of a text with one runs the line into the next; this puts the next line on
/// its own, as the text says.
pub(crate) fn lay_out_lines(
    index: &LineIndex,
    lines: Range<usize>,
    layout_line: LayoutLine<'_>,
) -> TextInteractionLayout {
    debug_assert!(!lines.is_empty() && lines.end <= index.len());
    let window_bytes = index.line_bytes(lines.start).start..index.line_bytes(lines.end - 1).end;
    let text = index.text()[window_bytes.clone()].to_owned();

    let mut laid_lines: Vec<TextLine> = Vec::new();
    let mut clusters: Vec<TextCluster> = Vec::new();
    let mut metrics: Option<ParagraphMetrics> = None;
    let (mut origin_x, mut origin_y) = (0.0, 0.0);
    let (mut width, mut line_gap) = (0.0_f32, 0.0_f32);
    let mut y_offset = 0.0_f32;

    for line in lines.clone() {
        let bytes = index.line_bytes(line);
        let layout = layout_line(&index.text()[bytes.clone()]);
        // A layout of text that ends in a line feed ends in an empty line after it. A line
        // ending in a carriage return and a line feed has none: the shaper breaks only at
        // a lone line feed, so that is not a break to it, and the next hard line is the
        // next line all the same.
        let phantom = layout.lines.len() >= 2
            && layout.lines.last().is_some_and(|last| last.text_range.is_empty())
            && layout.lines[layout.lines.len() - 2].hard_break;
        let own = layout.lines.len() - usize::from(phantom);
        let pitch = layout.metrics.line_height;
        let byte_offset = bytes.start - window_bytes.start;
        let visual_offset = laid_lines.len();

        for line in layout.lines.iter().take(own) {
            laid_lines.push(TextLine {
                text_range: line.text_range.start + byte_offset..line.text_range.end + byte_offset,
                glyph_range: line.glyph_range.clone(),
                baseline: line.baseline + y_offset,
                width: line.width,
                ascent: line.ascent,
                descent: line.descent,
                line_gap: line.line_gap,
                hard_break: line.hard_break,
            });
        }
        for cluster in layout.clusters.iter().filter(|cluster| cluster.line_index < own) {
            clusters.push(TextCluster {
                text_range: cluster.text_range.start + byte_offset..cluster.text_range.end + byte_offset,
                line_index: cluster.line_index + visual_offset,
                level: cluster.level,
                start_x: cluster.start_x,
                end_x: cluster.end_x,
                start_y: cluster.start_y + y_offset,
                end_y: cluster.end_y + y_offset,
                y: cluster.y + y_offset,
                height: cluster.height,
            });
        }

        if metrics.is_none() {
            origin_x = layout.origin_x;
            origin_y = layout.origin_y;
            metrics = Some(layout.metrics.clone());
        }
        width = width.max(layout.metrics.width);
        line_gap = line_gap.max(layout.metrics.line_gap);
        index.observe_line_gap(layout.metrics.line_gap);
        let height = own as f32 * pitch;
        index.refine(line, height);
        if own == 1 {
            // A line that did not wrap: its width over its graphemes is how wide one is.
            let graphemes = index.graphemes_before(line + 1) - index.graphemes_before(line);
            let line_feed = usize::from(index.text()[bytes.clone()].ends_with('\n'));
            index.observe(graphemes.saturating_sub(line_feed), layout.lines[0].width);
        }
        y_offset += height;
    }

    let first = metrics.expect("a window has at least one line");
    TextInteractionLayout {
        text,
        metrics: ParagraphMetrics {
            width,
            height: y_offset,
            ascent: first.ascent,
            descent: first.descent,
            line_gap,
            line_height: first.line_height,
            line_count: laid_lines.len(),
        },
        lines: laid_lines,
        clusters,
        origin_x,
        origin_y,
        writing_mode: TextWritingMode::HorizontalTb,
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;
    use std::rc::Rc;
    use std::sync::Arc;

    use aimer_cupid::font::{FontFamily, FontStyle};
    use aimer_style::TextAlign;
    use aimer_widget::base::BuildContext;
    use unicode_segmentation::UnicodeSegmentation;

    use super::lay_out_lines;
    use crate::editable_text::EditableGeometry;
    use crate::editable_text::line_index::{LineIndex, LineMetrics};
    use crate::input_field::raw_fields::test_support::dummy_build_context;

    const WIDTH: f32 = 220.0;
    const FONT_SIZE: f32 = 16.0;

    fn full(ctx: &BuildContext, text: &str) -> EditableGeometry {
        let layout = ctx.canvas.layout_text_styled(
            text,
            FONT_SIZE,
            WIDTH,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            400,
        );
        EditableGeometry::from_interaction(Arc::from(text), Vec::new(), layout, WIDTH, TextAlign::TopLeft)
    }

    fn layout_line<'a>(ctx: &'a BuildContext) -> impl Fn(&str) -> Rc<aimer_cupid::text_layout::TextInteractionLayout> + 'a {
        move |line| {
            ctx.canvas.layout_text_styled_shared(
                line,
                FONT_SIZE,
                WIDTH,
                FontFamily::SANS_SERIF,
                FontStyle::Normal,
                400,
            )
        }
    }

    fn windowed(ctx: &BuildContext, index: &Rc<LineIndex>, lines: Range<usize>) -> EditableGeometry {
        let interaction = lay_out_lines(index, lines.clone(), &layout_line(ctx));
        EditableGeometry::windowed(Rc::clone(index), lines, interaction, WIDTH, TextAlign::TopLeft)
    }

    /// An index whose every line is exact, as it is once the whole text has been laid out.
    fn exact_index(ctx: &BuildContext, text: &str) -> Rc<LineIndex> {
        let metrics = LineMetrics { content_width: WIDTH, line_height: 20.0, advance: 8.0 };
        let index = Rc::new(LineIndex::new(Arc::from(text), metrics));
        let _ = lay_out_lines(&index, 0..index.len(), &layout_line(ctx));
        index
    }

    fn note() -> String {
        let mut text = String::new();
        for index in 0..40 {
            match index % 8 {
                0 => text.push_str(&format!("Heading {index}\n")),
                1 => text.push_str(&format!(
                    "A paragraph line {index} with enough words in it that it has to wrap across \
                     the width of the pane more than once, at least when it is narrow like this\n"
                )),
                2 => text.push('\n'),
                3 => text.push_str(&format!("- item {index} caf\u{e9} \u{4f60}\u{597d}\n")),
                4 => text.push_str(&format!("tab\tseparated {index} \u{1f468}\u{200d}\u{1f469} done\n")),
                5 => text.push_str("\n\n"),
                _ => text.push_str(&format!("short {index}\n")),
            }
        }
        text.push_str("the last line, with no line feed after it");
        text
    }

    fn near(a: f32, b: f32, what: &str) {
        assert!((a - b).abs() <= 0.02 + 1e-5 * a.abs().max(b.abs()), "{what}: {a} against {b}");
    }

    fn windows(index: &LineIndex) -> Vec<Range<usize>> {
        let n = index.len();
        vec![0..n, 0..1, 0..3, n - 1..n, n - 3..n, 5..6, 5..12, 10..n - 4, n / 2..n / 2 + 7]
    }

    #[test]
    fn a_windowed_geometry_answers_as_the_whole_geometry_does() {
        let ctx = dummy_build_context(400.0, 300.0);
        let text = note();
        let whole = full(&ctx, &text);
        let index = exact_index(&ctx, &text);
        assert_eq!(index.total_graphemes(), text.graphemes(true).count());

        for lines in windows(&index) {
            let window = windowed(&ctx, &index, lines.clone());
            let info = window.window.as_ref().expect("a windowed geometry");
            let what = format!("window {lines:?}");

            near(window.content_height(20.0), whole.content_height(20.0), &format!("{what}: content height"));
            assert_eq!(window.grapheme_count(), text.graphemes(true).count());

            // Every caret in the window.
            for offset in info.grapheme_base..=info.grapheme_base + info.grapheme_len {
                if !info.lays_out_caret(offset) {
                    continue;
                }
                let got = window.caret_geometry(offset).expect("a caret in the window");
                let want = whole.caret_geometry(offset).expect("a caret");
                near(got.x, want.x, &format!("{what}: caret {offset} x"));
                near(got.y, want.y, &format!("{what}: caret {offset} y"));
                near(got.height, want.height, &format!("{what}: caret {offset} height"));
                near(got.width, want.width, &format!("{what}: caret {offset} width"));
            }

            // Points over the window, one in a few pixels.
            let top = info.y_base + 1.0;
            let bottom = info.y_base + window.interaction.as_ref().unwrap().metrics.height - 1.0;
            let mut y = top;
            while y < bottom {
                for x in [-5.0, 3.0, 40.0, 117.0, 300.0] {
                    assert_eq!(
                        window.hit_test(x, y),
                        whole.hit_test(x, y),
                        "{what}: point ({x}, {y})"
                    );
                }
                y += 7.0;
            }

            // Selections inside the window.
            let (low, high) = (info.grapheme_base, info.grapheme_base + info.grapheme_len);
            let probes = [
                (low, high),
                (low, low + (high - low) / 2),
                (low + (high - low) / 3, high),
                (low + (high - low) / 3, low + 2 * (high - low) / 3),
            ];
            for (start, end) in probes {
                let got = window.selection_rects(start, end);
                let want = whole.selection_rects(start, end);
                assert_eq!(got.len(), want.len(), "{what}: selection {start}..{end} rects");
                for (got, want) in got.iter().zip(&want) {
                    near(got.x, want.x, &format!("{what}: selection x"));
                    near(got.y, want.y, &format!("{what}: selection y"));
                    near(got.width, want.width, &format!("{what}: selection width"));
                    near(got.height, want.height, &format!("{what}: selection height"));
                }
            }

            // The lines of the window, as the caret's own line sees them.
            for offset in (low..=high).step_by(((high - low) / 12).max(1)) {
                if !info.lays_out_caret(offset) {
                    continue;
                }
                let got = window.reveal_span(offset, 20.0);
                let want = whole.line_span(whole.line_for_offset(offset), 20.0);
                near(got.0, want.0, &format!("{what}: line top at {offset}"));
                near(got.1, want.1, &format!("{what}: line bottom at {offset}"));
            }
        }
    }

    #[test]
    fn a_caret_out_of_the_window_is_where_its_line_is() {
        let ctx = dummy_build_context(400.0, 300.0);
        let text = note();
        let whole = full(&ctx, &text);
        let index = exact_index(&ctx, &text);
        let window = windowed(&ctx, &index, 10..16);
        let info = window.window.as_ref().unwrap();

        for offset in [0, info.grapheme_base - 1, info.grapheme_base + info.grapheme_len + 1, index.total_graphemes()] {
            let got = window.caret_geometry(offset).expect("a caret anywhere");
            let want = whole.caret_geometry(offset).expect("a caret");
            // With every line exact the estimate of a line's top is its top.
            let line = index.line_of_grapheme(offset).0;
            near(got.y, index.top(line) as f32, &format!("caret {offset}: where its line is"));
            assert!(got.y <= want.y + 0.5 && want.y <= got.y + index.height(line) + 0.5, "caret {offset}: in its line");
            let span = window.reveal_span(offset, 20.0);
            near(span.0, index.top(line) as f32, &format!("caret {offset}: reveal top"));
            near(span.1 - span.0, index.height(line), &format!("caret {offset}: reveal height"));
        }
    }

    #[test]
    fn laying_out_lines_teaches_the_index_their_heights() {
        let ctx = dummy_build_context(400.0, 300.0);
        let text = note();
        let metrics = LineMetrics { content_width: WIDTH, line_height: 20.0, advance: 8.0 };
        let index = Rc::new(LineIndex::new(Arc::from(text.as_str()), metrics));
        let estimated = index.total_height();
        assert!(!index.is_exact(1));

        let _ = windowed(&ctx, &index, 0..4);
        assert!((0..4).all(|line| index.is_exact(line)));
        assert!(!index.is_exact(4));
        // The long line wraps: its estimate was a guess and is now what it measured.
        let _ = windowed(&ctx, &index, 0..index.len());
        let whole = full(&ctx, &text);
        near(index.total_height() as f32 + 0.0, whole.content_height(20.0) - whole.interaction.as_ref().unwrap().metrics.line_gap, "total height");
        assert!(estimated > 0.0);
    }

    #[test]
    fn a_carriage_return_line_feed_ends_a_hard_line_of_its_own() {
        let ctx = dummy_build_context(400.0, 300.0);
        let text = "one\r\ntwo\r\n\r\nthree words here that wrap across the narrow pane a few times over\r\nend";
        let index = exact_index(&ctx, text);
        assert_eq!(index.len(), 5);
        let window = windowed(&ctx, &index, 0..index.len());
        let layout = window.interaction.as_ref().unwrap();
        // One visual line each, but for the one that wraps.
        let wrapped = layout.lines.len() - 4;
        assert!(wrapped >= 2, "{} visual lines", layout.lines.len());
        // The heights stack: each hard line starts where the one before it ended.
        for line in 1..index.len() {
            let top = index.top(line) as f32;
            let first = layout.lines.iter().find(|visual| visual.text_range.start >= index.line_bytes(line).start).unwrap();
            near(first.baseline - first.ascent, top, &format!("line {line} starts where the line above ends"));
        }
        for offset in 0..=index.total_graphemes() {
            assert!(window.caret_geometry(offset).is_some());
        }
    }
}
