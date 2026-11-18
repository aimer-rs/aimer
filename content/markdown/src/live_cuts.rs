//! Finding the pieces of a very long note near a place, without reading the note.
//!
//! [`segment_cuts`](crate::live_segments::segment_cuts) reads every line of a text to say where its
//! pieces start, which is the cost of the note at each key typed. The state a line is read in is
//! small, though (see [`Scan`]), and the lines before the window are not needed except for it. So
//! the state is kept every few kilobytes, and the pieces around a place are found by reading on
//! from the last state before it.
//!
//! An edit leaves what comes before it as it was, so it costs the states after it, which are made
//! again as a window reaches them: typing in a window never reads past it, and scrolling to a place
//! far from the last edit reads the lines between, once.
//!
//! Where a link reference or footnote definition is does not decide a cut here, as it does when a
//! note is read whole: a definition is read with the piece it is in, so a reference in another
//! piece does not find it. That is the price of not reading the note, and the reason a note is
//! read whole until it is [long](crate::live_segments::WINDOWED_BYTES).

use std::ops::Range;

use aimer_text_buffer::TextSnapshot;

use crate::live_segments::{Scan, Step};

/// How far apart, in bytes, the states are kept.
const CHECKPOINT_BYTES: usize = 32 * 1024;

/// How much text is read at once.
const BLOCK_BYTES: usize = 64 * 1024;

/// The states of a note's scan, for the text the index follows.
pub(crate) struct CutIndex {
    text: TextSnapshot,
    /// States at line starts, in order, each made by reading the text before it.
    checkpoints: Vec<Scan>,
}

impl CutIndex {
    pub(crate) fn new() -> Self {
        Self { text: TextSnapshot::new(), checkpoints: Vec::new() }
    }

    /// Follows the index to `text`, which is the text it knew edited: the states before the first
    /// change are still true, and the others are dropped.
    pub(crate) fn follow(&mut self, text: &TextSnapshot) {
        if self.text.ptr_eq(text) {
            return;
        }
        let kept = match self.text.diff(text) {
            None => usize::MAX,
            Some((changed, _)) => changed.start,
        };
        let keep = self.checkpoints.partition_point(|state| state.at <= kept);
        self.checkpoints.truncate(keep);
        self.text = text.clone();
    }

    /// The nearest character boundary at or before `byte`.
    fn boundary(&self, mut byte: usize) -> usize {
        while !self.text.is_char_boundary(byte) {
            byte -= 1;
        }
        byte
    }

