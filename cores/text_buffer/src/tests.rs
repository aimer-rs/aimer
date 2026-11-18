//! The buffer against a `String`, which it must always agree with, in chunks small enough that
//! every join and boundary comes up (`CHUNK_TARGET` is 16 bytes under test).

use std::ops::Range;

use aimer_sum_tree::Bias;
use unicode_segmentation::UnicodeSegmentation;

use crate::{CHUNK_MAX, TextBuffer, TextSnapshot};

/// A small deterministic generator, so a failure reproduces.
struct Random(u64);

impl Random {
    fn below(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % below as u64) as usize
    }
}

/// Pieces that make text hard: line feeds and carriage returns (alone and as pairs), accents that
/// attach to the character before, joiners, regional indicators that pair, Hangul jamo, Indic and
/// Khmer signs, and characters of every width.
const PIECES: [&str; 26] = [
    "a", "b", " ", "\n", "\r", "\r\n", "1", "-", "e", "\u{301}", "\u{e9}", "\u{200d}", "\u{fe0f}", "\u{1f468}", "\u{1f469}", "\u{1f1e6}",
    "\u{1f1e7}", "\u{1f1e8}", "\u{1100}", "\u{1161}", "\u{11a8}", "\u{915}", "\u{94d}", "\u{1780}", "\u{17d2}", "\u{4f60}",
];

fn random_text(random: &mut Random, pieces: usize) -> String {
    (0..pieces).map(|_| PIECES[random.below(PIECES.len())]).collect()
}

fn char_boundaries(text: &str) -> Vec<usize> {
    (0..=text.len()).filter(|&at| text.is_char_boundary(at)).collect()
}

/// Every question the snapshot answers, against the string it stands for.
#[track_caller]
fn assert_agrees(snapshot: &TextSnapshot, oracle: &str, random: &mut Random, what: &str) {
    assert_eq!(snapshot.len(), oracle.len(), "{what}: len");
    assert_eq!(snapshot.to_string(), oracle, "{what}: text");
    assert_eq!(snapshot.is_ascii(), oracle.is_ascii(), "{what}: is_ascii");
    assert_eq!(snapshot.has_carriage_return(), oracle.contains('\r'), "{what}: has_carriage_return");
    assert_eq!(snapshot.line_count(), oracle.matches('\n').count() + 1, "{what}: line_count");

    // The chunks themselves: none empty, none past the size edits merge up to. (A carriage return
    // and its line feed may end up in two chunks, when text is joined to a neighbour: every
    // question here is answered across the join, and the grapheme ones are checked below.)
    for chunk in snapshot.rope.iter() {
        assert!(!chunk.is_empty(), "{what}: an empty chunk");
        assert!(chunk.len() <= 2 * CHUNK_MAX, "{what}: a chunk of {} bytes", chunk.len());
    }
    snapshot.rope.check_invariants();

    for at in 0..=oracle.len() {
        assert_eq!(snapshot.is_char_boundary(at), oracle.is_char_boundary(at), "{what}: is_char_boundary {at}");
        assert_eq!(snapshot.byte_at(at), oracle.as_bytes().get(at).copied(), "{what}: byte_at {at}");
        if oracle.is_char_boundary(at) {
            assert_eq!(snapshot.byte_to_line(at), oracle[..at].matches('\n').count(), "{what}: byte_to_line {at}");
        }
    }
    for line in 0..snapshot.line_count() + 2 {
        let start = if line == 0 { 0 } else { oracle.match_indices('\n').nth(line - 1).map_or(oracle.len(), |(at, _)| at + 1) };
        let end = oracle.match_indices('\n').nth(line).map_or(oracle.len(), |(at, _)| at + 1);
        let end = if line >= snapshot.line_count() { oracle.len() } else { end };
        assert_eq!(snapshot.line_to_byte(line), start, "{what}: line_to_byte {line}");
        assert_eq!(snapshot.line_range(line), start..end.max(start), "{what}: line_range {line}");
    }

    let boundaries = char_boundaries(oracle);
    for _ in 0..12 {
        let a = boundaries[random.below(boundaries.len())];
        let b = boundaries[random.below(boundaries.len())];
        let range: Range<usize> = a.min(b)..a.max(b);
        assert_eq!(snapshot.slice(range.clone()), &oracle[range.clone()], "{what}: slice {range:?}");
        assert_eq!(snapshot.chunks(range.clone()).collect::<String>(), &oracle[range.clone()], "{what}: chunks {range:?}");
        assert_eq!(snapshot.slice_snapshot(range.clone()).to_string(), &oracle[range.clone()], "{what}: slice_snapshot {range:?}");
        snapshot.slice_snapshot(range).rope.check_invariants();
    }

    // Grapheme boundaries, from every character boundary, as a walk over the whole string finds them.
    let graphemes: Vec<usize> = oracle.grapheme_indices(true).map(|(at, _)| at).chain([oracle.len()]).collect();
    for &at in &boundaries {
        assert_eq!(snapshot.is_grapheme_boundary(at), graphemes.contains(&at), "{what}: is_grapheme_boundary {at}");
        let floor = graphemes.iter().copied().take_while(|&b| b <= at).last().unwrap_or(0);
        let ceil = graphemes.iter().copied().find(|&b| b >= at).unwrap_or(oracle.len());
        assert_eq!(snapshot.floor_grapheme_boundary(at), floor, "{what}: floor {at}");
        assert_eq!(snapshot.ceil_grapheme_boundary(at), ceil, "{what}: ceil {at}");
        let next = graphemes.iter().copied().find(|&b| b > at).unwrap_or(oracle.len());
        let previous = graphemes.iter().copied().take_while(|&b| b < at).last().unwrap_or(0);
        if at < oracle.len() && graphemes.contains(&at) {
            assert_eq!(snapshot.next_grapheme_boundary(at), next, "{what}: next {at}");
        }
        if at > 0 && graphemes.contains(&at) {
            assert_eq!(snapshot.prev_grapheme_boundary(at), previous, "{what}: prev {at}");
        }
    }
}

