//! Counting the graphemes of text in which every character is a grapheme of its own.
//!
//! Counting graphemes with the Unicode rules costs a state machine and a table search for each
//! character, and most text needs neither: Latin, Greek, Cyrillic and CJK ideographs are made of
//! characters that never join the one before or after them. A character whose grapheme break
//! property is "other" or "control" always starts a grapheme and ends one, so a text made only of
//! them has as many graphemes as characters, except that a carriage return before a line feed makes
//! one grapheme of the two.
//!
//! [`simple_grapheme_count`] answers for such text and says so when it is given anything else, which
//! is then counted by the full rules. The ranges below are a subset of what is safe, picked to
//! cover the scripts text is usually written in; a text that uses none of the rest is counted at
//! scan speed.

/// Code points that are always a grapheme of their own, as inclusive ranges in ascending order.
/// None holds a combining mark, a joiner, a prepended character, a Hangul jamo or syllable, or a
/// regional indicator, which is what makes a character join its neighbours. The `aimer_text_buffer`
/// tests check every code point of every range against the Unicode rules.
const SIMPLE: [(u32, u32); 21] = [
    (0x0000, 0x02FF),
    (0x0370, 0x03FF),
    (0x0400, 0x0482),
    (0x048A, 0x052F),
    (0x1E00, 0x1EFF),
    (0x2010, 0x2027),
    (0x2030, 0x205E),
    (0x20A0, 0x20BF),
    (0x2190, 0x23FF),
    (0x2500, 0x26FF),
    (0x3000, 0x3029),
    (0x3030, 0x303F),
    (0x3041, 0x3096),
    (0x30A1, 0x30FF),
    (0x3105, 0x312F),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xF900, 0xFAFF),
    (0xFE30, 0xFE4F),
    (0xFF01, 0xFF60),
    (0xFFE0, 0xFFE6),
];

/// Whether the code point `cp` is one of the ranges that never join a neighbour.
#[inline(always)]
fn is_simple(cp: u32) -> bool {
    // The common cases first: Latin and the unified ideographs.
    if cp < 0x300 || (0x4E00..=0x9FFF).contains(&cp) {
        return true;
    }
    let mut low = 0;
    let mut high = SIMPLE.len();
    while low < high {
        let middle = (low + high) / 2;
        let (start, end) = SIMPLE[middle];
        if cp < start {
            high = middle;
        } else if cp > end {
            low = middle + 1;
        } else {
            return true;
        }
    }
    false
}

/// How many graphemes `text` has, if every character in it is a grapheme of its own (see the
/// module): `None` when it holds any character that may join another, which the caller counts by
/// the full rules.
///
/// `text` must be valid UTF-8; the result for bytes that are not is unspecified, but the call does not
/// read out of bounds.
///
/// # Examples
///
/// ```
/// use aimer_simd::kernels::simple_grapheme_count;
///
/// assert_eq!(simple_grapheme_count("caf\u{e9} \u{4f60}\u{597d}\r\n".as_bytes()), Some(8));
/// // A combining accent joins the letter before it: the full rules are needed.
/// assert_eq!(simple_grapheme_count("e\u{301}".as_bytes()), None);
/// ```
// An `#[inline]` function is compiled in the crate that calls it, at that crate's optimization
// level; this one is a function of this crate's own, compiled once, optimized.
#[cfg_attr(debug_assertions, inline(never))]
#[cfg_attr(not(debug_assertions), inline)]
pub fn simple_grapheme_count(text: &[u8]) -> Option<usize> {
    let length = text.len();
    let mut at = 0;
    let mut count = 0;
    while at < length {
        // Eight ASCII bytes with no carriage return are eight graphemes: most text is mostly that.
        if let Some(word) = text.get(at..at + 8) {
            let word = u64::from_le_bytes(word.try_into().expect("a slice of eight bytes"));
            const HIGH: u64 = 0x8080_8080_8080_8080;
            const LOW: u64 = 0x0101_0101_0101_0101;
            let carriage = word ^ (LOW * u64::from(b'\r'));
            if word & HIGH == 0 && carriage.wrapping_sub(LOW) & !carriage & HIGH == 0 {
                at += 8;
                count += 8;
                continue;
            }
        }
        let byte = text[at];
        if byte < 0x80 {
            // A carriage return and the line feed after it are one grapheme.
            at += if byte == b'\r' && text.get(at + 1) == Some(&b'\n') { 2 } else { 1 };
        } else if byte < 0xE0 {
            let second = *text.get(at + 1)?;
            if !is_simple((u32::from(byte & 0x1F) << 6) | u32::from(second & 0x3F)) {
                return None;
            }
            at += 2;
        } else if byte < 0xF0 {
            let (second, third) = (*text.get(at + 1)?, *text.get(at + 2)?);
            let cp = (u32::from(byte & 0x0F) << 12) | (u32::from(second & 0x3F) << 6) | u32::from(third & 0x3F);
            if !is_simple(cp) {
                return None;
            }
            at += 3;
        } else {
            // Four bytes: emoji and the rest of the supplementary planes, which join and modify.
            return None;
        }
        count += 1;
    }
    Some(count)
}
