//! Where every hard line of a long text is, how tall it is, and how many graphemes it has.
//!
//! This is the editor's view of an [`aimer_text_buffer::LayoutIndex`]: the same index, kept with the
//! text it is of, shared by the field and the geometry it lays out, and answering in the terms the
//! field asks in (lines, graphemes, heights). A "line" here is a unit of the index: a hard line, or
//! a segment of one too long to lay out whole.
//!
//! A line has
//!
//! * a **height**, estimated from its length until the line has been laid out and then exact
//!   ([`refine`](LineIndex::refine)), the way a windowed list corrects what it guessed;
//! * a **grapheme count**, so a grapheme offset anywhere in the note finds its line in O(log n) and
//!   the note's grapheme count never needs a walk over the whole text.
//!
//! After an edit the index is made from the previous one ([`LineIndex::updated`]): the difference
//! between the two texts is found from the chunks they share, and only the lines it touches are
//! estimated and counted again.

use std::cell::RefCell;
use std::ops::Range;

use aimer_text_buffer::{EditDelta, LayoutIndex, TextSnapshot};

use super::large_text::{byte_of_grapheme_in, grapheme_of_byte_in};

pub(crate) use aimer_text_buffer::LayoutMetrics as LineMetrics;

pub(crate) struct LineIndex {
    text: TextSnapshot,
    inner: RefCell<LayoutIndex>,
}

impl LineIndex {
    /// The index of `text`, every height estimated.
    pub(crate) fn new(text: TextSnapshot, metrics: LineMetrics) -> Self {
        let inner = LayoutIndex::build(&text, metrics);
        Self { text, inner: RefCell::new(inner) }
    }

    /// The index of `text`, made from this one, `text` being this one's text edited: the lines
    /// the edit did not touch keep what this index knew of them, and the rest are estimated and
    /// counted anew.
    ///
    /// A change of `metrics` that decides how lines wrap discards what was learned of the heights,
    /// which were for another wrapping.
    pub(crate) fn updated(&self, text: TextSnapshot, metrics: LineMetrics) -> Self {
        let old_metrics = self.inner.borrow().metrics();
        if !old_metrics.same_wrapping(&metrics) {
            return Self::new(text, metrics);
        }
        let mut inner = self.inner.borrow().clone();
        // What has been learned of how wide a grapheme is outranks the caller's first guess.
        inner.set_metrics(metrics);
        if !self.text.ptr_eq(&text) {
            match self.text.diff(&text) {
                None => {}
                Some((old, new)) => {
                    let delta = EditDelta::new(old.clone(), self.text.slice_snapshot(old), text.slice_snapshot(new));
                    inner.apply_edit(&delta, &text);
                }
            }
        }
        Self { text, inner: RefCell::new(inner) }
    }

    /// The text the index is of.
    #[inline]
    pub(crate) fn text(&self) -> &TextSnapshot {
        &self.text
    }

    /// Notes the gap between lines of a line that was laid out; see [`Self::line_gap`].
    pub(crate) fn observe_line_gap(&self, gap: f32) {
        self.inner.borrow_mut().observe_line_gap(gap);
    }

    /// The largest gap between lines that any line laid out has had.
    #[inline]
    pub(crate) fn line_gap(&self) -> f32 {
        self.inner.borrow().line_gap()
    }

    /// How many times a height has changed since the index was made: a layout made of the heights
    /// as they were is stale when this has moved.
    #[inline]
    pub(crate) fn version(&self) -> u64 {
        self.inner.borrow().version()
    }

    /// How many lines there are: one more than the line feeds, so a text that ends in one ends in
    /// an empty line.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    /// The bytes of `line`, with its line feed if it has one.
    pub(crate) fn line_bytes(&self, line: usize) -> Range<usize> {
        self.inner.borrow().unit_bytes(line)
    }

    /// The line that holds the byte at `byte`; the last line for the end of the text.
    pub(crate) fn line_of_byte(&self, byte: usize) -> usize {
        self.inner.borrow().unit_of_byte(byte)
    }

    /// The height from the top of the text to the top of `line`.
    pub(crate) fn top(&self, line: usize) -> f64 {
        self.inner.borrow().top(line)
    }

    /// The height of `line`.
    pub(crate) fn height(&self, line: usize) -> f32 {
        self.inner.borrow().height(line)
    }

    /// Whether the height of `line` was laid out and not estimated.
    pub(crate) fn is_exact(&self, line: usize) -> bool {
        self.inner.borrow().is_exact(line)
    }

    /// The height of the text.
    pub(crate) fn total_height(&self) -> f64 {
        self.inner.borrow().total_height()
    }

    /// The line that holds the height `y`: the first line for a negative one, the last for one
    /// past the end.
    pub(crate) fn line_at_y(&self, y: f64) -> usize {
        self.inner.borrow().unit_at_y(y)
    }

    /// The lines that overlap the heights from `top` to `bottom`.
    pub(crate) fn lines_between(&self, top: f64, bottom: f64) -> Range<usize> {
        self.inner.borrow().units_between(top, bottom)
    }

    /// Records the height `line` was laid out to, which is exact.
    pub(crate) fn refine(&self, line: usize, height: f32) {
        self.inner.borrow_mut().refine(line, height);
    }

    /// Learns how wide a grapheme is from a line that was laid out: `graphemes` of them took
    /// `width` in one visual line.
    pub(crate) fn observe(&self, graphemes: usize, width: f32) {
        self.inner.borrow_mut().observe(graphemes, width);
    }