#[test]
fn an_empty_text_answers_every_question() {
    let snapshot = TextSnapshot::new();
    assert_agrees(&snapshot, "", &mut Random(1), "empty");
    assert_eq!(snapshot.line_range(0), 0..0);
    assert_eq!(snapshot.next_grapheme_boundary(0), 0);
    assert_eq!(snapshot.prev_grapheme_boundary(0), 0);
    let mut buffer = TextBuffer::new();
    assert!(buffer.undo().is_none() && buffer.redo().is_none());
    buffer.replace(0..0, "");
    assert_eq!(buffer.snapshot().len(), 0);
}

#[test]
fn text_built_from_a_string_agrees_in_every_shape() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for round in 0..80 {
        let pieces = random.below(160);
        let oracle = random_text(&mut random, pieces);
        assert_agrees(&TextSnapshot::from_text(&oracle), &oracle, &mut random, &format!("round {round}"));
    }
}

#[test]
fn random_edits_keep_the_buffer_equal_to_the_string_and_leave_snapshots_alone() {
    let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
    for round in 0..40 {
        let pieces = random.below(60);
        let mut oracle = random_text(&mut random, pieces);
        let mut buffer = TextBuffer::from_text(&oracle);
        for edit in 0..30 {
            let boundaries = char_boundaries(&oracle);
            let a = boundaries[random.below(boundaries.len())];
            let b = match random.below(4) {
                0 => boundaries[random.below(boundaries.len())],
                _ => boundaries.iter().copied().find(|&at| at >= a + random.below(6)).unwrap_or(oracle.len()),
            };
            let range = a.min(b)..a.max(b);
            let count = match random.below(5) {
                0 => 0,
                1 => random.below(40),
                _ => random.below(4),
            };
            let inserted = random_text(&mut random, count);
            let before = buffer.snapshot();
            let before_text = oracle.clone();

            let delta = buffer.replace(range.clone(), &inserted);
            oracle.replace_range(range.clone(), &inserted);

            let what = format!("round {round} edit {edit}: {range:?} -> {inserted:?}");
            assert_agrees(&buffer.snapshot(), &oracle, &mut random, &what);
            assert_eq!(before.to_string(), before_text, "{what}: the snapshot taken before is unchanged");
            assert_eq!(delta.range(), range, "{what}");
            assert_eq!(delta.removed().to_string(), before_text[range.clone()], "{what}: removed");
            assert_eq!(delta.inserted().to_string(), inserted, "{what}: inserted");
            assert_eq!(delta.len_change(), inserted.len() as isize - range.len() as isize, "{what}");
        }
    }
}

