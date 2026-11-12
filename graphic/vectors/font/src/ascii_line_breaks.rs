//! Line break opportunities of plain ASCII text, without the general algorithm.
//!
//! Measuring a paragraph needs its UAX #14 break opportunities, and the
//! general implementation looks every character up in a Unicode table and runs
//! a state machine over the result. Most paragraphs are, for this purpose,
//! nothing but words, numbers, spaces and a little punctuation, and for those
//! the answer depends only on the class of a character and the class of the one
//! before it, so a pair table settles each position with one array read.
//!
//! The table is only valid for the classes listed in [`Class`], none of which
//! introduces a rule that looks further back than the previous character: that
//! rules out opening and closing brackets, quotes, prefix and postfix symbols
//! and everything that is not ASCII. Text containing any other byte is left to
//! the general algorithm, so adding a class here means proving it against
//! `unicode_linebreak` in the tests below, not trusting the rules by reading
//! them.

/// The line breaking classes this module can decide on its own.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Class {
    /// Not handled here; the text goes to the general algorithm.
    Other = 0,
    /// Letters (`AL`).
    Alphabetic,
    /// Digits (`NU`).
    Numeric,
    /// The space (`SP`).
    Space,
    /// The hyphen-minus (`HY`).
    Hyphen,
    /// `.` `,` `:` `;` (`IS`).
    InfixSeparator,
    /// `!` `?` (`EX`).
    Exclamation,
    /// `/` (`SY`).
    Solidus,
}

const CLASS_COUNT: usize = 8;

/// The class of every byte, [`Class::Other`] for everything that is not one of
/// the characters above, so a byte indexes the table with no range check of
/// its own and no `Option` to unwrap.
const CLASS_OF: [u8; 256] = {
    let mut table = [Class::Other as u8; 256];
    let mut byte = 0;
    while byte < 128 {
        table[byte] = (match byte as u8 {
            b'a'..=b'z' | b'A'..=b'Z' => Class::Alphabetic,
            b'0'..=b'9' => Class::Numeric,
            b' ' => Class::Space,
            b'-' => Class::Hyphen,
            b'.' | b',' | b':' | b';' => Class::InfixSeparator,
            b'!' | b'?' => Class::Exclamation,
            b'/' => Class::Solidus,
            _ => Class::Other,
        }) as u8;
        byte += 1;
    }
    table
};

/// Whether a line may start between a character of class `before` and one of
/// class `after`, in the classes above.
///
/// Each arm names the rule that decides it; the rules are applied in UAX #14's
/// order, so a rule listed first wins over one listed later.
const fn may_break(before: Class, after: Class) -> bool {
    // LB7 (no break before a space) and LB13 (none before `IS`, `EX`, `SY`)
    // come before everything that could allow a break.
    if matches!(
        after,
        Class::Space | Class::InfixSeparator | Class::Exclamation | Class::Solidus
    ) {
        return false;
    }
    match (before, after) {
        // LB18: break after spaces, whatever follows them (LB21's "no break
        // before a hyphen" comes later in the order and does not apply).
        (Class::Space, Class::Alphabetic | Class::Numeric | Class::Hyphen) => true,
        // After a hyphen a word may start a line, but LB25 keeps a number with
        // its sign and LB21 keeps a second hyphen with the first.
        (Class::Hyphen, Class::Alphabetic) => true,
        // LB31: nothing binds `EX` to what follows it, except LB21 for a hyphen.
        (Class::Exclamation, Class::Alphabetic | Class::Numeric) => true,
        // As above, but LB25 keeps `SY` with a number after it.
        (Class::Solidus, Class::Alphabetic) => true,
        // `AL`/`NU` bind to each other and to a following hyphen (LB23, LB21);
        // nothing breaks after `IS` (LB29, LB25, LB21).
        _ => false,
    }
}

