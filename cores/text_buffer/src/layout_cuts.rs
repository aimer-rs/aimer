//! Where the units of a text end.
//!
//! A hard line is one unit, up to [`LINE_LIMIT`] bytes. A longer line is cut into segments of
//! between [`SEGMENT_MIN`] and [`SEGMENT_MAX`] bytes at places the *content* picks: a place
//! where a word may break and a few bytes before it hash a certain way. A cut then depends on the
//! text just before it, and on how far it is from the cut before, and on nothing else. Edit a
//! line of fifty megabytes in the middle and the cuts far from the edit are where they were, so
//! only the segments around it are made again; cutting every N bytes would move every cut after
//! the edit.

use std::borrow::Cow;

use aimer_simd::kernels::{find_byte, simple_grapheme_count};
use unicode_segmentation::UnicodeSegmentation;

use crate::rope::TextSnapshot;
use crate::{LINE_LIMIT, SEGMENT_MAX, SEGMENT_MIN};

/// How many bytes before a cut decide whether there is one. A segment's cuts read only text
/// after its own start (`SEGMENT_MIN` is longer than this), so how a unit ends depends on the
/// text from its start on and on nothing before it.
pub(crate) const CUT_WINDOW: usize = 8;

/// The end of the unit that starts at `start`: after its line feed, or at the cut that ends a
/// segment of a long line, or at the end of the text. `start` is a unit start, which is a
/// grapheme boundary.
pub(crate) fn unit_end(text: &TextSnapshot, start: usize) -> usize {
    let len = text.len();
    if start >= len {
        return len;
    }
    // The first line feed within the most a whole line may be.
    let window_end = (start + LINE_LIMIT + 1).min(len);
    let mut at = start;
    for piece in text.byte_chunks(start..window_end) {
        if let Some(found) = find_byte(piece, b'\n') {
            return at + found + 1;
        }
        at += piece.len();
    }
    if window_end == len {
        return len;
    }
    segment_end(text, start)
}

/// Whether a segment may end at the byte `at` of `bytes` (a window of the text starting at
/// `base`), by what is just before it: a place a line may break, picked by a hash of the text.
fn is_cut(bytes: &[u8], at: usize) -> bool {
    if (bytes[at] as i8) < -0x40 {
        return false; // inside a character
    }
    let before = bytes[at - 1];
    let breaks = before == b' ' || before == b'\t' || before >= 0x80 || bytes[at] >= 0x80;
    if !breaks {
        return false;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in &bytes[at - CUT_WINDOW..at] {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash >> 32) & 3 == 0
}

/// The end of the segment of a long line that starts at `start`: the first cut at least
/// [`SEGMENT_MIN`] on, or the grapheme boundary at [`SEGMENT_MAX`] when the text has none before.
fn segment_end(text: &TextSnapshot, start: usize) -> usize {
    let from = start + SEGMENT_MIN;
    let to = start + SEGMENT_MAX;
    // The bytes the decisions read: the window before the first candidate, through the last.
    let bytes: Cow<[u8]> = text.bytes(from - CUT_WINDOW..(to + 1).min(text.len()));
    for at in from..=to.min(text.len().saturating_sub(1)) {
        let relative = at - (from - CUT_WINDOW);
        if relative < bytes.len() && is_cut(&bytes, relative) && text.is_grapheme_boundary(at) {
            return at;
        }
    }
    text.floor_grapheme_boundary(to.min(text.len()))
}

/// How many graphemes the text in `range` has. `range` is a unit, so it starts and ends on
/// grapheme boundaries and is counted by itself, however many chunks it runs over.
pub(crate) fn grapheme_count(text: &TextSnapshot, range: std::ops::Range<usize>) -> u32 {
    grapheme_count_of(&text.slice(range))
}

/// How many graphemes `unit` has, when it is a unit of its own already read out of the text.
pub(crate) fn grapheme_count_of(unit: &str) -> u32 {
    if unit.is_ascii() && !unit.contains('\r') {
        return unit.len() as u32;
    }
    // Text whose characters all stand alone (Latin, Cyrillic, CJK ideographs, ...) is counted without
    // the rules; anything that may join is counted by them.
    match simple_grapheme_count(unit.as_bytes()) {
        Some(count) => count as u32,
        None => unit.graphemes(true).count() as u32,
    }
}