    /// The metrics heights are estimated by now, with the advance as learned.
    pub(crate) fn metrics(&self) -> LineMetrics {
        self.inner.borrow().metrics()
    }

    /// How many graphemes the text has.
    pub(crate) fn total_graphemes(&self) -> usize {
        self.inner.borrow().total_graphemes()
    }

    /// How many graphemes come before `line`.
    pub(crate) fn graphemes_before(&self, line: usize) -> usize {
        self.inner.borrow().graphemes_before(line)
    }

    /// The line a grapheme offset is in, and how many graphemes of the line come before it. The
    /// offset one past the last grapheme is the end of the last line.
    pub(crate) fn line_of_grapheme(&self, offset: usize) -> (usize, usize) {
        self.inner.borrow().unit_of_grapheme(offset)
    }

    /// The grapheme offset of a byte offset on a grapheme boundary.
    pub(crate) fn grapheme_of_byte(&self, byte: usize) -> usize {
        grapheme_of_byte_in(&self.text, &self.inner.borrow(), byte)
    }

    /// The byte offset of a grapheme offset.
    pub(crate) fn byte_of_grapheme(&self, offset: usize) -> usize {
        byte_of_grapheme_in(&self.text, &self.inner.borrow(), offset)
    }
}

#[cfg(test)]
mod tests {
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    const METRICS: LineMetrics = LineMetrics { content_width: 100.0, line_height: 20.0, advance: 10.0 };

    fn index(text: &str) -> LineIndex {
        LineIndex::new(TextSnapshot::from_text(text), METRICS)
    }

    /// Hard lines the plain way: split after every line feed, with an empty line after a last one.
    fn naive_lines(text: &str) -> Vec<&str> {
        let mut lines = text.split_inclusive('\n').collect::<Vec<_>>();
        if text.is_empty() || text.ends_with('\n') {
            lines.push("");
        }
        lines
    }

    const TEXTS: [&str; 12] = [
        "",
        "a",
        "a\n",
        "a\nb",
        "\n\n\n",
        "one\ntwo\nthree\n",
        "crlf\r\nlines\r\nend",
        "lone\rcr and\r\n",
        "caf\u{e9} e\u{301} \u{4f60}\u{597d}\nnext",
        "family \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} here\n\u{1f1e6}\u{1f1e7}\u{1f1e8}",
        "no newline at all, only a long line that goes on and on",
        "x\n\ny\n\n",
    ];

    #[test]
    fn lines_bytes_and_graphemes_are_what_a_walk_of_the_text_finds() {
        for text in TEXTS {
            let index = index(text);
            let lines = naive_lines(text);
            assert_eq!(index.len(), lines.len(), "{text:?}");
            let mut start = 0;
            let mut graphemes = 0;
            for (number, line) in lines.iter().enumerate() {
                assert_eq!(index.line_bytes(number), start..start + line.len(), "{text:?} line {number}");
                assert_eq!(index.graphemes_before(number), graphemes, "{text:?} line {number}");
                assert_eq!(index.line_of_byte(start), number.min(index.len() - 1), "{text:?} byte {start}");
                start += line.len();
                graphemes += line.graphemes(true).count();
            }
            assert_eq!(index.total_graphemes(), text.graphemes(true).count(), "{text:?}");
            // Every grapheme boundary maps both ways.
            for (grapheme, (byte, _)) in text.grapheme_indices(true).enumerate() {
                assert_eq!(index.grapheme_of_byte(byte), grapheme, "{text:?}");
                assert_eq!(index.byte_of_grapheme(grapheme), byte, "{text:?}");
            }
            assert_eq!(index.grapheme_of_byte(text.len()), text.graphemes(true).count());
            assert_eq!(index.byte_of_grapheme(usize::MAX / 2), text.len());
        }
    }

    #[test]
    fn heights_are_estimated_from_the_width_until_a_line_is_laid_out() {
        let index = index("short\nthis line is long enough to wrap onto more than one visual line of the column\n");
        assert_eq!(index.height(0), 20.0);
        assert!(index.height(1) > 20.0);
        assert!(!index.is_exact(1));
        let (total, before) = (index.total_height(), f64::from(index.height(1)));
        index.refine(1, 77.0);
        assert!(index.is_exact(1) && index.height(1) == 77.0);
        assert!((index.total_height() - (total - before + 77.0)).abs() < 1e-6);
        assert_eq!(index.line_at_y(index.top(1) + 1.0), 1);
    }

    #[test]
    fn an_index_made_from_an_edit_is_the_index_of_the_edited_text_and_keeps_what_was_laid_out() {
        let before = "alpha\nbeta\ngamma\ndelta\n";
        let old = index(before);
        old.refine(0, 33.0);
        old.refine(3, 44.0);
        let edited = TextSnapshot::from_text(before).with_replaced(12..12, "XYZ\nW");
        let updated = old.updated(edited.clone(), METRICS);
        let whole = LineIndex::new(edited, METRICS);
        assert_eq!(updated.len(), whole.len());
        for line in 0..whole.len() {
            assert_eq!(updated.line_bytes(line), whole.line_bytes(line));
        }
        assert_eq!(updated.total_graphemes(), whole.total_graphemes());
        assert!(updated.is_exact(0) && updated.height(0) == 33.0, "a line before the edit keeps its height");
        assert!(updated.is_exact(updated.len() - 2) && updated.height(updated.len() - 2) == 44.0, "and one after it");
        // Another wrapping starts over.
        let narrow = old.updated(TextSnapshot::from_text(before), LineMetrics { content_width: 50.0, ..METRICS });
        assert!(!narrow.is_exact(0));
    }
}
