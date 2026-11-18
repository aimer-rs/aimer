//! The layout index against the text it stands for: built, built a part at a time, and made
//! again from edits, in a text whose lines are long enough to be cut into segments (`LINE_LIMIT`
//! is 64 bytes under test).

use aimer_sum_tree::Bias;
use unicode_segmentation::UnicodeSegmentation;

use crate::{LINE_LIMIT, LayoutIndex, LayoutMetrics, SEGMENT_MAX, SEGMENT_MIN, TextBuffer, TextSnapshot};

struct Random(u64);

impl Random {
    fn below(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % below as u64) as usize
    }
}

const METRICS: LayoutMetrics = LayoutMetrics { content_width: 100.0, line_height: 20.0, advance: 8.0 };

/// Pieces of every kind, and spaces so that long lines have places to be cut.
const PIECES: [&str; 22] = [
    "a", "bb", " ", " ", " ", "\n", "\n", "word ", "e", "\u{301}", "\u{e9}", "\u{200d}", "\u{1f468}", "\u{1f469}", "\u{1f1e6}", "\u{1f1e7}",
    "\u{4f60}", "\u{597d}", "\r\n", "-", "1", "\t",
];

fn random_text(random: &mut Random, pieces: usize) -> String {
    (0..pieces).map(|_| PIECES[random.below(PIECES.len())]).collect()
}

/// A text with some lines that are long, and so are cut, and some that are not.
fn text_with_long_lines(random: &mut Random) -> String {
    let mut text = String::new();
    for _ in 0..random.below(8) {
        let long = random.below(3) == 0;
        let pieces = if long { 60 + random.below(400) } else { random.below(12) };
        text.push_str(&random_text(random, pieces).replace('\n', if long { " " } else { "\n" }));
        if random.below(4) != 0 {
            text.push('\n');
        }
    }
    text
}

/// What an index of `text` has to be, worked out from the string: the units cover the text, a hard
/// line is one unit unless it is long, a segment is neither shorter than `SEGMENT_MIN` nor longer
/// than `SEGMENT_MAX` (but for the last of its line), and the grapheme counts are right.
#[track_caller]
fn assert_sound(index: &LayoutIndex, text: &str, what: &str) {
    index.check_invariants();
    let shape = index.shape();
    assert_eq!(shape.iter().map(|unit| unit.0).sum::<usize>(), text.len(), "{what}: the units cover the text");
    assert_eq!(index.total_graphemes(), text.graphemes(true).count(), "{what}: graphemes");
    assert_eq!(index.hard_line_count(), text.matches('\n').count() + 1, "{what}: hard lines");
    let mut start = 0;
    let mut line_start = 0;
    for (number, &(bytes, graphemes, ends_line)) in shape.iter().enumerate() {
        let unit = &text[start..start + bytes];
        assert_eq!(graphemes as usize, unit.graphemes(true).count(), "{what}: unit {number} {unit:?}");
        assert_eq!(ends_line, unit.ends_with('\n') || start + bytes == text.len(), "{what}: unit {number} ends_line");
        assert!(!unit.strip_suffix('\n').unwrap_or(unit).contains('\n'), "{what}: unit {number} holds a line feed in its middle");
        let line_len = text[line_start..].find('\n').map_or(text.len() - line_start, |at| at + 1);
        if ends_line {
            let from_line_start = start + bytes - line_start;
            if from_line_start <= LINE_LIMIT + 1 {
                assert_eq!(start, line_start, "{what}: a short line is one unit");
            } else {
                assert!(bytes <= SEGMENT_MAX.max(LINE_LIMIT + 1), "{what}: the last segment of a line, {bytes} bytes");
            }
            line_start = start + bytes;
        } else {
            assert!((SEGMENT_MIN..=SEGMENT_MAX).contains(&bytes), "{what}: a segment of {bytes} bytes");
            assert!(line_len > LINE_LIMIT, "{what}: only a long line has segments");
        }
        start += bytes;
    }
    assert_eq!(start, text.len());
}

#[test]
fn the_index_of_a_text_covers_it_in_units_of_the_right_kind() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for round in 0..120 {
        let text = text_with_long_lines(&mut random);
        let index = LayoutIndex::build(&TextSnapshot::from_text(&text), METRICS);
        assert_sound(&index, &text, &format!("round {round}"));
        assert!(index.is_complete(&TextSnapshot::from_text(&text)));
    }
    for text in ["", "\n", "a", "a\n", "\n\n", "a\nb"] {
        let index = LayoutIndex::build(&TextSnapshot::from_text(text), METRICS);
        assert_sound(&index, text, &format!("{text:?}"));
    }
    // "a\nb\n" is three lines, the last empty, as a text area shows it.
    assert_eq!(LayoutIndex::build(&TextSnapshot::from_text("a\nb\n"), METRICS).len(), 3);
    assert_eq!(LayoutIndex::build(&TextSnapshot::from_text(""), METRICS).len(), 1);
}

