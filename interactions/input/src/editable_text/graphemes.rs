//! Where the graphemes of a long text are, without walking it from its start.
//!
//! A cursor is counted in graphemes and an edit is made in bytes, and going from one to the other
//! by walking the text from its start costs the whole text for each key typed. Most text is ASCII,
//! where a grapheme is a byte, and needs no walk at all. Other text keeps a mark every so many
//! graphemes (a [`GraphemeIndex`]), so an offset is found from the mark before it and the
//! graphemes between, and an edit moves the marks it passes instead of making them again.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

pub(crate) use super::layout::is_one_byte_graphemes;

/// About how many graphemes lie between two marks: a lookup walks at most this many. The tests
/// use a short one, so that the short texts they can check against a walk have many marks.
const STRIDE: usize = if cfg!(test) { 8 } else { 128 };

/// A mark on a grapheme boundary: its byte offset and how many graphemes come before it.
type Mark = (usize, usize);

/// The graphemes of one text, found by the nearest mark and not by a walk from the start.
///
/// The marks are on boundaries of the text, so a walk from one is the walk the whole text would
/// make there: a boundary leaves nothing before it that the next grapheme's extent depends on.
#[derive(Clone, Debug)]
pub(crate) struct GraphemeIndex {
    /// In increasing order, the first at the start of the text.
    marks: Vec<Mark>,
    total: usize,
}

impl GraphemeIndex {
    /// The index of `text`, made by one walk over it.
    pub(crate) fn build(text: &str) -> Self {
        let mut marks = vec![(0, 0)];
        let mut total = 0;
        for (byte, _) in text.grapheme_indices(true) {
            if total > 0 && total - marks[marks.len() - 1].1 >= STRIDE {
                marks.push((byte, total));
            }
            total += 1;
        }
        Self { marks, total }
    }

    /// How many graphemes the text has.
    #[inline]
    pub(crate) fn count(&self) -> usize {
        self.total
    }

    /// The nearest mark at or before the grapheme `offset`.
    fn mark_at_grapheme(&self, offset: usize) -> Mark {
        self.marks[self.marks.partition_point(|mark| mark.1 <= offset).saturating_sub(1)]
    }

    /// The nearest mark at or before the byte `byte`.
    fn mark_at_byte(&self, byte: usize) -> Mark {
        self.marks[self.marks.partition_point(|mark| mark.0 <= byte).saturating_sub(1)]
    }

    /// The byte offset of the grapheme `offset`; the end of the text for one past the last.
    pub(crate) fn byte_of(&self, text: &str, offset: usize) -> usize {
        if offset >= self.total {
            return text.len();
        }
        let (byte, grapheme) = self.mark_at_grapheme(offset);
        text[byte..]
            .grapheme_indices(true)
            .nth(offset - grapheme)
            .map_or(text.len(), |(at, _)| byte + at)
    }

    /// How many graphemes come before the byte `byte`, which is on a boundary.
    pub(crate) fn grapheme_of(&self, text: &str, byte: usize) -> usize {
        if byte >= text.len() {
            return self.total;
        }
        let (mark, grapheme) = self.mark_at_byte(byte);
        grapheme + text[mark..byte].graphemes(true).count()
    }

    /// The greatest boundary at or before `byte`.
    pub(crate) fn floor(&self, text: &str, byte: usize) -> usize {
        let byte = byte.min(text.len());
        if byte == text.len() {
            return byte;
        }
        let (mark, _) = self.mark_at_byte(byte);
        text[mark..]
            .grapheme_indices(true)
            .map(|(at, _)| mark + at)
            .take_while(|&start| start <= byte)
            .last()
            .unwrap_or(mark)
    }

    /// The least boundary at or after `byte`.
    pub(crate) fn ceil(&self, text: &str, byte: usize) -> usize {
        let byte = byte.min(text.len());
        let (mark, _) = self.mark_at_byte(byte);
        text[mark..]
            .grapheme_indices(true)
            .map(|(at, _)| mark + at)
            .find(|&start| start >= byte)
            .unwrap_or(text.len())
    }

