//! Where every unit of a long text is, how tall it is and how many graphemes it has.
//!
//! A unit is a hard line, the text up to and including a line feed, or a segment of a hard line
//! too long to lay out whole (see `layout_cuts`). A text ending in a line feed ends in an empty
//! unit, as a text area shows it, and an empty text is one empty unit.
//!
//! Laying a long note out costs in proportion to the note, so a windowed editor lays out the
//! units near the viewport. What it cannot lay out it still has to place: the height of the note
//! is how far it scrolls, and the offset of a caret in a unit far from the window is where that
//! unit is. This index answers both without the layout. A height is estimated from the unit's
//! grapheme count until the unit has been laid out, and is then exact ([`refine`]), as a windowed
//! list corrects what it guessed. After an edit only the units it touches are made again
//! ([`apply_edit`]), whatever the size of the text.
//!
//! [`refine`]: LayoutIndex::refine
//! [`apply_edit`]: LayoutIndex::apply_edit

use std::ops::Range;

use aimer_sum_tree::{Bias, SumTree};

use crate::buffer::EditDelta;
use aimer_simd::kernels::find_byte;

use crate::layout_cuts::{grapheme_count, grapheme_count_of, unit_end};
use crate::layout_units::{ByteStart, GraphemeStart, HardLines, Top, Unit};
use crate::LINE_LIMIT;
use crate::rope::TextSnapshot;

/// What the heights of units are estimated from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutMetrics {
    /// The width a unit wraps to, in physical pixels.
    pub content_width: f32,
    /// The height of one visual line.
    pub line_height: f32,
    /// How wide a grapheme is, on average. Learned from the units laid out
    /// ([`LayoutIndex::observe`]); this is where it starts.
    pub advance: f32,
}

impl LayoutMetrics {
    /// Whether heights estimated for `self` are still estimates for `other`: the wrapping is the same.
    pub fn same_wrapping(&self, other: &Self) -> bool {
        self.content_width == other.content_width && self.line_height == other.line_height
    }
}

/// The height of a unit of `graphemes` graphemes, were it wrapped at the width the metrics give:
/// at least one visual line.
fn estimate_height(graphemes: u32, feed: bool, metrics: &LayoutMetrics) -> f32 {
    let visible = graphemes.saturating_sub(u32::from(feed));
    let width = visible as f32 * metrics.advance;
    let visual = if metrics.content_width > 0.0 { (width / metrics.content_width).ceil().max(1.0) } else { 1.0 };
    visual * metrics.line_height
}

/// What [`LayoutIndex::apply_edit`] did: how many units it took out and how many it put in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reindexed {
    /// Units that were replaced.
    pub removed: usize,
    /// Units that replaced them.
    pub added: usize,
}

/// The units of a text, with their heights and grapheme counts; see the module documentation.
#[derive(Clone)]
pub struct LayoutIndex {
    units: SumTree<Unit>,
    metrics: LayoutMetrics,
    observed_graphemes: f64,
    observed_width: f64,
    /// Counts the changes to heights, so a layout made of them can tell they moved.
    version: u64,
    /// The largest gap between lines that any unit laid out has had.
    line_gap: f32,
}

impl LayoutIndex {
    /// An index of no text.
    pub fn new(metrics: LayoutMetrics) -> Self {
        Self { units: SumTree::new(), metrics, observed_graphemes: 0.0, observed_width: 0.0, version: 0, line_gap: 0.0 }
    }

    /// The index of all of `text`, every height estimated. O(n): see
    /// [`index_more`](Self::index_more) to do it a part at a time.
    pub fn build(text: &TextSnapshot, metrics: LayoutMetrics) -> Self {
        let mut index = Self::new(metrics);
        // A slice at a time: the units of a slice are put in the tree before the next are made,
        // so the whole text's units are never held twice over at once.
        while !index.index_more(text, 1024 * 1024) {}
        index
    }