#[test]
fn indexing_a_part_at_a_time_gives_the_index_of_the_whole() {
    let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
    for round in 0..60 {
        let text = text_with_long_lines(&mut random);
        let snapshot = TextSnapshot::from_text(&text);
        let whole = LayoutIndex::build(&snapshot, METRICS);
        let mut parts = LayoutIndex::new(METRICS);
        let budget = 1 + random.below(120);
        let mut rounds = 0;
        while !parts.index_more(&snapshot, budget) {
            rounds += 1;
            assert!(rounds < 10_000, "indexing makes no progress");
        }
        assert_eq!(parts.shape(), whole.shape(), "round {round}");
    }
}

#[test]
fn a_text_that_is_still_arriving_is_indexed_as_it_grows() {
    let mut random = Random(0x1234_5678_9ABC_DEF1);
    for round in 0..40 {
        let text = text_with_long_lines(&mut random);
        let mut index = LayoutIndex::new(METRICS);
        let mut shown = 0;
        while shown < text.len() {
            shown = (shown + 1 + random.below(60)).min(text.len());
            while !text.is_char_boundary(shown) {
                shown += 1;
            }
            // Loaded as far as `shown`; what is there is indexed, as the text is a prefix of the file.
            let snapshot = TextSnapshot::from_text(&text[..shown]);
            index.index_more(&snapshot, usize::MAX);
            assert_sound(&index, &text[..shown], &format!("round {round} at {shown}"));
        }
    }
}

#[test]
fn edits_make_the_index_the_one_of_the_edited_text_and_touch_few_units() {
    let mut random = Random(0xA5A5_5A5A_1234_4321);
    for round in 0..80 {
        let mut text = text_with_long_lines(&mut random);
        let mut buffer = TextBuffer::from_text(&text);
        let mut index = LayoutIndex::build(buffer.text(), METRICS);
        for edit in 0..16 {
            let boundaries: Vec<usize> = (0..=text.len()).filter(|&at| text.is_char_boundary(at)).collect();
            let a = boundaries[random.below(boundaries.len())];
            let b = if random.below(3) == 0 { boundaries[random.below(boundaries.len())] } else { boundaries.iter().copied().find(|&at| at >= a + random.below(5)).unwrap_or(text.len()) };
            let range = a.min(b)..a.max(b);
            let pieces = match random.below(4) {
                0 => 0,
                1 => random.below(90),
                _ => random.below(4),
            };
            let inserted = random_text(&mut random, pieces);
            let delta = buffer.replace(range.clone(), &inserted);
            text.replace_range(range.clone(), &inserted);
            index.apply_edit(&delta, buffer.text());

            let what = format!("round {round} edit {edit}: {range:?} -> {inserted:?}");
            assert_sound(&index, &text, &what);
            assert_eq!(index.shape(), LayoutIndex::build(buffer.text(), METRICS).shape(), "{what}: the same as the index made whole");
        }
    }
}

#[test]
fn an_edit_in_a_very_long_line_makes_again_only_the_segments_around_it() {
    // One line of many segments, edited in the middle again and again.
    let sentence = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau ";
    let text = sentence.repeat(300);
    let mut buffer = TextBuffer::from_text(&text);
    let mut index = LayoutIndex::build(buffer.text(), METRICS);
    let units = index.len();
    assert!(units > 100, "{units} units: the line is cut");
    let mut worst = 0;
    for step in 0..60 {
        let at = buffer.text().floor_grapheme_boundary(text.len() / 2 + step * 7);
        let delta = buffer.replace(at..at, if step % 3 == 0 { "XY" } else { "q" });
        let reindexed = index.apply_edit(&delta, buffer.text());
        worst = worst.max(reindexed.added.max(reindexed.removed));
        assert_eq!(index.shape(), LayoutIndex::build(buffer.text(), METRICS).shape(), "step {step}");
    }
    // The segments within the reach of `LINE_LIMIT` before the edit, and those until the cuts agree
    // again after it: a handful, and not a number that grows with the line.
    assert!(worst <= 10, "an edit remade {worst} of {units} units");
}

