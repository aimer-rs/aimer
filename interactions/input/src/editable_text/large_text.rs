//! The text of a value when it is long: a rope, and the grapheme counts of its parts.
//!
//! A cursor is counted in graphemes and an edit is made in bytes, so a long text needs to be
//! asked where grapheme `n` is without a walk from the start. ASCII text, the common case,
//! needs nothing: a grapheme is a byte. Other text is indexed by the units of an
//! [`aimer_text_buffer::LayoutIndex`] (lines and segments of long lines, each with its grapheme
//! count), made the first time a grapheme is asked for and carried across edits by
//! re-indexing only the units an edit touched.

use std::sync::{Arc, OnceLock};

use aimer_text_buffer::{EditDelta, LayoutIndex, LayoutMetrics, TextSnapshot};
use unicode_segmentation::UnicodeSegmentation;

use super::graphemes::is_one_byte_graphemes;

/// The size from which a text is kept as a rope: a short text is cheaper as it is, a long one
/// is cheaper as a tree. Under test it is small, and `AIMER_LARGE_TEXT_BYTES` sets it (zero makes
/// every text a rope, which runs the whole suite on the rope).
pub(crate) fn large_text_bytes() -> usize {
    #[cfg(test)]
    {
        static LIMIT: OnceLock<usize> = OnceLock::new();
        *LIMIT.get_or_init(|| std::env::var("AIMER_LARGE_TEXT_BYTES").ok().and_then(|value| value.parse().ok()).unwrap_or(4096))
    }
    #[cfg(not(test))]
    {
        256 * 1024
    }
}

/// What the grapheme index needs of its metrics: nothing, since heights are not used.
const NO_METRICS: LayoutMetrics = LayoutMetrics { content_width: 0.0, line_height: 1.0, advance: 1.0 };

#[derive(Clone)]
pub(crate) struct LargeText {
    snapshot: TextSnapshot,
    /// The grapheme counts of its parts, made when first needed, and not at all for text of
    /// one-byte graphemes.
    units: Arc<OnceLock<LayoutIndex>>,
}

impl LargeText {
    pub(crate) fn new(text: &str) -> Self {
        Self::from_snapshot(TextSnapshot::from_text(text))
    }

    pub(crate) fn from_snapshot(snapshot: TextSnapshot) -> Self {
        Self { snapshot, units: Arc::new(OnceLock::new()) }
    }

    #[inline]
    pub(crate) fn snapshot(&self) -> &TextSnapshot {
        &self.snapshot
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.snapshot.len()
    }

    fn is_one_byte(&self) -> bool {
        self.snapshot.is_ascii() && !self.snapshot.has_carriage_return()
    }

    fn units(&self) -> &LayoutIndex {
        self.units.get_or_init(|| LayoutIndex::build(&self.snapshot, NO_METRICS))
    }

    pub(crate) fn grapheme_count(&self) -> usize {
        if self.is_one_byte() { self.len() } else { self.units().total_graphemes() }
    }

    pub(crate) fn byte_of_grapheme(&self, offset: usize) -> usize {
        if self.is_one_byte() {
            return offset.min(self.len());
        }
        byte_of_grapheme_in(&self.snapshot, self.units(), offset)
    }

    pub(crate) fn grapheme_of_byte(&self, byte: usize) -> usize {
        let byte = byte.min(self.len());
        if self.is_one_byte() {
            return byte;
        }
        grapheme_of_byte_in(&self.snapshot, self.units(), byte)
    }

    /// The text with `range` replaced, and the grapheme counts carried across the edit when they
    /// were made.
    pub(crate) fn spliced(&self, range: std::ops::Range<usize>, replacement: &str) -> Self {
        let snapshot = self.snapshot.with_replaced(range.clone(), replacement);
        self.edited(range, snapshot, replacement.len())
    }

    /// [`spliced`](Self::spliced) with a replacement that is already a snapshot, whose chunks the
    /// result shares: a paste of megabytes is not copied.
    pub(crate) fn spliced_snapshot(&self, range: std::ops::Range<usize>, with: &TextSnapshot) -> Self {
        let snapshot = self.snapshot.with_replaced_snapshot(range.clone(), with);
        self.edited(range, snapshot, with.len())
    }

    /// The text `snapshot`, which is this one with `range` replaced by `inserted` bytes.
    fn edited(&self, range: std::ops::Range<usize>, snapshot: TextSnapshot, inserted: usize) -> Self {
        let units = OnceLock::new();
        // Counts that exist are carried, and one that the edit made necessary is left to be made.
        if let Some(old) = self.units.get()
            && !(snapshot.is_ascii() && !snapshot.has_carriage_return())
        {
            let mut index = old.clone();
            let delta = EditDelta::new(
                range.clone(),
                self.snapshot.slice_snapshot(range.clone()),
                snapshot.slice_snapshot(range.start..range.start + inserted),
            );
            index.apply_edit(&delta, &snapshot);
            let _ = units.set(index);
        }
        Self { snapshot, units: Arc::new(units) }
    }
}

/// The byte offset of grapheme `offset` of `snapshot`, found by the units of `units` (an index of
/// it): the unit holding the grapheme, then a walk over that unit alone.
pub(crate) fn byte_of_grapheme_in(snapshot: &TextSnapshot, units: &LayoutIndex, offset: usize) -> usize {
    let (unit, local) = units.unit_of_grapheme(offset);
    let range = units.unit_bytes(unit);
    if local == 0 {
        return range.start;
    }
    let text = snapshot.slice(range.clone());
    if is_one_byte_graphemes(&text) {
        return range.start + local.min(text.len());
    }
    text.grapheme_indices(true).nth(local).map_or(range.end, |(at, _)| range.start + at)
}

/// How many graphemes come before the byte `byte`, which is on a grapheme boundary of `snapshot`,
/// found by the units of `units` (an index of it).
pub(crate) fn grapheme_of_byte_in(snapshot: &TextSnapshot, units: &LayoutIndex, byte: usize) -> usize {
    let byte = byte.min(snapshot.len());
    let unit = units.unit_of_byte(byte);
    let start = units.unit_bytes(unit).start;
    let before = snapshot.slice(start..byte);
    let count = if is_one_byte_graphemes(&before) { before.len() } else { before.graphemes(true).count() };
    units.graphemes_before(unit) + count
}

/// The text a selection area is given of a field: the text itself, shared, for a short one, and
/// nothing for a long one, which lays out a window of itself and takes no part in selection across
/// widgets, so the area never reads it and a copy of megabytes for each rebuild is not made.
pub(crate) fn selection_source(controller: &super::TextEditingController) -> std::rc::Rc<str> {
    let value = controller.value();
    if value.len() >= large_text_bytes() { std::rc::Rc::from("") } else { std::rc::Rc::from(&*value.text()) }
}