    /// The pieces that cover `range`, in bytes: from the start of the one that holds its first
    /// byte, to the end of the one that holds its last. Each is cut where the whole note would be.
    pub(crate) fn pieces(&mut self, range: Range<usize>) -> Vec<Range<usize>> {
        let len = self.text.len();
        let range = range.start.min(len)..range.end.min(len);
        let from = self.checkpoints.partition_point(|state| state.at <= range.start);
        let mut state = if from == 0 { Scan::START } else { self.checkpoints[from - 1] };
        let mut first = state.last_cut;
        let mut cuts: Vec<usize> = Vec::new();
        let mut end = len;
        let mut done = false;
        let mut at = state.at;
        while at < len && !done {
            // A block of whole lines; a line longer than a block makes the block longer.
            let mut stop = self.boundary((at + BLOCK_BYTES).min(len));
            let block = loop {
                let block = self.text.slice(at..stop);
                if stop == len {
                    break block;
                }
                match block.rfind('\n') {
                    Some(last) => break self.text.slice(at..at + last + 1),
                    None => stop = self.boundary((stop + BLOCK_BYTES).min(len)),
                }
            };
            for line in block.split_inclusive('\n') {
                if state.at >= self.checkpoints.last().map_or(0, |last| last.at) + CHECKPOINT_BYTES {
                    self.checkpoints.push(state);
                }
                let line_start = state.at;
                if let Step::Cut = state.step(line) {
                    if line_start <= range.start {
                        first = line_start;
                    } else if line_start >= range.end {
                        end = line_start;
                        done = true;
                        break;
                    } else {
                        cuts.push(line_start);
                    }
                }
            }
            at = state.at;
        }
        let mut pieces = Vec::with_capacity(cuts.len() + 1);
        let mut start = first;
        for cut in cuts {
            pieces.push(start..cut);
            start = cut;
        }
        pieces.push(start..end);
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live_segments::segment_cuts;

    struct Random(u64);

    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    /// A note of the blocks a live preview meets: paragraphs, lists, quotes, fences that hold
    /// blank lines, and html.
    fn note(random: &mut Random, blocks: usize) -> String {
        let mut text = String::new();
        for index in 0..blocks {
            match random.next(8) {
                0 => text.push_str(&format!("# Heading {index}\n\n")),
                1 => text.push_str(&format!("- item {index}\n- item two\n\n")),
                2 => text.push_str(&format!("> quoted {index} {}\n\n", "words ".repeat(random.next(40)))),
                3 => text.push_str(&format!("```rust\nfn a{index}() {{}}\n\nlet b = {index};\n```\n\n")),
                4 => text.push_str(&format!("<!--\n\ncomment {index}\n-->\n\n")),
                5 => text.push_str(&format!("| a | b |\n|---|---|\n| {index} | 2 |\n\n")),
                6 => text.push_str(&format!("  indented under it {index}\n\n")),
                _ => text.push_str(&format!("{}\n\n", format!("paragraph {index} words ").repeat(random.next(60) + 1))),
            }
        }
        text
    }

    /// The pieces the whole scan makes, as spans.
    fn whole_pieces(text: &str) -> Vec<Range<usize>> {
        let cuts = segment_cuts(text).unwrap_or_default();
        let mut start = 0;
        let mut pieces = Vec::new();
        for cut in cuts {
            pieces.push(start..cut);
            start = cut;
        }
        pieces.push(start..text.len());
        pieces
    }

    #[test]
    fn the_pieces_around_a_range_are_the_ones_the_whole_scan_makes() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for blocks in [30, 200, 900] {
            let text = note(&mut random, blocks);
            let pieces = whole_pieces(&text);
            assert!(pieces.len() > 3, "{} pieces", pieces.len());
            let mut index = CutIndex::new();
            index.follow(&TextSnapshot::from_text(&text));
            for _ in 0..60 {
                let start = random.next(text.len());
                let end = (start + random.next(6000)).min(text.len());
                let found = index.pieces(start..end);
                let first = pieces.iter().position(|piece| piece.contains(&start)).unwrap_or(pieces.len() - 1);
                let last = pieces.iter().position(|piece| piece.contains(&end.saturating_sub(1).max(start))).unwrap_or(pieces.len() - 1);
                assert_eq!(found, pieces[first..=last].to_vec(), "range {start}..{end} of {} bytes", text.len());
            }
        }
    }

    #[test]
    fn the_pieces_after_an_edit_are_the_ones_the_edited_note_is_cut_into() {
        let mut random = Random(0xD1B5_4A32_D192_ED03);
        let text = note(&mut random, 600);
        let mut index = CutIndex::new();
        let mut snapshot = TextSnapshot::from_text(&text);
        index.follow(&snapshot);
        // Read far in, so states are kept past where the edits are.
        index.pieces(text.len() - 100..text.len());
        for round in 0..40 {
            let at = random.next(snapshot.len());
            let at = (0..=at).rev().find(|&byte| snapshot.is_char_boundary(byte)).unwrap();
            let inserted = ["x", "\n\n", "```\n", "\n\nnew block\n\n", "<!--", "-->"][random.next(6)];
            snapshot = snapshot.with_replaced(at..at, inserted);
            index.follow(&snapshot);
            let whole = snapshot.to_string();
            let pieces = whole_pieces(&whole);
            for place in [at, random.next(whole.len()), whole.len() - 1] {
                let found = index.pieces(place..place + 1);
                let held = pieces.iter().position(|piece| piece.contains(&place)).unwrap();
                assert_eq!(found, vec![pieces[held].clone()], "round {round}: byte {place}, edit {inserted:?} at {at}");
            }
        }
    }

    #[test]
    fn a_range_far_from_the_states_reads_on_from_the_nearest_one_before_it() {
        let mut random = Random(77);
        let text = note(&mut random, 1500);
        let mut index = CutIndex::new();
        index.follow(&TextSnapshot::from_text(&text));
        index.pieces(0..10);
        let kept = index.checkpoints.len();
        assert!(kept <= 2, "a window at the top reads only the top: {kept} states");
        index.pieces(text.len() / 2..text.len() / 2 + 100);
        assert!(index.checkpoints.len() > kept, "reading on from the top to the middle leaves states between");
        assert!(index.checkpoints.windows(2).all(|pair| pair[0].at < pair[1].at));
    }

    #[test]
    fn a_text_with_no_cut_is_one_piece() {
        let text = "one long line without a break ".repeat(2000);
        let mut index = CutIndex::new();
        index.follow(&TextSnapshot::from_text(&text));
        assert_eq!(index.pieces(100..200), vec![0..text.len()]);
        assert_eq!(index.pieces(0..0), vec![0..text.len()]);
    }
}