    /// The index of `new`, which is `old` with the bytes `replaced` of it taken out and
    /// `inserted` bytes put in their place; this index is `old`'s.
    ///
    /// The marks before the edit are still on boundaries: a boundary is decided by what is
    /// before it and the one character after, and none of that changed. After the edit the
    /// graphemes are walked again until a boundary falls where one of the old marks was, moved by
    /// the size of the edit; from there on the text is the same, and so are the boundaries, and
    /// the rest of the marks are carried over, moved.
    pub(crate) fn spliced(&self, new: &str, replaced: Range<usize>, inserted: usize) -> Self {
        let kept = self.marks.partition_point(|mark| mark.0 < replaced.start);
        let (from_byte, from_grapheme) = kept.checked_sub(1).map_or((0, 0), |last| self.marks[last]);
        let delta = inserted as isize - replaced.len() as isize;
        let after_edit = replaced.start + inserted;

        let mut marks = self.marks[..kept].to_vec();
        if marks.is_empty() {
            marks.push((0, 0));
        }
        let mut grapheme = from_grapheme;
        let mut last_mark = marks[marks.len() - 1].1;
        let mut old_mark = kept;
        for (offset, _) in new[from_byte..].grapheme_indices(true) {
            let byte = from_byte + offset;
            if byte >= after_edit {
                // The old mark that would be here, if there is one.
                let old_byte = (byte as isize - delta) as usize;
                while old_mark < self.marks.len() && self.marks[old_mark].0 < old_byte {
                    old_mark += 1;
                }
                if let Some(&(mark_byte, mark_grapheme)) = self.marks.get(old_mark)
                    && mark_byte == old_byte
                    && mark_byte >= replaced.end
                {
                    // Back in step: what follows is what the old text had, moved.
                    let moved = grapheme as isize - mark_grapheme as isize;
                    marks.push((byte, grapheme));
                    marks.extend(self.marks[old_mark + 1..].iter().map(|&(mark_byte, mark_grapheme)| {
                        ((mark_byte as isize + delta) as usize, (mark_grapheme as isize + moved) as usize)
                    }));
                    let total = (self.total as isize + moved) as usize;
                    marks.dedup_by_key(|mark| mark.0);
                    return Self { marks, total };
                }
            }
            if byte > from_byte && grapheme - last_mark >= STRIDE {
                marks.push((byte, grapheme));
                last_mark = grapheme;
            }
            grapheme += 1;
        }
        Self { marks, total: grapheme }
    }
}

/// The greatest boundary at or before `offset` in `text`, found without an index: from the
/// start of the line the offset is on, which is a boundary whatever is before it.
pub(crate) fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    // An offset inside a character is on no boundary: start from the character's start.
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    if offset == text.len() {
        return offset;
    }
    let bytes = text.as_bytes();
    // Two ASCII neighbours are a boundary, unless they are a carriage return and a line feed.
    if offset == 0 || (bytes[offset - 1] < 0x80 && bytes[offset] < 0x80 && !(bytes[offset - 1] == b'\r' && bytes[offset] == b'\n')) {
        return offset;
    }
    let line = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    text[line..]
        .grapheme_indices(true)
        .map(|(at, _)| line + at)
        .take_while(|&start| start <= offset)
        .last()
        .unwrap_or(line)
}

