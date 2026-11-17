//! Where every hard line of a long text is, how tall it is, and how many graphemes it has.
//!
//! Laying a long note out costs in proportion to the note, so a windowed field lays
//! out only the lines near the viewport. What it cannot lay out it still has to place:
//! the height of the note decides how far it scrolls, and the offset of a caret in a
//! line far from the window decides where that line is. This index answers both
//! without the layout.
//!
//! A line is a hard line: the text up to and including a line feed, so the text
//! `"a\nb\n"` has three lines, the last one empty, as a text area shows it. Each has
//!
//! * a **height**, estimated from its length until the line has been laid out and then
//!   exact (`refine`), the way a windowed list corrects what it guessed;
//! * a **grapheme count**, so a grapheme offset anywhere in the note finds its line
//!   in O(log n) and the note's grapheme count never needs a walk over the whole text.
//!
//! Heights and grapheme counts are kept in Fenwick trees, so the top of any line, the
//! line at any height and the line holding any grapheme are all O(log n), and a
//! correction is too.
//!
//! After an edit the index is made from the previous one ([`LineIndex::updated`]): the
//! lines before the first changed byte and after the last keep their heights and counts,
//! so only the lines the edit touched are estimated and counted again.

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

/// A Fenwick (binary indexed) tree of sums over a fixed number of slots.
#[derive(Clone)]
struct Fenwick<T> {
    tree: Vec<T>,
}

trait Slot: Copy + Default + PartialOrd + std::ops::AddAssign + std::ops::SubAssign {}
impl Slot for f64 {}
impl Slot for u64 {}

impl<T: Slot> Fenwick<T> {
    /// A tree over `values`, built in linear time.
    fn new(values: impl ExactSizeIterator<Item = T>) -> Self {
        let mut tree = Vec::with_capacity(values.len() + 1);
        tree.push(T::default());
        tree.extend(values);
        for index in 1..tree.len() {
            let parent = index + (index & index.wrapping_neg());
            if parent < tree.len() {
                let value = tree[index];
                tree[parent] += value;
            }
        }
        Self { tree }
    }

    /// The sum of the first `count` slots.
    fn prefix(&self, count: usize) -> T {
        let mut sum = T::default();
        let mut index = count.min(self.tree.len() - 1);
        while index > 0 {
            sum += self.tree[index];
            index &= index - 1;
        }
        sum
    }

    fn add(&mut self, slot: usize, delta: T, negative: bool) {
        let mut index = slot + 1;
        while index < self.tree.len() {
            if negative {
                self.tree[index] -= delta;
            } else {
                self.tree[index] += delta;
            }
            index += index & index.wrapping_neg();
        }
    }

    /// How many leading slots sum to at most `target`: the first slot that would take
    /// the sum past it is the one that holds `target`.
    fn lower_bound(&self, target: T) -> usize {
        let mut position = 0;
        let mut remaining = target;
        let mut step = (self.tree.len() - 1).next_power_of_two();
        while step > 0 {
            let next = position + step;
            if next < self.tree.len() && self.tree[next] <= remaining {
                remaining -= self.tree[next];
                position = next;
            }
            step >>= 1;
        }
        position
    }
}

/// What the estimate of a line's height is made from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineMetrics {
    /// The width a line wraps to, in physical pixels.
    pub content_width: f32,
    /// The height of one visual line.
    pub line_height: f32,
    /// How wide a grapheme is, on average. Learned from the lines laid out
    /// ([`LineIndex::observe`]); this is where it starts.
    pub advance: f32,
}

impl LineMetrics {
    /// Whether heights estimated for `self` are still estimates for `other`.
    fn same_layout(&self, other: &Self) -> bool {
        self.content_width == other.content_width && self.line_height == other.line_height
    }

    /// Whether a layout made for `self` wraps its lines as one made for `other` does.
    pub(crate) fn same_wrapping(&self, other: &Self) -> bool {
        self.same_layout(other)
    }
}

/// The height of a line of `graphemes` graphemes, were it wrapped at the width the
/// metrics give: at least one visual line.
fn estimate_height(graphemes: u32, metrics: &LineMetrics) -> f32 {
    // The line feed takes no room.
    let width = graphemes.saturating_sub(1) as f32 * metrics.advance;
    let visual = if metrics.content_width > 0.0 {
        (width / metrics.content_width).ceil().max(1.0)
    } else {
        1.0
    };
    visual * metrics.line_height
}

