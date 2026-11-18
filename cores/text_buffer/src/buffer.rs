use std::ops::Range;

use aimer_sum_tree::{Bias, SumTree};

use crate::history::History;
use crate::rope::{TextSnapshot, chunks_of};
use crate::summary::{ByteAndLines, Chunk};
use crate::{CHUNK_MAX, CHUNK_TARGET};

/// What one edit changed, in the coordinates of the text before it.
#[derive(Clone, Debug)]
pub struct EditDelta {
    pub(crate) range: Range<usize>,
    pub(crate) removed: TextSnapshot,
    pub(crate) inserted: TextSnapshot,
}

impl EditDelta {
    /// An edit that replaced `removed` (which was at `range` in the earlier text) with `inserted`.
    ///
    /// For an edit that was not made through a [`TextBuffer`], such as the difference found between
    /// two snapshots by [`TextSnapshot::diff`].
    pub fn new(range: Range<usize>, removed: TextSnapshot, inserted: TextSnapshot) -> Self {
        debug_assert_eq!(removed.len(), range.len());
        Self { range, removed, inserted }
    }

    /// The bytes the edit replaced, in the text before it.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// The text the edit took out.
    pub fn removed(&self) -> &TextSnapshot {
        &self.removed
    }

    /// The text the edit put in, which now starts at `range().start`.
    pub fn inserted(&self) -> &TextSnapshot {
        &self.inserted
    }

    /// How many bytes the text grew by, which is negative when it shrank.
    pub fn len_change(&self) -> isize {
        self.inserted.len() as isize - self.removed.len() as isize
    }

    /// Where `offset`, a position in the text before the edit, is after it.
    ///
    /// A position before the edit stays and one after it moves by the change in length. A
    /// position inside what was replaced, or at either end of it, goes to the start of the new
    /// text (`bias` left) or after it (`bias` right); at a pure insertion left stays before the
    /// new text and right goes after it. Left sticks to the text before the position, right to
    /// the text after it.
    pub fn transform(&self, offset: usize, bias: Bias) -> usize {
        let (start, end) = (self.range.start, self.range.end);
        if offset < start || (offset == start && start == end && bias == Bias::Left) {
            offset
        } else if offset > end {
            (offset as isize + self.len_change()) as usize
        } else if bias == Bias::Left {
            start
        } else {
            start + self.inserted.len()
        }
    }
}

/// A text that is edited, and the history that undoes the edits.
///
/// An edit copies the chunk or two it touches, so it costs the same in a kilobyte of text and in
/// fifty megabytes, and takes memory for the edit and not for the text. The text is read through
/// [`snapshot`](Self::snapshot).
#[derive(Default)]
pub struct TextBuffer {
    text: TextSnapshot,
    history: History,
    version: u64,
}

impl TextBuffer {
    /// An empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text`, copied once.
    pub fn from_text(text: &str) -> Self {
        Self { text: TextSnapshot::from_text(text), ..Self::default() }
    }

    /// The text now, as a snapshot that stays as it is.
    #[inline]
    pub fn snapshot(&self) -> TextSnapshot {
        self.text.clone()
    }

    /// The text now, borrowed.
    #[inline]
    pub fn text(&self) -> &TextSnapshot {
        &self.text
    }