/// The least boundary at or after `offset` in `text`, found without an index.
pub(crate) fn ceil_boundary(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let floor = floor_boundary(text, offset);
    if floor == offset {
        return offset;
    }
    // The next boundary after the grapheme the offset is inside.
    text[floor..]
        .grapheme_indices(true)
        .map(|(at, _)| floor + at)
        .find(|&start| start >= offset)
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so the tests are the same on every run.
    struct Random(u64);
    impl Random {
        fn next(&mut self, below: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % below as u64) as usize
        }
    }

    /// Pieces that make graphemes hard: more than one scalar, joined by what comes before, and
    /// whose boundaries move when a neighbour does.
    const PIECES: [&str; 20] = [
        "a", "b", " ", "\n", "\r\n", "\r", "e", "\u{301}", "\u{e9}", "\u{4f60}", "\u{1f468}", "\u{200d}",
        "\u{1f469}", "\u{1f1e6}", "\u{1f1e7}", "\u{1f1e8}", "\u{1780}\u{17d2}\u{1794}", "\u{17bb}", "\u{1100}", "\u{1161}",
    ];

    fn random_text(random: &mut Random, pieces: usize) -> String {
        (0..pieces).map(|_| PIECES[random.next(PIECES.len())]).collect()
    }

    fn boundaries(text: &str) -> Vec<usize> {
        text.grapheme_indices(true).map(|(at, _)| at).chain([text.len()]).collect()
    }

    /// Every question the index answers, against the walk it stands for.
    fn assert_agrees(index: &GraphemeIndex, text: &str, what: &str) {
        let boundaries = boundaries(text);
        assert_eq!(index.count(), boundaries.len() - 1, "{what}: count");
        for (grapheme, &byte) in boundaries.iter().enumerate() {
            assert_eq!(index.byte_of(text, grapheme), byte, "{what}: byte of grapheme {grapheme}");
            assert_eq!(index.grapheme_of(text, byte), grapheme, "{what}: grapheme of byte {byte}");
        }
        for byte in (0..=text.len()).filter(|&byte| text.is_char_boundary(byte)) {
            let floor = boundaries.iter().copied().take_while(|&b| b <= byte).last().unwrap();
            let ceil = boundaries.iter().copied().find(|&b| b >= byte).unwrap();
            assert_eq!(index.floor(text, byte), floor, "{what}: floor of {byte}");
            assert_eq!(index.ceil(text, byte), ceil, "{what}: ceil of {byte}");
            assert_eq!(floor_boundary(text, byte), floor, "{what}: local floor of {byte}");
            assert_eq!(ceil_boundary(text, byte), ceil, "{what}: local ceil of {byte}");
        }
    }

    #[test]
    fn an_index_answers_what_a_walk_of_the_text_does() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for round in 0..60 {
            let pieces = random.next(900);
            let text = random_text(&mut random, pieces);
            assert_agrees(&GraphemeIndex::build(&text), &text, &format!("round {round}"));
        }
        assert_agrees(&GraphemeIndex::build(""), "", "empty");
    }

    #[test]
    fn a_spliced_index_answers_for_the_edited_text() {
        let mut random = Random(0xDEAD_BEEF_CAFE_F00D);
        for round in 0..60 {
            // Long enough that there are marks to carry past the edit.
            let pieces = 100 + random.next(300);
            let mut text = random_text(&mut random, pieces);
            let mut index = GraphemeIndex::build(&text);
            // A few edits in a row, each made from the index the one before made.
            for edit in 0..4 {
                let spots = boundaries(&text);
                let from = spots[random.next(spots.len())];
                let far = spots[random.next(spots.len())].max(from);
                let span = random.next(8);
                let to = if random.next(3) == 0 {
                    far
                } else {
                    spots.iter().copied().find(|&b| b >= (from + span).min(text.len())).unwrap_or(text.len()).max(from)
                };
                let count = random.next(5);
                let inserted = random_text(&mut random, count);
                let mut edited = String::with_capacity(text.len() + inserted.len());
                edited.push_str(&text[..from]);
                edited.push_str(&inserted);
                edited.push_str(&text[to..]);

                let spliced = index.spliced(&edited, from..to, inserted.len());
                assert_agrees(&spliced, &edited, &format!("round {round} edit {edit}: {from}..{to} for {inserted:?}"));
                text = edited;
                index = spliced;
            }
        }
    }

    #[test]
    fn marks_stay_a_stride_apart_however_often_the_same_place_is_edited() {
        let mut text = "word ".repeat(2000);
        let mut index = GraphemeIndex::build(&text);
        for _ in 0..600 {
            let at = text.len() / 2;
            text.insert(at, 'x');
            index = index.spliced(&text, at..at, 1);
        }
        let widest = index.marks.windows(2).map(|pair| pair[1].1 - pair[0].1).max().unwrap();
        assert!(widest <= 2 * STRIDE, "the marks drifted apart: {widest} graphemes between two");
        assert_agrees(&index, &text, "after typing in one place");
    }
}