/// Everything about a line that a layout can correct.
struct HeightModel {
    heights: Vec<f32>,
    exact: Vec<bool>,
    tree: Fenwick<f64>,
    metrics: LineMetrics,
    /// Learning the advance: bytes of text and widths of the lines laid out.
    observed_graphemes: f64,
    observed_width: f64,
}

pub(crate) struct LineIndex {
    text: Arc<str>,
    /// Where each line starts, in bytes.
    starts: Vec<usize>,
    /// How many graphemes each line holds.
    graphemes: Vec<u32>,
    grapheme_tree: Fenwick<u64>,
    model: RefCell<HeightModel>,
    /// Counts the changes to heights, so a layout made of them can tell they moved.
    version: std::cell::Cell<u64>,
    /// The largest gap between lines that any line laid out has had. A layout's height
    /// has this added once, to the last line.
    line_gap: std::cell::Cell<f32>,
}

/// Whether every grapheme of `text` is a single byte: ASCII, but for a carriage return
/// before a line feed.
#[inline]
fn is_one_byte_graphemes(text: &str) -> bool {
    text.is_ascii() && !text.as_bytes().contains(&b'\r')
}

fn grapheme_count(line: &str) -> u32 {
    if is_one_byte_graphemes(line) {
        line.len() as u32
    } else {
        line.graphemes(true).count() as u32
    }
}

/// The start of every line of `text`.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut at = 0;
    while let Some(found) = text[at..].find('\n') {
        at += found + 1;
        starts.push(at);
    }
    starts
}

/// How many bytes `a` and `b` start with in common, a block at a time: comparing
/// slices is one `memcmp`, where a byte loop is slow in a debug build.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let limit = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= limit && a[at..at + BLOCK] == b[at..at + BLOCK] {
        at += BLOCK;
    }
    while at < limit && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many bytes `a` and `b` end with in common.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let limit = a.len().min(b.len());
    let mut len = 0;
    while len + BLOCK <= limit
        && a[a.len() - len - BLOCK..a.len() - len] == b[b.len() - len - BLOCK..b.len() - len]
    {
        len += BLOCK;
    }
    while len < limit && a[a.len() - 1 - len] == b[b.len() - 1 - len] {
        len += 1;
    }
    len
}

impl LineIndex {
    /// The index of `text`, every height estimated.
    pub(crate) fn new(text: Arc<str>, metrics: LineMetrics) -> Self {
        let starts = line_starts(&text);
        let graphemes = (0..starts.len())
            .map(|line| grapheme_count(&text[Self::range_in(&starts, text.len(), line)]))
            .collect::<Vec<_>>();
        Self::assemble(text, starts, graphemes, None, metrics)
    }

    /// The index of `text`, made from this one, `text` being this one's text edited:
    /// the lines before the first changed byte and after the last keep what this
    /// index knew of them, and the rest are estimated and counted anew.
    ///
    /// A change of `metrics` that decides how lines wrap discards what was learned of
    /// the heights, which were for another wrapping.
    pub(crate) fn updated(&self, text: Arc<str>, metrics: LineMetrics) -> Self {
        let old_metrics = self.model.borrow().metrics;
        if !old_metrics.same_layout(&metrics) {
            return Self::new(text, metrics);
        }
        if Arc::ptr_eq(&self.text, &text) || *self.text == *text {
            let kept = Self::new_like(self, text);
            kept.model.borrow_mut().metrics = metrics;
            return kept;
        }

        let old = self.text.as_bytes();
        let new = text.as_bytes();
        let prefix = common_prefix(old, new);
        let suffix = common_suffix(&old[prefix..], &new[prefix..]);
        let starts = line_starts(&text);

        // The lines of the old text that are the same lines of the new:
        // * those that end inside the common prefix, which are the first lines of both;
        // * those that start inside the common suffix, which are the last lines of both,
        //   so the old line of one is the new line's index moved by how many lines the
        //   edit added, and it is that line if it starts where the new one does, moved
        //   by how many bytes the edit added.
        let prefix_lines = starts.partition_point(|&start| start <= prefix).saturating_sub(1);
        let suffix_from = new.len() - suffix;
        let line_shift = self.starts.len() as isize - starts.len() as isize;
        let byte_shift = old.len() as isize - new.len() as isize;
        let reused_line = |line: usize, start: usize| -> Option<usize> {
            if line < prefix_lines {
                return Some(line);
            }
            if start < suffix_from {
                return None;
            }
            let old_line = usize::try_from(line as isize + line_shift).ok()?;
            let old_start = usize::try_from(start as isize + byte_shift).ok()?;
            (self.starts.get(old_line) == Some(&old_start)).then_some(old_line)
        };

        let model = self.model.borrow();
        // What has been learned of how wide a grapheme is outranks the caller's first guess.
        let metrics = if model.observed_graphemes > 0.0 {
            LineMetrics { advance: (model.observed_width / model.observed_graphemes) as f32, ..metrics }
        } else {
            metrics
        };
        let mut graphemes = Vec::with_capacity(starts.len());
        let mut heights = Vec::with_capacity(starts.len());
        let mut exact = Vec::with_capacity(starts.len());
        for (line, &start) in starts.iter().enumerate() {
            match reused_line(line, start) {
                Some(old_line) => {
                    graphemes.push(self.graphemes[old_line]);
                    heights.push(model.heights[old_line]);
                    exact.push(model.exact[old_line]);
                }
                None => {
                    let count = grapheme_count(&text[Self::range_in(&starts, text.len(), line)]);
                    graphemes.push(count);
                    heights.push(estimate_height(count, &metrics));
                    exact.push(false);
                }
            }
        }
        let learned = (model.observed_graphemes, model.observed_width);
        drop(model);
        let index = Self::assemble(text, starts, graphemes, Some((heights, exact)), metrics);
        index.line_gap.set(self.line_gap.get());
        {
            let mut model = index.model.borrow_mut();
            model.observed_graphemes = learned.0;
            model.observed_width = learned.1;
            if learned.0 > 0.0 {
                model.metrics.advance = (learned.1 / learned.0) as f32;
            }
        }
        index
    }