#[test]
fn undoing_every_edit_gives_back_every_earlier_text_and_redoing_gives_them_forward() {
    let mut random = Random(0x1234_5678_9ABC_DEF1);
    for round in 0..30 {
        let pieces = random.below(50);
        let mut oracle = random_text(&mut random, pieces);
        let mut buffer = TextBuffer::from_text(&oracle);
        let mut states = vec![oracle.clone()];
        for _ in 0..25 {
            let boundaries = char_boundaries(&oracle);
            let a = boundaries[random.below(boundaries.len())];
            let b = boundaries.iter().copied().find(|&at| at >= a + random.below(8)).unwrap_or(oracle.len());
            // Some of the edits are big enough to be shared by undo and not copied.
            let count = if random.below(6) == 0 { 30 + random.below(80) } else { random.below(5) };
            let inserted = random_text(&mut random, count);
            buffer.replace(a..b, &inserted);
            oracle.replace_range(a..b, &inserted);
            states.push(oracle.clone());
        }
        for (step, state) in states.iter().enumerate().rev().skip(1) {
            let delta = buffer.undo().expect("there is a step to undo");
            let what = format!("round {round}: undone to state {step}");
            assert_agrees(&buffer.snapshot(), state, &mut random, &what);
            assert_eq!(delta.removed().len() as isize - delta.inserted().len() as isize, -delta.len_change());
        }
        assert!(buffer.undo().is_none());
        for (step, state) in states.iter().enumerate().skip(1) {
            buffer.redo().expect("there is a step to redo");
            assert_agrees(&buffer.snapshot(), state, &mut random, &format!("round {round}: redone to state {step}"));
        }
        assert!(buffer.redo().is_none() && buffer.can_undo());
    }
}

#[test]
fn an_edit_after_an_undo_forgets_what_could_have_been_redone() {
    let mut buffer = TextBuffer::from_text("abc");
    buffer.replace(3..3, "d");
    buffer.undo();
    assert!(buffer.can_redo());
    buffer.replace(0..0, "x");
    assert!(!buffer.can_redo());
    assert_eq!(buffer.snapshot().to_string(), "xabc");
}

#[test]
fn typing_that_carries_on_is_one_step_and_a_line_feed_or_a_jump_starts_another() {
    let mut buffer = TextBuffer::from_text("");
    for (at, ch) in "hello".chars().enumerate() {
        buffer.replace_continuing(at..at, &ch.to_string());
    }
    buffer.undo();
    assert_eq!(buffer.snapshot().to_string(), "", "five characters were one step");

    buffer.redo();
    buffer.replace_continuing(5..5, "\n");
    buffer.replace_continuing(6..6, "x");
    buffer.undo();
    assert_eq!(buffer.snapshot().to_string(), "hello\n", "a line feed is a step of its own");
    buffer.undo();
    assert_eq!(buffer.snapshot().to_string(), "hello");
    buffer.replace_continuing(0..0, "A");
    buffer.replace_continuing(3..3, "B");
    buffer.undo();
    assert_eq!(buffer.snapshot().to_string(), "Ahello", "an edit somewhere else is a step of its own");
}

#[test]
fn the_history_holds_at_most_its_limit_and_always_the_latest_step() {
    let mut buffer = TextBuffer::from_text("");
    buffer.set_history_limit(100);
    for index in 0..50 {
        buffer.replace(index * 10..index * 10, "0123456789");
    }
    let mut undone = 0;
    while buffer.undo().is_some() {
        undone += 1;
    }
    assert_eq!(undone, 10, "ten steps of ten bytes are what a limit of 100 bytes keeps");
    assert_eq!(buffer.snapshot().len(), 400, "only the newest ten steps were taken back");

    let mut big = TextBuffer::from_text("x");
    big.set_history_limit(10);
    big.replace(1..1, &"y".repeat(5000));
    assert!(big.undo().is_some(), "the latest step stays however large");
    assert_eq!(big.snapshot().to_string(), "x");
}

#[test]
fn an_edit_moves_a_position_as_text_moves() {
    let mut buffer = TextBuffer::from_text("0123456789");
    let delta = buffer.replace(3..6, "ab");
    assert_eq!(delta.transform(2, Bias::Left), 2);
    assert_eq!(delta.transform(2, Bias::Right), 2);
    assert_eq!(delta.transform(8, Bias::Left), 7, "after it: moved by the change in length");
    assert_eq!(delta.transform(7, Bias::Right), 6);
    assert_eq!(delta.transform(4, Bias::Left), 3, "inside it: to the start of what replaced it");
    assert_eq!(delta.transform(4, Bias::Right), 5, "or to its end");
    assert_eq!((delta.transform(3, Bias::Left), delta.transform(3, Bias::Right)), (3, 5), "at its start");
    assert_eq!((delta.transform(6, Bias::Left), delta.transform(6, Bias::Right)), (3, 5), "and at its end");
    let insert = buffer.replace(5..5, "XY");
    assert_eq!(insert.transform(5, Bias::Left), 5, "at an insertion, left stays before it");
    assert_eq!(insert.transform(5, Bias::Right), 7, "and right goes after it");
}