/// `may_break` for every pair, so deciding a position is one read.
const BREAK_BETWEEN: [[bool; CLASS_COUNT]; CLASS_COUNT] = {
    const ALL: [Class; CLASS_COUNT] = [
        Class::Other,
        Class::Alphabetic,
        Class::Numeric,
        Class::Space,
        Class::Hyphen,
        Class::InfixSeparator,
        Class::Exclamation,
        Class::Solidus,
    ];
    let mut table = [[false; CLASS_COUNT]; CLASS_COUNT];
    let mut before = 0;
    while before < CLASS_COUNT {
        let mut after = 0;
        while after < CLASS_COUNT {
            table[before][after] = may_break(ALL[before], ALL[after]);
            after += 1;
        }
        before += 1;
    }
    table
};

/// Marks the break opportunities of `text` in `allowed` and reports `true`, or
/// reports `false` when `text` is not entirely made of the classes above.
///
/// `allowed` must be `text.len() + 1` entries, whatever they held: a handled
/// text writes every one of them, so a buffer can be reused without clearing it.
/// On `false` it may have been partly written and means nothing. Entry `i`
/// means a line may start at byte `i`, exactly as for
/// [`crate::text_layout::line_break_opportunities`].
pub(crate) fn fill(text: &str, allowed: &mut [bool]) -> bool {
    debug_assert_eq!(allowed.len(), text.len() + 1);
    const OTHER: usize = Class::Other as usize;
    let bytes = text.as_bytes();
    let length = bytes.len();
    // The first character never starts a line (LB2), so it only sets `before`.
    allowed[0] = false;
    if length == 0 {
        return true;
    }
    let mut before = usize::from(CLASS_OF[usize::from(bytes[0])]);
    if before == OTHER {
        return false;
    }
    let mut index = 1;
    while index < length {
        let class = usize::from(CLASS_OF[usize::from(bytes[index])]);
        if class == OTHER {
            return false;
        }
        allowed[index] = BREAK_BETWEEN[before][class];
        before = class;
        index += 1;
    }
    // LB3: always break at the end of the text.
    allowed[length] = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_linebreak::linebreaks;

    /// What the general algorithm says, in the shape `fill` produces.
    fn reference(text: &str) -> Vec<bool> {
        let mut allowed = vec![false; text.len() + 1];
        for (offset, _) in linebreaks(text) {
            allowed[offset] = true;
        }
        allowed[0] = false;
        allowed
    }

    fn fast(text: &str) -> Option<Vec<bool>> {
        let mut allowed = vec![false; text.len() + 1];
        fill(text, &mut allowed).then_some(allowed)
    }

    fn supported(byte: u8) -> bool {
        CLASS_OF[usize::from(byte)] != Class::Other as u8
    }

    #[track_caller]
    fn assert_same(text: &str) {
        let expected = reference(text);
        match fast(text) {
            Some(got) => assert_eq!(got, expected, "{text:?}"),
            None => panic!("{text:?} is made of handled classes but was declined"),
        }
    }

    #[test]
    fn every_sequence_of_classes_up_to_six_long_matches_the_algorithm() {
        // One character per class: the rules are about classes, not letters.
        const REPRESENTATIVES: [char; 7] = ['a', '1', ' ', '-', '.', '!', '/'];
        let mut text = String::new();
        let mut counters = Vec::new();
        let mut checked = 0_usize;
        for length in 0..=6_usize {
            counters.clear();
            counters.resize(length, 0_usize);
            loop {
                text.clear();
                text.extend(counters.iter().map(|index| REPRESENTATIVES[*index]));
                assert_same(&text);
                checked += 1;
                // Odometer step over all sequences of this length.
                let mut position = 0;
                loop {
                    if position == length {
                        break;
                    }
                    counters[position] += 1;
                    if counters[position] < REPRESENTATIVES.len() {
                        break;
                    }
                    counters[position] = 0;
                    position += 1;
                }
                if position == length {
                    break;
                }
            }
        }
        // 7^0 + 7^1 + ... + 7^6: every sequence was visited exactly once.
        assert_eq!(checked, 137_257);
    }

    #[test]
    fn every_pair_and_triple_of_bytes_matches_or_is_declined() {
        let bytes: Vec<u8> = (0..128).collect();
        for first in &bytes {
            for second in &bytes {
                let text = String::from_utf8(vec![*first, *second]).expect("ASCII");
                check(&text, supported(*first) && supported(*second));
            }
        }
        let handled: Vec<u8> = bytes.iter().copied().filter(|byte| supported(*byte)).collect();
        for first in &handled {
            for second in &handled {
                for third in &handled {
                    let text = String::from_utf8(vec![*first, *second, *third]).expect("ASCII");
                    check(&text, true);
                }
            }
        }
    }

    #[track_caller]
    fn check(text: &str, handled: bool) {
        if handled {
            assert_same(text);
        } else {
            assert!(fast(text).is_none(), "{text:?} must be left to the algorithm");
        }
    }

    #[test]
    fn random_text_matches_the_algorithm() {
        // A fixed xorshift stream keeps the test deterministic.
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let handled: Vec<u8> = (0..128).filter(|byte| supported(*byte)).collect();
        for _ in 0..60_000 {
            let length = (next() % 48) as usize;
            let bytes: Vec<u8> = (0..length)
                .map(|_| handled[(next() % handled.len() as u64) as usize])
                .collect();
            assert_same(std::str::from_utf8(&bytes).expect("ASCII"));
        }
    }

    #[test]
    fn text_with_other_classes_is_left_to_the_algorithm() {
        for text in [
            "caf\u{e9}",
            "a\nb",
            "tab\there",
            "日本語",
            "it's",
            "(a)",
            "a\u{a0}b",
            "100%",
            "\u{0}",
            "a\u{7f}",
        ] {
            assert!(fast(text).is_none(), "{text:?} must be left to the algorithm");
        }
    }

    #[test]
    fn line_break_opportunities_agree_with_the_algorithm_for_any_text() {
        // Handled, declined, and declined only after a handled prefix: the
        // partly written table of the last kind must not leak into the answer.
        for text in [
            "",
            "plain words and 123, numbers.",
            "plain words then (brackets) and 'quotes'",
            "a b c d e f g h i j k\nl m",
            "日本語のテキスト and some English, 100% sure",
            "tab\tseparated words here",
            "caf\u{e9} au lait - très bon!",
            "aaaa bbbb cccc dddd \u{2014} eeee",
            "https://example.com/a/b?c=d&e=f#g",
        ] {
            assert_eq!(
                crate::text_layout::line_break_opportunities(text),
                reference(text),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_handled_text_overwrites_whatever_the_buffer_held() {
        // A caller reusing a buffer must not have to clear it first.
        for text in [
            "",
            "a",
            " ",
            "plain words and 123, numbers.",
            "  leading and trailing  ",
            "well-known a-b 5-6 -x! ok? a/b",
        ] {
            let expected = reference(text);
            for stale in [true, false] {
                let mut allowed = vec![stale; text.len() + 1];
                assert!(fill(text, &mut allowed), "{text:?} is handled");
                assert_eq!(allowed, expected, "{text:?} over a buffer of {stale}");
            }
        }
    }

    #[test]
    fn a_kept_buffer_gives_the_same_answer_as_a_fresh_one() {
        // Longer, shorter and stale buffers, for handled and declined text, and
        // for a declined text that is only declined after a handled prefix.
        let texts = [
            "",
            "plain words and 123, numbers.",
            "plain words then (brackets) and 'quotes'",
            "日本語のテキスト and English",
            "a\nb",
            "x",
        ];
        let mut kept = vec![true; 7];
        for text in texts.iter().chain(texts.iter().rev()) {
            crate::text_layout::line_break_opportunities_into(text, &mut kept);
            assert_eq!(kept, reference(text), "{text:?}");
            assert_eq!(kept.len(), text.len() + 1);
        }
    }

    #[test]
    fn the_empty_text_has_no_opportunity() {
        assert_eq!(fast("").expect("empty text is handled"), vec![false]);
    }

    #[test]
    fn running_prose_is_handled() {
        // The shape of real paragraphs: words, numbers, spaces and the common
        // punctuation. If this is declined the fast path is not earning its keep.
        assert_same(
            "Detail paragraph 12: The compositor keeps this decorated scrolling row on the \
             live culled path because its container owns clipping and hit-test bounds. Text \
             shaping is cached per width, while stable paint islands elsewhere can be retained \
             automatically or promoted explicitly with RepaintBoundary. See a/b, 3.14; ok? yes!",
        );
    }
}