    fn new_like(old: &Self, text: Arc<str>) -> Self {
        let model = old.model.borrow();
        Self {
            text,
            starts: old.starts.clone(),
            graphemes: old.graphemes.clone(),
            grapheme_tree: old.grapheme_tree.clone(),
            version: std::cell::Cell::new(0),
            line_gap: std::cell::Cell::new(old.line_gap.get()),
            model: RefCell::new(HeightModel {
                heights: model.heights.clone(),
                exact: model.exact.clone(),
                tree: model.tree.clone(),
                metrics: model.metrics,
                observed_graphemes: model.observed_graphemes,
                observed_width: model.observed_width,
            }),
        }
    }

    fn assemble(
        text: Arc<str>,
        starts: Vec<usize>,
        graphemes: Vec<u32>,
        heights: Option<(Vec<f32>, Vec<bool>)>,
        metrics: LineMetrics,
    ) -> Self {
        let (heights, exact) = heights.unwrap_or_else(|| {
            let heights = graphemes.iter().map(|&count| estimate_height(count, &metrics)).collect::<Vec<_>>();
            let exact = vec![false; heights.len()];
            (heights, exact)
        });
        let tree = Fenwick::new(heights.iter().map(|&height| f64::from(height)));
        let grapheme_tree = Fenwick::new(graphemes.iter().map(|&count| u64::from(count)));
        Self {
            text,
            starts,
            graphemes,
            grapheme_tree,
            version: std::cell::Cell::new(0),
            line_gap: std::cell::Cell::new(0.0),
            model: RefCell::new(HeightModel {
                heights,
                exact,
                tree,
                metrics,
                observed_graphemes: 0.0,
                observed_width: 0.0,
            }),
        }
    }

    fn range_in(starts: &[usize], text_len: usize, line: usize) -> Range<usize> {
        starts[line]..starts.get(line + 1).copied().unwrap_or(text_len)
    }

    /// Notes the gap between lines of a line that was laid out; see [`Self::line_gap`].
    pub(crate) fn observe_line_gap(&self, gap: f32) {
        if gap.is_finite() && gap > self.line_gap.get() {
            self.line_gap.set(gap);
            self.version.set(self.version.get() + 1);
        }
    }

    /// The largest gap between lines that any line laid out has had.
    #[inline]
    pub(crate) fn line_gap(&self) -> f32 {
        self.line_gap.get()
    }

    /// How many times a height has changed since the index was made: a layout placed with
    /// the heights as they were is stale when this has moved.
    #[inline]
    pub(crate) fn version(&self) -> u64 {
        self.version.get()
    }

    /// The text the index is of.
    #[inline]
    pub(crate) fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// How many lines there are: one more than the line feeds, so a text that ends in
    /// one ends in an empty line.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }

    /// The bytes of `line`, with its line feed.
    #[inline]
    pub(crate) fn line_bytes(&self, line: usize) -> Range<usize> {
        Self::range_in(&self.starts, self.text.len(), line)
    }