    /// How many edits (undos and redos included) have been made: the revision of the text.
    #[inline]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Replaces `range` with `text`, as a new step of the history.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> EditDelta {
        let delta = self.apply(range, text);
        self.history.record(&delta, false);
        delta
    }

    /// Like [`replace`](Self::replace), but joins the previous step of the history when this edit
    /// carries it on: typing a character right after the one typed before. What counts as carrying
    /// on is decided here, so that a controller only says it would like the edits grouped.
    pub fn replace_continuing(&mut self, range: Range<usize>, text: &str) -> EditDelta {
        let delta = self.apply(range, text);
        self.history.record(&delta, true);
        delta
    }

    /// Takes back the latest step of the history, if there is one.
    pub fn undo(&mut self) -> Option<EditDelta> {
        let step = self.history.pop_undo()?;
        let delta = self.apply_snapshot(step.start..step.start + step.inserted.len(), &step.removed);
        self.history.push_redo(step);
        Some(delta)
    }

    /// Does again the step most recently undone, if there is one.
    pub fn redo(&mut self) -> Option<EditDelta> {
        let step = self.history.pop_redo()?;
        let delta = self.apply_snapshot(step.start..step.start + step.removed.len(), &step.inserted);
        self.history.push_undo(step);
        Some(delta)
    }

    /// Whether there is a step to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is a step to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Sets how many bytes of text the history may hold at most; the oldest steps go first.
    pub fn set_history_limit(&mut self, bytes: usize) {
        self.history.set_limit(bytes);
    }

    /// Forgets every step of the history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    fn apply(&mut self, range: Range<usize>, text: &str) -> EditDelta {
        let removed = self.text.slice_snapshot(range.clone());
        replace_text(&mut self.text.rope, range.clone(), text);
        self.version += 1;
        EditDelta { range: range.clone(), removed, inserted: self.text.slice_snapshot(range.start..range.start + text.len()) }
    }

    /// The same, for text that is already chunks: undoing a paste of megabytes shares them.
    fn apply_snapshot(&mut self, range: Range<usize>, with: &TextSnapshot) -> EditDelta {
        if with.len() <= CHUNK_MAX {
            return self.apply(range, &with.to_string());
        }
        let removed = self.text.slice_snapshot(range.clone());
        splice_snapshot(&mut self.text, range.clone(), with);
        self.version += 1;
        EditDelta { range: range.clone(), removed, inserted: self.text.slice_snapshot(range.start..range.start + with.len()) }
    }
}

/// Replaces `range` of the text in `rope` with `text`: the chunks the range touches are put
/// together with the text and cut again, and the rest are left as they are.
pub(crate) fn replace_text(rope: &mut SumTree<Chunk>, range: Range<usize>, text: &str) {
    let len = rope.summary().bytes;
    assert!(range.start <= range.end && range.end <= len, "{range:?} is not in 0..{len}");
    if rope.is_empty() {
        *rope = chunks_of(text).into_iter().collect();
        return;
    }
    // The chunks the range touches: the one holding its start (the last chunk, when it starts at
    // the end) through the one holding its end.
    let first = rope.seek(&ByteAndLines { bytes: range.start, lines: 0 }, Bias::Right);
    let last = rope.seek(&ByteAndLines { bytes: range.end, lines: 0 }, Bias::Left);
    // An empty range at the join of two chunks, or at the end, holds its start in the chunk that
    // ends there, which is where the text is put.
    let first = if first.index > last.index { last.clone() } else { first };
    let (mut from, mut from_start) = (first.index, first.start.bytes);
    let mut to = last.index + 1;
    let mut to_end = last.start.bytes + rope.get(last.index).expect("a chunk holds the end").len();
    // What is left of them is joined with a neighbour when it would be a scrap.
    let kept = (range.start - from_start) + text.len() + (to_end - range.end);
    if kept < CHUNK_TARGET / 2 {
        if from > 0 {
            from -= 1;
            from_start -= rope.get(from).expect("the chunk before").len();
        } else if to < rope.len() {
            to_end += rope.get(to).expect("the chunk after").len();
            to += 1;
        }
    }

    let mut joined = String::with_capacity(kept + CHUNK_TARGET);
    let mut at = from_start;
    let mut inserted = false;
    for chunk in rope.iter_from(from).take(to - from) {
        let (start, end) = (at, at + chunk.len());
        if start < range.start {
            joined.push_str(&chunk[..range.start.min(end) - start]);
        }
        if !inserted && end >= range.start {
            joined.push_str(text);
            inserted = true;
        }
        if end > range.end {
            joined.push_str(&chunk[range.end.max(start) - start..]);
        }
        at = end;
    }
    debug_assert!(inserted && at == to_end);
    rope.replace_range(from..to, chunks_of(&joined));
}

/// Replaces `range` of `text` with the text of `with`, sharing the chunks of `with`.
pub(crate) fn splice_snapshot(text: &mut TextSnapshot, range: Range<usize>, with: &TextSnapshot) {
    let head = text.slice_snapshot(0..range.start);
    let tail = text.slice_snapshot(range.end..text.len());
    let mut rope = head.rope;
    rope.append(with.rope.clone());
    rope.append(tail.rope);
    text.rope = rope;
}