#[test]
fn a_big_text_is_edited_in_the_middle_without_the_chunk_count_changing_much() {
    let line = "the quick brown fox jumps over the lazy dog\n";
    let text = line.repeat(2000);
    let mut buffer = TextBuffer::from_text(&text);
    let chunks = buffer.snapshot().chunk_count();
    for step in 0..500 {
        let at = buffer.snapshot().len() / 2 + step;
        let at = buffer.snapshot().floor_grapheme_boundary(at);
        buffer.replace_continuing(at..at, "x");
    }
    let after = buffer.snapshot().chunk_count();
    // 500 bytes more is about 500 / CHUNK_TARGET chunks more, and not a chunk for each typed byte.
    assert!(after <= chunks + 3 * 500 / crate::CHUNK_TARGET, "{chunks} chunks became {after}: typing in one place must not fragment the text");
    let fill = buffer.snapshot().len() / after;
    assert!(fill >= crate::CHUNK_TARGET / 2, "chunks average {fill} bytes");
    buffer.snapshot().rope.check_invariants();
}

#[test]
fn a_snapshot_edited_without_a_buffer_shares_what_it_did_not_touch() {
    let text = "the quick brown fox jumps over the lazy dog\n".repeat(400);
    let before = TextSnapshot::from_text(&text);
    let after = before.with_replaced(8000..8003, "slow");
    let mut expected = text.clone();
    expected.replace_range(8000..8003, "slow");
    assert_eq!(after.to_string(), expected);
    assert_eq!(before.to_string(), text, "the snapshot it was made from is unchanged");
    let shared = after.rope.iter().filter(|chunk| before.rope.iter().any(|old| old.same(chunk))).count();
    assert!(shared + 6 >= before.chunk_count(), "{shared} of {} chunks are shared", before.chunk_count());
}

#[test]
fn the_difference_between_two_snapshots_is_what_an_edit_changed() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for round in 0..200 {
        let pieces = random.below(120);
        let mut oracle = random_text(&mut random, pieces);
        let mut snapshot = TextSnapshot::from_text(&oracle);
        let first = snapshot.clone();
        // One edit or several between the two snapshots that are compared.
        for _ in 0..1 + random.below(3) {
            let boundaries = char_boundaries(&oracle);
            let a = boundaries[random.below(boundaries.len())];
            let b = boundaries.iter().copied().find(|&at| at >= a + random.below(6)).unwrap_or(oracle.len());
            let count = random.below(8);
            let inserted = random_text(&mut random, count);
            oracle.replace_range(a..b, &inserted);
            snapshot = snapshot.with_replaced(a..b, &inserted);
        }
        let what = format!("round {round}");
        match first.diff(&snapshot) {
            None => assert_eq!(first.to_string(), snapshot.to_string(), "{what}: said equal"),
            Some((old, new)) => {
                let (old_text, new_text) = (first.to_string(), snapshot.to_string());
                assert_eq!(old_text[..old.start], new_text[..new.start], "{what}: the start is common");
                assert_eq!(old_text[old.end..], new_text[new.end..], "{what}: the end is common");
                assert!(old_text.is_char_boundary(old.start) && old_text.is_char_boundary(old.end), "{what}");
                assert!(new_text.is_char_boundary(new.start) && new_text.is_char_boundary(new.end), "{what}");
                assert_eq!(old.start, new.start);
                assert_ne!(old_text[old.clone()], new_text[new.clone()], "{what}: what differs does");
            }
        }
        // Two texts that share no chunks are compared by their bytes.
        let rebuilt = TextSnapshot::from_text(&oracle);
        assert_eq!(rebuilt.diff(&snapshot), None, "{what}: the same text built apart");
        if let Some((old, new)) = TextSnapshot::from_text(&first.to_string()).diff(&rebuilt) {
            assert_eq!(first.to_string()[..old.start], oracle[..new.start], "{what}: apart");
        }
    }
}


// ── Chunks that are slices of a text held elsewhere ─────────────────────────────

use crate::SharedText;

fn shared(text: &str) -> SharedText {
    std::sync::Arc::new(text.to_owned())
}

/// Texts that put every kind of boundary at a cut: multi-byte characters, a carriage return and a
/// line feed, combining marks, with chunks of 16 bytes under test.
fn awkward_texts() -> Vec<String> {
    vec![
        String::new(),
        "short".to_owned(),
        "caf\u{e9} \u{4f60}\u{597d} \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} e\u{301}".repeat(30),
        "line one\r\nline two\r\n".repeat(40),
        "a\n".repeat(200),
        "x".repeat(1000),
    ]
}