    /// The line a byte offset is in; the end of the text is in the last line.
    pub(crate) fn line_of_byte(&self, byte: usize) -> usize {
        self.starts.partition_point(|&start| start <= byte).saturating_sub(1)
    }

    /// The top of `line`, in the height the lines above it add up to.
    pub(crate) fn top(&self, line: usize) -> f64 {
        self.model.borrow().tree.prefix(line)
    }

    #[inline]
    pub(crate) fn height(&self, line: usize) -> f32 {
        self.model.borrow().heights[line]
    }

    #[inline]
    pub(crate) fn is_exact(&self, line: usize) -> bool {
        self.model.borrow().exact[line]
    }

    /// The height of the whole text.
    pub(crate) fn total_height(&self) -> f64 {
        let model = self.model.borrow();
        model.tree.prefix(model.heights.len())
    }

    /// The line that holds the height `y`: the first line for a negative one, the last
    /// for one past the end.
    pub(crate) fn line_at_y(&self, y: f64) -> usize {
        let model = self.model.borrow();
        model.tree.lower_bound(y.max(0.0)).min(model.heights.len() - 1)
    }

    /// The lines that overlap the heights from `top` to `bottom`.
    pub(crate) fn lines_between(&self, top: f64, bottom: f64) -> Range<usize> {
        let first = self.line_at_y(top);
        let last = self.line_at_y(bottom.max(top));
        first..last + 1
    }

    /// Records the height `line` was laid out to, which is exact.
    pub(crate) fn refine(&self, line: usize, height: f32) {
        let mut model = self.model.borrow_mut();
        let previous = model.heights[line];
        if previous != height {
            let delta = f64::from(height) - f64::from(previous);
            model.tree.add(line, delta.abs(), delta < 0.0);
            model.heights[line] = height;
            self.version.set(self.version.get() + 1);
        }
        model.exact[line] = true;
    }

    /// Learns how wide a grapheme is from a line that was laid out: `graphemes` of them
    /// took `width` in one visual line.
    pub(crate) fn observe(&self, graphemes: usize, width: f32) {
        if graphemes == 0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        let mut model = self.model.borrow_mut();
        model.observed_graphemes += graphemes as f64;
        model.observed_width += f64::from(width);
        model.metrics.advance = (model.observed_width / model.observed_graphemes) as f32;
    }

    /// The metrics heights are estimated by now, with the advance as learned.
    pub(crate) fn metrics(&self) -> LineMetrics {
        self.model.borrow().metrics
    }

    /// How many graphemes the text has.
    pub(crate) fn total_graphemes(&self) -> usize {
        self.grapheme_tree.prefix(self.graphemes.len()) as usize
    }

    /// How many graphemes come before `line`.
    pub(crate) fn graphemes_before(&self, line: usize) -> usize {
        self.grapheme_tree.prefix(line) as usize
    }

    /// The line a grapheme offset is in, and how many graphemes of the line come before
    /// it. The offset one past the last grapheme is the end of the last line.
    pub(crate) fn line_of_grapheme(&self, offset: usize) -> (usize, usize) {
        let line = self
            .grapheme_tree
            .lower_bound(offset as u64)
            .min(self.graphemes.len() - 1);
        (line, offset.saturating_sub(self.graphemes_before(line)))
    }

    /// The grapheme offset of a byte offset on a grapheme boundary.
    pub(crate) fn grapheme_of_byte(&self, byte: usize) -> usize {
        let byte = byte.min(self.text.len());
        let line = self.line_of_byte(byte);
        let start = self.starts[line];
        self.graphemes_before(line) + grapheme_count(&self.text[start..byte]) as usize
    }