    /// Indexes the text after what is indexed, up to `byte_budget` bytes more (a unit is never
    /// cut off, so it can be a little more), and returns whether all of `text` is indexed now.
    /// This is how a text that is still loading, or too large to index in one frame, is indexed.
    pub fn index_more(&mut self, text: &TextSnapshot, byte_budget: usize) -> bool {
        let len = text.len();
        let mut pos = self.indexed_bytes();
        if self.is_complete(text) {
            return true;
        }
        // The last unit of the text as it was may have been cut by the end of the text and not
        // by its own end: a last line with no line feed, or the empty line after one. It is
        // made again, as the text now goes on.
        if let Some(last) = self.units.len().checked_sub(1)
            && self.units.get(last).is_some_and(|unit| unit.ends_line && !unit.feed)
        {
            pos -= self.units.get(last).map_or(0, |unit| unit.bytes);
            self.units.replace_range(last..last + 1, []);
        }
        let stop = pos.saturating_add(byte_budget);
        let mut added = Vec::new();
        // The chunk the walk is in: most lines lie inside one, and are read from it without a
        // search of the tree for each.
        let mut chunk: Option<(&str, usize)> = None;
        loop {
            let (end, unit) = self.next_unit(text, pos, &mut chunk);
            added.push(unit);
            let ended_in_feed = added.last().is_some_and(|unit| unit.feed);
            pos = end;
            if end == len {
                if ended_in_feed {
                    added.push(self.make_unit(text, len..len, true));
                }
                break;
            }
            if pos >= stop {
                break;
            }
        }
        self.units.extend(added);
        self.version += 1;
        self.is_complete(text)
    }

    /// Whether every byte of `text` is indexed.
    pub fn is_complete(&self, text: &TextSnapshot) -> bool {
        // A text ends in a unit with no line feed (the last line), or in the empty unit after one.
        self.indexed_bytes() == text.len() && self.units.get(self.units.len().wrapping_sub(1)).is_some_and(|unit| unit.ends_line)
    }

    /// How many bytes of the text are indexed.
    #[inline]
    pub fn indexed_bytes(&self) -> usize {
        self.units.summary().bytes
    }

    /// The unit that starts at `pos`, and where it ends. A line with its line feed inside the
    /// chunk `chunk` (found again when `pos` has left it) is made from the chunk; any other unit is
    /// made as [`unit_end`] finds it.
    fn next_unit<'a>(&self, text: &'a TextSnapshot, pos: usize, chunk: &mut Option<(&'a str, usize)>) -> (usize, Unit) {
        let len = text.len();
        if pos < len {
            let here = match *chunk {
                Some((held, start)) if pos >= start && pos < start + held.len() => Some((held, start)),
                _ => text.chunk_containing(pos),
            };
            *chunk = here;
            if let Some((held, start)) = here
                && let Some(found) = find_byte(&held.as_bytes()[pos - start..], b'\n')
            {
                // A line feed inside a chunk is within `LINE_LIMIT`: a chunk is never larger.
                let end = pos + found + 1;
                let line = &held[pos - start..end - start];
                let graphemes = grapheme_count_of(line);
                return (
                    end,
                    Unit {
                        bytes: line.len(),
                        graphemes,
                        height: estimate_height(graphemes, true, &self.metrics),
                        exact: false,
                        feed: true,
                        ends_line: true,
                    },
                );
            }
        }
        let end = unit_end(text, pos);
        (end, self.make_unit(text, pos..end, end == len))
    }

    fn make_unit(&self, text: &TextSnapshot, range: Range<usize>, last: bool) -> Unit {
        let feed = !range.is_empty() && text.byte_at(range.end - 1) == Some(b'\n');
        let graphemes = grapheme_count(text, range.clone());
        Unit {
            bytes: range.len(),
            graphemes,
            height: estimate_height(graphemes, feed, &self.metrics),
            exact: false,
            feed,
            ends_line: feed || last,
        }
    }