#[test]
fn a_rope_of_slices_reads_as_the_text_it_slices() {
    for text in awkward_texts() {
        let rope = TextSnapshot::from_shared(shared(&text));
        let copy = TextSnapshot::from_text(&text);
        assert_eq!(rope.to_string(), text);
        assert_eq!(rope.len(), copy.len());
        assert_eq!(rope.line_count(), copy.line_count());
        assert_eq!(rope.is_ascii(), copy.is_ascii());
        assert_eq!(rope.has_carriage_return(), copy.has_carriage_return());
        for byte in (0..=text.len()).filter(|byte| text.is_char_boundary(*byte)).step_by(7) {
            assert_eq!(rope.byte_to_line(byte), copy.byte_to_line(byte), "line of byte {byte}");
        }
        let mut random = Random(7);
        assert_agrees(&rope, &text, &mut random, "from_shared");
    }
}

#[test]
fn slices_hold_the_text_they_slice_and_copy_nothing() {
    let text = "the quick brown fox jumps over the lazy dog\n".repeat(100);
    let backing = shared(&text);
    let rope = TextSnapshot::from_shared(std::sync::Arc::clone(&backing));
    assert!(rope.chunk_count() > 10);
    // One handle for each chunk, and this one: the chunks point into the text, they do not hold a copy of it.
    assert_eq!(std::sync::Arc::strong_count(&backing), rope.chunk_count() + 1);
    let weak = std::sync::Arc::downgrade(&backing);
    drop(backing);
    assert!(weak.upgrade().is_some(), "the chunks keep the text alive");
    drop(rope);
    assert!(weak.upgrade().is_none(), "and it goes with the last of them");
}

#[test]
fn editing_a_rope_of_slices_copies_only_the_chunks_the_edit_touches() {
    let text = "the quick brown fox jumps over the lazy dog\n".repeat(200);
    let before = TextSnapshot::from_shared(shared(&text));
    let after = before.with_replaced(4000..4003, "slow");
    let mut expected = text.clone();
    expected.replace_range(4000..4003, "slow");
    assert_eq!(after.to_string(), expected);
    assert_eq!(before.to_string(), text);
    let untouched = after.rope.iter().filter(|chunk| before.rope.iter().any(|old| old.same(chunk))).count();
    assert!(untouched + 6 >= before.chunk_count(), "{untouched} of {} chunks are still the slices", before.chunk_count());
    let (old, new) = before.diff(&after).expect("they differ");
    assert!(old.len() < 200 && new.len() < 200, "{old:?} {new:?}: the difference is found from the chunks they share");
}

#[test]
fn replacing_a_range_with_a_snapshot_is_what_replacing_it_with_its_text_is() {
    let mut random = Random(0x2545_F491_4F6C_DD1D);
    let base = "alpha beta gamma\ndelta \u{4f60}\u{597d} epsilon\r\nzeta eta theta ".repeat(40);
    for round in 0..150 {
        let inserted: String = (0..random.below(900)).map(|i| ["a", "b c", "\n", "\u{e9}", "\u{1f600}"][(i + round) % 5]).collect();
        let at = |random: &mut Random| {
            let mut at = random.below(base.len() + 1);
            while !base.is_char_boundary(at) {
                at -= 1;
            }
            at
        };
        let (a, b) = (at(&mut random), at(&mut random));
        let range = a.min(b)..a.max(b);
        let with = TextSnapshot::from_shared(shared(&inserted));
        let result = TextSnapshot::from_text(&base).with_replaced_snapshot(range.clone(), &with);
        let mut expected = base.clone();
        expected.replace_range(range.clone(), &inserted);
        assert_eq!(result.to_string(), expected, "round {round}: {range:?} with {} bytes", inserted.len());
        let mut checker = Random(round as u64 + 1);
        assert_agrees(&result, &expected, &mut checker, "with_replaced_snapshot");
    }
}


#[test]
fn bytes_from_reads_the_whole_text_from_any_offset_a_chunk_at_a_time() {
    let text = "caf\u{e9} \u{4f60}\u{597d} line\nnext line, and a few more words to make chunks\n".repeat(6);
    let snapshot = TextSnapshot::from_text(&text);
    assert!(snapshot.chunk_count() > 4);
    // Every offset, even in the middle of a character.
    for from in (0..=text.len()).step_by(3) {
        let mut read = Vec::new();
        let mut at = from;
        while let Some(piece) = snapshot.bytes_from(at) {
            assert!(!piece.is_empty());
            read.extend_from_slice(piece);
            at += piece.len();
        }
        assert_eq!(read, &text.as_bytes()[from..], "from {from}");
    }
    assert!(snapshot.bytes_from(text.len()).is_none(), "nothing at the end");
}