    /// The byte offset of a grapheme offset.
    pub(crate) fn byte_of_grapheme(&self, offset: usize) -> usize {
        let (line, local) = self.line_of_grapheme(offset);
        let range = self.line_bytes(line);
        let line_text = &self.text[range.clone()];
        if local == 0 {
            return range.start;
        }
        if is_one_byte_graphemes(line_text) {
            return range.start + local.min(line_text.len());
        }
        line_text
            .grapheme_indices(true)
            .nth(local)
            .map_or(range.end, |(byte, _)| range.start + byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: LineMetrics = LineMetrics { content_width: 100.0, line_height: 20.0, advance: 10.0 };

    fn index(text: &str) -> LineIndex {
        LineIndex::new(Arc::from(text), METRICS)
    }

    /// Hard lines the plain way: split after every line feed, with an empty line after a
    /// last one.
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
    fn lines_and_grapheme_counts_are_those_of_the_text() {
        for text in TEXTS {
            let index = index(text);
            let lines = naive_lines(text);
            assert_eq!(index.len(), lines.len(), "{text:?}: lines");
            let mut start = 0;
            for (number, line) in lines.iter().enumerate() {
                assert_eq!(index.line_bytes(number), start..start + line.len(), "{text:?}: line {number}");
                assert_eq!(
                    index.graphemes_before(number + 1) - index.graphemes_before(number),
                    line.graphemes(true).count(),
                    "{text:?}: graphemes of line {number}"
                );
                start += line.len();
            }
            assert_eq!(index.total_graphemes(), text.graphemes(true).count(), "{text:?}: all graphemes");
        }
    }

    #[test]
    fn offsets_convert_between_bytes_and_graphemes() {
        for text in TEXTS {
            let index = index(text);
            let boundaries = text
                .grapheme_indices(true)
                .map(|(byte, _)| byte)
                .chain([text.len()])
                .collect::<Vec<_>>();
            for (grapheme, &byte) in boundaries.iter().enumerate() {
                assert_eq!(index.byte_of_grapheme(grapheme), byte, "{text:?}: byte of grapheme {grapheme}");
                assert_eq!(index.grapheme_of_byte(byte), grapheme, "{text:?}: grapheme of byte {byte}");
                let (line, local) = index.line_of_grapheme(grapheme);
                assert_eq!(index.graphemes_before(line) + local, grapheme);
                assert!(index.line_bytes(line).start <= byte && byte <= index.line_bytes(line).end);
            }
            for byte in 0..=text.len() {
                let line = index.line_of_byte(byte);
                let range = index.line_bytes(line);
                assert!(range.start <= byte && (byte < range.end || line + 1 == index.len()), "{text:?}: byte {byte}");
            }
        }
    }

    #[test]
    fn heights_add_up_and_a_correction_moves_every_line_below_it() {
        let text = "a\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\nccc\n";
        let index = index(text);
        // Ten graphemes a visual line: a short line is one, the long one three.
        let naive = |index: &LineIndex| (0..index.len()).map(|line| f64::from(index.height(line))).collect::<Vec<_>>();
        let check = |index: &LineIndex, what: &str| {
            let heights = naive(index);
            let mut top = 0.0;
            for (line, height) in heights.iter().enumerate() {
                assert!((index.top(line) - top).abs() < 1e-6, "{what}: top of {line}");
                for probe in [top, top + height / 2.0, top + height - 0.01] {
                    assert_eq!(index.line_at_y(probe), line, "{what}: line at {probe}");
                }
                top += height;
            }
            assert!((index.total_height() - top).abs() < 1e-6, "{what}: total");
            assert_eq!(index.line_at_y(-5.0), 0, "{what}: above");
            assert_eq!(index.line_at_y(top + 500.0), index.len() - 1, "{what}: below");
        };
        check(&index, "estimated");
        assert!(!index.is_exact(1));
        assert_eq!(index.height(0), 20.0);
        assert_eq!(index.height(1), 60.0);

        index.refine(1, 140.0);
        index.refine(0, 20.0);
        assert!(index.is_exact(1) && index.is_exact(0) && !index.is_exact(2));
        assert_eq!(index.height(1), 140.0);
        check(&index, "corrected");
        index.refine(1, 40.0);
        check(&index, "corrected again");
        // Heights are now 20, 40, 20, 20: the second line is 20..60 and the third 60..80.
        assert_eq!(index.lines_between(25.0, 70.0), 1..3);
    }

    /// A small deterministic generator: the tests are the same on every run.
    struct Random(u64);
    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    fn piece(random: &mut Random) -> &'static str {
        const PIECES: [&str; 12] =
            ["a", "bc", " ", "\n", "\n", "\r\n", "\u{e9}", "e\u{301}", "\u{4f60}", "\u{1f468}\u{200d}\u{1f469}", "word ", "\t"];
        PIECES[random.next(PIECES.len())]
    }

    fn random_text(random: &mut Random, pieces: usize) -> String {
        (0..pieces).map(|_| piece(random)).collect()
    }

    /// An edit at a grapheme boundary: pieces inserted, removed, or both.
    fn random_edit(random: &mut Random, text: &str) -> String {
        let boundaries = text.grapheme_indices(true).map(|(byte, _)| byte).chain([text.len()]).collect::<Vec<_>>();
        let from = boundaries[random.next(boundaries.len())];
        let to = boundaries[random.next(boundaries.len())].max(from);
        let to = if random.next(3) == 0 { to } else { from.max(to.min(from + random.next(6))) };
        let to = boundaries.iter().copied().find(|&byte| byte >= to).unwrap_or(text.len());
        let count = random.next(4);
        let inserted = (0..count).map(|_| piece(random)).collect::<String>();
        format!("{}{inserted}{}", &text[..from], &text[to..])
    }

    #[test]
    fn an_updated_index_is_the_index_of_the_edited_text() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for round in 0..300 {
            let pieces = 2 + random.next(60);
            let before = random_text(&mut random, pieces);
            let after = random_edit(&mut random, &before);
            let old = index(&before);
            let updated = old.updated(Arc::from(after.as_str()), METRICS);
            let fresh = index(&after);

            assert_eq!(updated.len(), fresh.len(), "round {round}: {before:?} -> {after:?}");
            for line in 0..fresh.len() {
                assert_eq!(updated.line_bytes(line), fresh.line_bytes(line), "round {round}: line {line}");
                assert_eq!(
                    updated.graphemes_before(line),
                    fresh.graphemes_before(line),
                    "round {round}: graphemes before line {line}"
                );
                assert_eq!(updated.height(line), fresh.height(line), "round {round}: height of line {line}");
            }
            assert_eq!(updated.total_graphemes(), fresh.total_graphemes());
            assert!((updated.total_height() - fresh.total_height()).abs() < 1e-3);
        }
    }