#[test]
fn an_edit_of_ordinary_lines_makes_again_the_lines_it_touches() {
    let text = "a line of text\n".repeat(500);
    let mut buffer = TextBuffer::from_text(&text);
    let mut index = LayoutIndex::build(buffer.text(), METRICS);
    let delta = buffer.replace(20..20, "x");
    assert_eq!(index.apply_edit(&delta, buffer.text()), crate::Reindexed { removed: 1, added: 1 });
    let delta = buffer.replace(15..15, "new\nlines\n");
    assert_eq!(index.apply_edit(&delta, buffer.text()), crate::Reindexed { removed: 1, added: 3 }, "one line became three");
    let delta = buffer.replace(0..buffer.text().len() / 2, "");
    let reindexed = index.apply_edit(&delta, buffer.text());
    assert!(reindexed.removed > 100 && reindexed.added <= 2, "{reindexed:?}");
    assert_eq!(index.shape(), LayoutIndex::build(buffer.text(), METRICS).shape());
}

#[test]
fn laid_out_heights_survive_edits_elsewhere_and_estimates_follow_the_metrics() {
    let text = "short\n".repeat(50) + &"w ".repeat(200) + "\nend";
    let mut buffer = TextBuffer::from_text(&text);
    let mut index = LayoutIndex::build(buffer.text(), METRICS);
    // Before anything is laid out every height is an estimate: one visual line for a short unit.
    assert_eq!(index.height(0), 20.0);
    assert!(!index.is_exact(0));
    index.refine(3, 47.0);
    assert!(index.is_exact(3) && index.height(3) == 47.0);
    let version = index.version();
    index.refine(3, 47.0);
    assert_eq!(index.version(), version, "the same height is no change");
    index.refine(3, 41.0);
    assert!(index.version() > version);

    let delta = buffer.replace(buffer.text().len() - 3..buffer.text().len(), "ending");
    index.apply_edit(&delta, buffer.text());
    assert!(index.is_exact(3) && index.height(3) == 41.0, "a unit the edit did not touch keeps what was laid out");

    let total: f64 = (0..index.len()).map(|unit| f64::from(index.height(unit))).sum();
    assert!((index.total_height() - total).abs() < 1e-6);
    for unit in [0, 1, 7, index.len() - 1] {
        let top: f64 = (0..unit).map(|unit| f64::from(index.height(unit))).sum();
        assert!((index.top(unit) - top).abs() < 1e-6, "top of unit {unit}");
        assert_eq!(index.unit_at_y(top + f64::from(index.height(unit)) * 0.5), unit, "the unit at the middle of unit {unit}");
    }
    assert_eq!(index.unit_at_y(-5.0), 0);
    assert_eq!(index.unit_at_y(1e12), index.len() - 1);
    let range = index.units_between(index.top(5) + 1.0, index.top(8) + 1.0);
    assert_eq!(range, 5..9);

    // Another wrapping discards what was laid out, which was for the other; the same keeps it.
    let before = index.height(3);
    index.set_metrics(LayoutMetrics { advance: 7.0, ..METRICS });
    assert_eq!(index.height(3), before);
    index.set_metrics(LayoutMetrics { content_width: 50.0, ..METRICS });
    assert!(!index.is_exact(3));
    let wide = LayoutIndex::build(buffer.text(), METRICS).total_height();
    assert!(index.total_height() > wide, "a narrower column is taller");
}

#[test]
fn bytes_graphemes_and_hard_lines_are_found_from_each_other() {
    let mut random = Random(0xFEED_FACE_0BAD_F00D);
    for round in 0..50 {
        let text = text_with_long_lines(&mut random);
        let snapshot = TextSnapshot::from_text(&text);
        let index = LayoutIndex::build(&snapshot, METRICS);
        let mut start = 0;
        let mut graphemes = 0;
        let mut lines = 0;
        for unit in 0..index.len() {
            let range = index.unit_bytes(unit);
            assert_eq!(range.start, start, "round {round}: unit {unit}");
            assert_eq!(index.unit_of_byte(range.start), unit, "round {round}: the unit at its first byte");
            if !range.is_empty() {
                assert_eq!(index.unit_of_byte(range.end - 1), unit, "round {round}: the unit at its last byte");
            }
            assert_eq!(index.graphemes_before(unit), graphemes);
            assert_eq!(index.hard_lines_before(unit), lines);
            if index.unit_graphemes(unit) > 0 {
                assert_eq!(index.unit_of_grapheme(graphemes), (unit, 0), "round {round}: the first grapheme of unit {unit}");
                let middle = graphemes + index.unit_graphemes(unit) - 1;
                assert_eq!(index.unit_of_grapheme(middle), (unit, index.unit_graphemes(unit) - 1));
            }
            start = range.end;
            graphemes += index.unit_graphemes(unit);
            lines += usize::from(index.ends_line(unit));
        }
        let end = index.unit_of_grapheme(graphemes);
        assert_eq!(end.0, index.len() - 1, "the end of the text is in the last unit");
        assert_eq!(index.unit_of_byte(text.len()), index.len() - 1);
        let _ = Bias::Left;
    }
}