    /// Makes the index of `text` again from the units the edit `delta` touched, `text` being the
    /// text after it; the units before them and after them keep their heights and counts.
    ///
    /// The units are made again from the start of the one that holds the edit, to the first
    /// boundary after the edit that is also a boundary of the old units (moved by what the edit
    /// added), where what follows is the old units again. For lines of ordinary length that is
    /// the end of the line, and for a very long line the next cut the text picks after the
    /// edit, a few thousand bytes on at the most.
    pub fn apply_edit(&mut self, delta: &EditDelta, text: &TextSnapshot) -> Reindexed {
        let old_indexed = self.indexed_bytes();
        let range = delta.range();
        if range.start > old_indexed {
            return Reindexed { removed: 0, added: 0 };
        }
        // The unit that holds the start of the edit: the one it appends to at the end.
        let mut first = self.units.seek(&ByteStart(range.start), Bias::Right).index;
        if first == self.units.len() {
            let last = first.wrapping_sub(1);
            first = if self.units.get(last).is_some_and(|unit| unit.ends_line && !unit.feed) { last } else { first };
        }
        // A segment is cut, or not, by whether a line feed is within `LINE_LIMIT` bytes after its
        // start, so the segments in that reach before the edit may be cut differently now.
        while first > 0 && self.units.get(first - 1).is_some_and(|unit| !unit.feed) {
            let previous = self.units.dimension_before::<ByteStart>(first - 1).0;
            if previous + LINE_LIMIT + 1 <= range.start {
                break;
            }
            first -= 1;
        }
        let from: usize = self.units.dimension_before::<ByteStart>(first).0;
        // An edit that reaches past what is indexed drops the rest, to be indexed again.
        let reaches_unindexed = range.end > old_indexed;
        let edit_end = range.start + delta.inserted().len();
        let change = delta.len_change();
        let len = text.len();
        let was_complete = old_indexed == (len as isize - change) as usize;
        // Where the old indexed text ends, in the text after the edit.
        let stop_at = if was_complete { usize::MAX } else { ((old_indexed as isize + change).max(edit_end as isize)) as usize };
        let mut pos = from;
        let mut added = Vec::new();
        let mut resync = self.units.len();
        loop {
            let end = unit_end(text, pos);
            added.push(self.make_unit(text, pos..end, end == len));
            let ended_in_feed = added.last().is_some_and(|unit| unit.feed);
            pos = end;
            if end == len {
                if ended_in_feed {
                    added.push(self.make_unit(text, len..len, true));
                }
                break;
            }
            // An index of part of the text goes on only as far as it did.
            if pos >= stop_at {
                break;
            }
            // Where the text is the old text again, a boundary that was a boundary of the old units
            // starts what the old units were: how a unit ends depends on the text from its start on.
            if !reaches_unindexed && pos >= edit_end {
                let old_pos = (pos as isize - change) as usize;
                let at = self.units.seek(&ByteStart(old_pos), Bias::Right);
                if at.index < self.units.len() && at.start.0 == old_pos && old_pos > range.end {
                    resync = at.index;
                    break;
                }
            }
        }
        // Old units run to the end of the old text unless the walk found where they begin again.
        let removed = resync - first;
        let count = added.len();
        self.units.replace_range(first..resync, added);
        self.version += 1;
        Reindexed { removed, added: count }
    }

    /// Changes what heights are estimated from. Heights estimated for another wrapping are
    /// estimated again, and those laid out are kept only when the wrapping is the same.
    pub fn set_metrics(&mut self, metrics: LayoutMetrics) {
        let wrapping_changed = !self.metrics.same_wrapping(&metrics);
        self.metrics = LayoutMetrics {
            advance: if self.observed_graphemes > 0.0 { (self.observed_width / self.observed_graphemes) as f32 } else { metrics.advance },
            ..metrics
        };
        if !wrapping_changed {
            return;
        }
        let metrics = self.metrics;
        self.units = self
            .units
            .iter()
            .map(|unit| Unit { height: estimate_height(unit.graphemes, unit.feed, &metrics), exact: false, ..unit.clone() })
            .collect();
        self.version += 1;
    }

    /// The metrics heights are estimated by now, with the advance as learned.
    pub fn metrics(&self) -> LayoutMetrics {
        self.metrics
    }

    /// How many units there are.
    #[inline]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether nothing is indexed.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// How many times a height has changed since the index was made: a layout placed with the
    /// heights as they were is stale when this has moved.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The bytes of unit `unit`.
    pub fn unit_bytes(&self, unit: usize) -> Range<usize> {
        let start = self.units.dimension_before::<ByteStart>(unit).0;
        start..start + self.units.get(unit).map_or(0, |unit| unit.bytes)
    }

    /// The unit that holds the byte at `byte`: the last unit for the end of the text.
    pub fn unit_of_byte(&self, byte: usize) -> usize {
        self.units.seek(&ByteStart(byte), Bias::Right).index.min(self.units.len().saturating_sub(1))
    }

    /// How many hard lines come before unit `unit`: its line number, if it starts one.
    pub fn hard_lines_before(&self, unit: usize) -> usize {
        self.units.dimension_before::<HardLines>(unit).0
    }

