//! Text typed the way a person types it.
//!
//! A `text` command is one edit: the whole string lands at once, as an input
//! method's commit does, and a viewer watching the app sees it appear. Typing
//! sends it one keystroke at a time instead, with an uneven rhythm — a little
//! faster or slower from one key to the next, a breath between words, a longer
//! one at the end of a sentence — so a recording of a driven app looks driven by
//! a hand. This plans those keystrokes; the server walks the plan.
//!
//! Nothing here is random. The unevenness comes from a fixed hash of the
//! position, so the same text always types the same way, which keeps tests and
//! demos repeatable.

use std::time::Duration;

use crate::command::MAX_GLIDE_MILLIS as MAX_MILLIS;

/// About how long one keystroke takes before the next: a fast, comfortable typist.
const KEY_MILLIS: f32 = 70.0;
/// How far a keystroke's time strays from [`KEY_MILLIS`], as a fraction either way.
const JITTER: f32 = 0.35;
/// What a pause adds after a character, in milliseconds.
const AFTER_SPACE: f32 = 50.0;
const AFTER_COMMA: f32 = 90.0;
const AFTER_SENTENCE: f32 = 160.0;
const AFTER_NEWLINE: f32 = 220.0;

/// What to type and how long to wait after each piece.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    /// A keystroke's text, and how long to wait after it before the next. The last
    /// wait is zero: there is nothing left to wait for.
    pub(crate) steps: Vec<(String, Duration)>,
}

/// Plans typing `text` in about `millis`, or as long as its length deserves.
///
/// Without a time the keystrokes take their natural time, but never more than a
/// client waits for a reply: a long text is typed faster, every character still
/// sent. A time outside `1..=3000` is brought inside it.
pub(crate) fn plan(text: &str, millis: Option<u32>) -> Plan {
    let pieces = clusters(text);
    let mut waits: Vec<f32> = pieces.iter().enumerate().map(|(index, piece)| natural_wait(index, piece)).collect();
    // Nothing follows the last keystroke.
    if let Some(last) = waits.last_mut() {
        *last = 0.0;
    }
    let natural: f32 = waits.iter().sum();
    let wanted = match millis {
        Some(millis) => millis.clamp(1, MAX_MILLIS) as f32,
        None => natural.min(MAX_MILLIS as f32),
    };
    // A single character has no rhythm to scale.
    let scale = if natural > 0.0 { wanted / natural } else { 1.0 };
    let steps = pieces
        .into_iter()
        .zip(waits)
        .map(|(piece, wait)| (piece, Duration::from_secs_f32(wait * scale / 1000.0)))
        .collect();
    Plan { steps }
}

/// How long a person waits after `piece`, the `index`th keystroke, in milliseconds.
fn natural_wait(index: usize, piece: &str) -> f32 {
    let uneven = 1.0 + JITTER * (2.0 * unit(index as u64) - 1.0);
    let pause = match piece.chars().next() {
        Some(' ') => AFTER_SPACE,
        Some(',' | ';' | ':') => AFTER_COMMA,
        Some('.' | '!' | '?') => AFTER_SENTENCE,
        Some('\n') => AFTER_NEWLINE,
        _ => 0.0,
    };
    KEY_MILLIS * uneven + pause
}

/// A fixed number in `[0, 1)` for `n`: the output of one round of SplitMix64.
fn unit(n: u64) -> f32 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// Splits `text` into what a person types in one keystroke: a character with the
/// accents and variation selectors that ride on it, a joined emoji sequence, a
/// flag. Not full grapheme segmentation, which would take a table of Unicode
/// data this crate does not carry, but it keeps a keystroke from being torn in
/// the middle of one character.
fn clusters(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(first) = chars.next() {
        let mut piece = String::from(first);
        let mut joined = false;
        let mut flag_open = is_regional_indicator(first);
        loop {
            if joined {
                // The character a joiner reaches for belongs to the same piece.
                let Some(next) = chars.next() else { break };
                piece.push(next);
                joined = next == '\u{200D}';
                continue;
            }
            match chars.peek().copied() {
                Some(next) if is_extender(next) => {
                    piece.push(next);
                    chars.next();
                    joined = next == '\u{200D}';
                }
                Some(next) if flag_open && is_regional_indicator(next) => {
                    piece.push(next);
                    chars.next();
                    flag_open = false;
                }
                _ => break,
            }
        }
        out.push(piece);
    }
    out
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// Combining marks, variation selectors, skin tones and the zero-width joiner.
fn is_extender(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FE20}'..='\u{FE2F}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{200D}'
    )
}

#[cfg(test)]
#[path = "typing_tests.rs"]
mod tests;