    #[test]
    fn an_edit_keeps_what_was_learned_of_the_lines_it_did_not_touch() {
        let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
        for round in 0..200 {
            let pieces = 10 + random.next(80);
            let before = random_text(&mut random, pieces);
            let old = index(&before);
            // A height that is a function of the line's own text, so any line that is
            // still exact after an edit shows whether it is still the line it was.
            let marker = |text: &str| 1000.0 + text.len() as f32 + text.matches('\n').count() as f32 * 0.5;
            for line in 0..old.len() {
                old.refine(line, marker(&before[old.line_bytes(line)]));
            }
            let after = random_edit(&mut random, &before);
            let updated = old.updated(Arc::from(after.as_str()), METRICS);

            let mut kept = 0;
            for line in 0..updated.len() {
                if updated.is_exact(line) {
                    kept += 1;
                    assert_eq!(
                        updated.height(line),
                        marker(&after[updated.line_bytes(line)]),
                        "round {round}: line {line} of {after:?} is exact but is not the line that was measured"
                    );
                }
            }
            // The lines the edit cannot have touched are kept: all but the few around it.
            let line_feeds_changed = (before.matches('\n').count() as isize - after.matches('\n').count() as isize).unsigned_abs();
            assert!(
                kept + 4 + line_feeds_changed >= updated.len().min(old.len()),
                "round {round}: only {kept} of {} lines kept, {before:?} -> {after:?}",
                updated.len()
            );
        }
    }

    #[test]
    fn a_change_of_wrapping_forgets_the_heights_and_keeps_the_text() {
        let text = "some words that wrap\nand more\n";
        let old = index(text);
        old.refine(0, 77.0);
        let wider = LineMetrics { content_width: 400.0, ..METRICS };
        let updated = old.updated(Arc::from(text), wider);
        assert!(!updated.is_exact(0), "the height was for another width");
        assert_eq!(updated.height(0), 20.0);
        assert_eq!(updated.len(), old.len());

        // The same width and the same text keeps everything.
        let same = old.updated(Arc::from(text), METRICS);
        assert!(same.is_exact(0));
        assert_eq!(same.height(0), 77.0);
    }

    #[test]
    fn the_advance_is_learned_from_the_lines_that_were_laid_out() {
        let index = index("abcdefghij\nabcdefghij\n");
        assert_eq!(index.metrics().advance, 10.0);
        index.observe(10, 55.0);
        index.observe(10, 45.0);
        assert!((index.metrics().advance - 5.0).abs() < 1e-6);

        // An edit starts from what was learned, and estimates with it: ten graphemes
        // of five pixels a visual line is half a line.
        let updated = index.updated(Arc::from("abcdefghij\nabcdefghij\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"), METRICS);
        assert!((updated.metrics().advance - 5.0).abs() < 1e-6);
        assert_eq!(updated.height(2), 40.0, "36 graphemes at 5px over a width of 100 is two visual lines");
    }
}