    /// How many hard lines there are.
    pub fn hard_line_count(&self) -> usize {
        self.units.summary().hard_lines
    }

    /// The height from the top of the text to the top of unit `unit`.
    pub fn top(&self, unit: usize) -> f64 {
        self.units.dimension_before::<Top>(unit).0
    }

    /// The height of unit `unit` (0 past the last).
    pub fn height(&self, unit: usize) -> f32 {
        self.units.get(unit).map_or(0.0, |unit| unit.height)
    }

    /// Whether the height of `unit` was laid out and not estimated.
    pub fn is_exact(&self, unit: usize) -> bool {
        self.units.get(unit).is_some_and(|unit| unit.exact)
    }

    /// The height of the text.
    pub fn total_height(&self) -> f64 {
        self.units.summary().height
    }

    /// The unit that holds the height `y`: the first for a negative one, the last for one past the end.
    pub fn unit_at_y(&self, y: f64) -> usize {
        self.units.seek(&Top(y.max(0.0)), Bias::Right).index.min(self.units.len().saturating_sub(1))
    }

    /// The units that overlap the heights from `top` to `bottom`.
    pub fn units_between(&self, top: f64, bottom: f64) -> Range<usize> {
        self.unit_at_y(top)..self.unit_at_y(bottom.max(top)) + 1
    }

    /// Records the height unit `unit` was laid out to, which is exact.
    pub fn refine(&mut self, unit: usize, height: f32) {
        let mut moved = false;
        self.units.update(unit, |unit| {
            moved = unit.height != height;
            unit.height = height;
            unit.exact = true;
        });
        if moved {
            self.version += 1;
        }
    }

    /// Learns how wide a grapheme is from a unit that was laid out: `graphemes` of them took
    /// `width` in one visual line.
    pub fn observe(&mut self, graphemes: usize, width: f32) {
        if graphemes == 0 || !width.is_finite() || width <= 0.0 {
            return;
        }
        self.observed_graphemes += graphemes as f64;
        self.observed_width += f64::from(width);
        self.metrics.advance = (self.observed_width / self.observed_graphemes) as f32;
    }

    /// Notes the gap between lines of a unit that was laid out; see [`line_gap`](Self::line_gap).
    pub fn observe_line_gap(&mut self, gap: f32) {
        if gap.is_finite() && gap > self.line_gap {
            self.line_gap = gap;
            self.version += 1;
        }
    }

    /// The largest gap between lines that any unit laid out has had. A layout's height has this
    /// added once, to the last line.
    pub fn line_gap(&self) -> f32 {
        self.line_gap
    }

    /// How many graphemes the indexed text has.
    pub fn total_graphemes(&self) -> usize {
        self.units.summary().graphemes
    }

    /// How many graphemes come before unit `unit`.
    pub fn graphemes_before(&self, unit: usize) -> usize {
        self.units.dimension_before::<GraphemeStart>(unit).0
    }

    /// The unit a grapheme offset is in, and how many of its graphemes come before it. The offset
    /// one past the last grapheme is the end of the last unit.
    pub fn unit_of_grapheme(&self, offset: usize) -> (usize, usize) {
        let position = self.units.seek(&GraphemeStart(offset), Bias::Right);
        let unit = position.index.min(self.units.len().saturating_sub(1));
        let before = if position.index == unit { position.start.0 } else { self.graphemes_before(unit) };
        (unit, offset.saturating_sub(before))
    }

    /// The graphemes unit `unit` has.
    pub fn unit_graphemes(&self, unit: usize) -> usize {
        self.units.get(unit).map_or(0, |unit| unit.graphemes as usize)
    }

    /// Whether unit `unit` ends a hard line.
    pub fn ends_line(&self, unit: usize) -> bool {
        self.units.get(unit).is_some_and(|unit| unit.ends_line)
    }

    /// The units as (bytes, graphemes, ends a hard line), for tests.
    #[doc(hidden)]
    pub fn shape(&self) -> Vec<(usize, u32, bool)> {
        self.units.iter().map(|unit| (unit.bytes, unit.graphemes, unit.ends_line)).collect()
    }

    /// Panics unless the tree is sound. For tests.
    #[doc(hidden)]
    pub fn check_invariants(&self) {
        self.units.check_invariants();
    }
}