/// What indexing costs at the sizes of files, and what one edit costs once it is indexed:
///
/// ```text
/// BENCH_MB=50 cargo run -p aimer_text_buffer --example big_index
/// ```
#[allow(dead_code)]
fn bench_documentation() {}


// ── Counting graphemes of text that never joins ─────────────────────────────────

fn graphemes_of(text: &str) -> usize {
    text.graphemes(true).count()
}

#[test]
fn every_character_the_fast_count_accepts_is_a_grapheme_on_its_own_by_the_unicode_rules() {
    // The rules a character can join by, tried with a representative of each kind of neighbour:
    // a letter, a combining mark, a joiner, a spacing mark, a prepended character, the three Hangul
    // jamo, a regional indicator and an emoji.
    let neighbours = ['a', '\u{301}', '\u{200d}', '\u{903}', '\u{600}', '\u{1100}', '\u{1161}', '\u{11a8}', '\u{1f1e6}', '\u{1f468}'];
    let mut accepted = 0;
    for code in 0..=0x10FFFF_u32 {
        let Some(c) = char::from_u32(code) else { continue };
        let mut utf8 = [0_u8; 4];
        let bytes = c.encode_utf8(&mut utf8).as_bytes();
        let Some(count) = aimer_simd::kernels::simple_grapheme_count(bytes) else { continue };
        accepted += 1;
        assert_eq!(count, 1, "U+{code:04X}");
        assert_eq!(graphemes_of(&c.to_string()), 1, "U+{code:04X}");
        if c == '\r' || c == '\n' {
            continue;
        }
        for neighbour in neighbours {
            // A neighbour that joins whatever comes next (an extender, a joiner, a spacing mark) is
            // not what is asked: the character must not join on its own account.
            let before = format!("{neighbour}{c}");
            let after = format!("{c}{neighbour}");
            let joins_before = graphemes_of(&before) == 1;
            let joins_after = graphemes_of(&after) == 1;
            let extender = matches!(neighbour, '\u{301}' | '\u{200d}' | '\u{903}');
            let prepend = neighbour == '\u{600}';
            let hangul = matches!(neighbour, '\u{1100}' | '\u{1161}' | '\u{11a8}');
            let regional = neighbour == '\u{1f1e6}';
            // Something before it: the character is not an extender, spacing mark or jamo.
            if !extender && !hangul && !regional {
                assert!(!joins_before || prepend, "U+{code:04X} joins what is before it ({neighbour:?})");
            } else if hangul || regional {
                assert!(!joins_before, "U+{code:04X} joins what is before it ({neighbour:?})");
            }
            // Something after it: not a prepended character, and not a jamo or indicator pair.
            if !extender {
                assert!(!joins_after, "U+{code:04X} joins what is after it ({neighbour:?})");
            }
        }
    }
    assert!(accepted > 30_000, "{accepted} code points are accepted: the ranges cover Latin and the ideographs");
}

#[test]
fn the_fast_count_is_the_unicode_count_for_any_text_it_accepts_and_declines_the_rest() {
    let pieces = ["a", "Z", " ", "\n", "\r", "\r\n", "\u{e9}", "\u{ff}", "\u{416}", "\u{4f60}", "\u{597d}", "\u{ff0c}", "\u{3042}", "\u{1ebf}", "\u{3b1}"];
    let joining = ["\u{301}", "\u{200d}", "\u{1f600}", "\u{1100}", "\u{ac00}", "\u{e01}", "\u{903}", "\u{1f1e6}"];
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |below: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % below as u64) as usize
    };
    for round in 0..3000 {
        let mut text = String::new();
        for _ in 0..next(40) {
            text.push_str(pieces[next(pieces.len())]);
        }
        let with_joiner = round % 3 == 0 && !text.is_empty();
        if with_joiner {
            let at = next(text.len() + 1);
            let at = (0..=at).rev().find(|&byte| text.is_char_boundary(byte)).unwrap_or(0);
            text.insert_str(at, joining[next(joining.len())]);
        }
        match aimer_simd::kernels::simple_grapheme_count(text.as_bytes()) {
            Some(count) => assert_eq!(count, graphemes_of(&text), "{text:?}"),
            None => assert!(with_joiner, "a text of characters that stand alone was declined: {text:?}"),
        }
        assert_eq!(crate::layout_cuts::grapheme_count_of(&text) as usize, graphemes_of(&text), "{text:?}");
    }
}
