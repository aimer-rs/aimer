use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use aimer_simd::kernels::{count_byte, find_byte};
use aimer_sum_tree::{Bias, Items, SumTree};

use crate::summary::{ByteAndLines, Chunk, LinesAndBytes, SharedText};
use crate::CHUNK_TARGET;

/// An immutable view of the text at one moment.
///
/// Cloning is O(1) and a snapshot never changes, whatever is done to the buffer it came from, so
/// it can be handed to another thread, kept for undo, or read for a whole frame while the next
/// key is being typed. Reads cost what they touch: a position is found in O(log n).
#[derive(Clone, Default)]
pub struct TextSnapshot {
    pub(crate) rope: SumTree<Chunk>,
}

impl std::fmt::Debug for TextSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TextSnapshot").field("len", &self.len()).field("lines", &self.line_count()).finish()
    }
}

/// `text` cut into chunks of about [`CHUNK_TARGET`] bytes, each on a character boundary and none
/// between a carriage return and the line feed after it.
pub(crate) fn chunks_of(text: &str) -> Vec<Chunk> {
    chunk_ranges(text).map(|range| Chunk::copy_of(&text[range])).collect()
}

/// Where `text` is cut into chunks, as the ranges of the chunks.
fn chunk_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let bytes = text.as_bytes();
    let mut at = 0;
    std::iter::from_fn(move || {
        if at >= text.len() {
            return None;
        }
        let mut end = (at + CHUNK_TARGET).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len() && bytes[end - 1] == b'\r' && bytes[end] == b'\n' {
            end += 1;
        }
        let range = at..end;
        at = end;
        Some(range)
    })
}

impl TextSnapshot {
    /// An empty text.
    pub fn new() -> Self {
        Self::default()
    }

    /// The text of `text`, copied once into chunks.
    pub fn from_text(text: &str) -> Self {
        Self { rope: chunks_of(text).into_iter().collect() }
    }

    /// The text of `text`, with no copy of it: the chunks are slices of `text`, which they keep alive
    /// until the last of them is dropped or edited away (an edit copies the chunks it touches and
    /// shares the rest).
    ///
    /// This is how a paste or a file that was read whole becomes a text of megabytes at the cost of
    /// the chunk list, and not of the text. The backing must not change while the snapshot lives:
    /// see [`SharedText`].
    pub fn from_shared(text: SharedText) -> Self {
        let whole: &str = (*text).as_ref();
        let chunks: Vec<Chunk> = chunk_ranges(whole).map(|range| Chunk::slice_of(&text, range)).collect();
        Self { rope: chunks.into_iter().collect() }
    }

    /// This text with `range` replaced by the text of `with`, as a new snapshot that shares the
    /// chunks of `with` and every chunk the edit did not touch. A short `with` is copied in, as
    /// [`with_replaced`](Self::with_replaced) does.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn with_replaced_snapshot(&self, range: Range<usize>, with: &TextSnapshot) -> Self {
        self.check(&range);
        if with.len() <= crate::CHUNK_MAX {
            return self.with_replaced(range, &with.slice(0..with.len()));
        }
        let mut next = self.clone();
        crate::buffer::splice_snapshot(&mut next, range, with);
        next
    }

    /// Whether `other` is this very snapshot or a clone of it, which is cheaper to find out than
    /// whether the texts are equal, and false for equal texts made apart.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.rope.ptr_eq(&other.rope)
    }

    /// How many bytes the text has.
    #[inline]
    pub fn len(&self) -> usize {
        self.rope.summary().bytes
    }

    /// Whether the text has no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many lines the text has: one more than its line feeds, so an empty text has one, and
    /// a text ending in a line feed ends with an empty line.
    #[inline]
    pub fn line_count(&self) -> usize {
        self.rope.summary().lines + 1
    }

    /// Whether every byte is ASCII.
    #[inline]
    pub fn is_ascii(&self) -> bool {
        !self.rope.summary().non_ascii
    }

    /// Whether any carriage return is in the text.
    #[inline]
    pub fn has_carriage_return(&self) -> bool {
        self.rope.summary().carriage_return
    }

    /// How many chunks the text is held in.
    #[inline]
    pub fn chunk_count(&self) -> usize {
        self.rope.len()
    }

    /// The bytes of the text from `byte` to the end of the chunk that holds it, or `None` at or past
    /// the end of the text. A reader that is asked for the text a piece at a time from any offset,
    /// as a parser is, calls this and goes on from where the piece ends. `byte` need not be on a
    /// character boundary: these are bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_text_buffer::TextSnapshot;
    ///
    /// let text = TextSnapshot::from_text("hello world");
    /// let mut read = Vec::new();
    /// let mut at = 3;
    /// while let Some(piece) = text.bytes_from(at) {
    ///     read.extend_from_slice(piece);
    ///     at += piece.len();
    /// }
    /// assert_eq!(read, b"lo world");
    /// ```
    pub fn bytes_from(&self, byte: usize) -> Option<&[u8]> {
        let (chunk, start) = self.chunk_containing(byte)?;
        Some(&chunk.as_bytes()[byte - start..])
    }

    /// The chunk that holds the byte at `byte` and the byte offset where it starts, or `None`
    /// at or past the end.
    pub(crate) fn chunk_containing(&self, byte: usize) -> Option<(&str, usize)> {
        let position = self.rope.seek(&ByteAndLines { bytes: byte, lines: 0 }, Bias::Right);
        self.rope.get(position.index).map(|chunk| (chunk.as_str(), position.start.bytes))
    }

    /// The chunk that ends at or after `byte` (the one holding the byte before it), and where
    /// it starts, or `None` for the start of the text.
    pub(crate) fn chunk_ending_at(&self, byte: usize) -> Option<(&str, usize)> {
        if byte == 0 {
            return None;
        }
        let position = self.rope.seek(&ByteAndLines { bytes: byte, lines: 0 }, Bias::Left);
        self.rope.get(position.index).map(|chunk| (chunk.as_str(), position.start.bytes))
    }

    /// The byte at `byte`, or `None` at or past the end.
    pub fn byte_at(&self, byte: usize) -> Option<u8> {
        let (chunk, start) = self.chunk_containing(byte)?;
        chunk.as_bytes().get(byte - start).copied()
    }

    /// Whether `byte` is on a character boundary (the end of the text is).
    pub fn is_char_boundary(&self, byte: usize) -> bool {
        if byte >= self.len() {
            return byte == self.len();
        }
        self.byte_at(byte).is_some_and(|byte| (byte as i8) >= -0x40)
    }

    /// The text in `range`, borrowed when it lies in one chunk and copied when it does not.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn slice(&self, range: Range<usize>) -> Cow<'_, str> {
        self.check(&range);
        if range.is_empty() {
            return Cow::Borrowed("");
        }
        let (chunk, start) = self.chunk_containing(range.start).expect("the range starts inside the text");
        if range.end <= start + chunk.len() {
            return Cow::Borrowed(&chunk[range.start - start..range.end - start]);
        }
        let mut text = String::with_capacity(range.len());
        for piece in self.chunks(range) {
            text.push_str(piece);
        }
        Cow::Owned(text)
    }

    /// The pieces of the text in `range`, in order, each in one chunk.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn chunks(&self, range: Range<usize>) -> Chunks<'_> {
        self.check(&range);
        let position = self.rope.seek(&ByteAndLines { bytes: range.start, lines: 0 }, Bias::Right);
        Chunks { items: self.rope.iter_from(position.index), at: position.start.bytes, range }
    }

    /// This text with `range` replaced by `text`, as a new snapshot that shares every chunk the
    /// edit did not touch with this one. This snapshot is not changed.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn with_replaced(&self, range: Range<usize>, text: &str) -> Self {
        self.check(&range);
        let mut next = self.clone();
        crate::buffer::replace_text(&mut next.rope, range, text);
        next
    }

    /// What differs between this text and `other`: the bytes of this text and the bytes of
    /// `other` that are left when what the two start with and end with in common is taken
    /// away, or `None` if they are the same text.
    ///
    /// Snapshots made from one another by edits share the chunks the edits did not touch, so what
    /// they have in common is found by comparing chunks by identity, and the cost is that of the
    /// part that differs and not of the text. Two texts that share nothing are compared byte
    /// by byte, as far as they agree.
    pub fn diff(&self, other: &Self) -> Option<(Range<usize>, Range<usize>)> {
        // The chunks both start with, by identity.
        let mut prefix = 0;
        for (a, b) in self.rope.iter().zip(other.rope.iter()) {
            if !a.same(b) {
                break;
            }
            prefix += a.len();
        }
        // The chunks both end with.
        let (mut a_end, mut b_end) = (self.len(), other.len());
        let (mut a_chunks, mut b_chunks) = (self.rope.len(), other.rope.len());
        while a_chunks > 0 && b_chunks > 0 {
            let (a, b) = (self.rope.get(a_chunks - 1).expect("a chunk"), other.rope.get(b_chunks - 1).expect("a chunk"));
            if !a.same(b) || a_end - a.len() < prefix || b_end - b.len() < prefix {
                break;
            }
            a_end -= a.len();
            b_end -= b.len();
            a_chunks -= 1;
            b_chunks -= 1;
        }
        // Then byte by byte in what is left.
        let prefix = prefix + common_prefix(self.byte_chunks(prefix..a_end), other.byte_chunks(prefix..b_end));
        let suffix = common_suffix(self, prefix..a_end, other, prefix..b_end);
        let (a_end, b_end) = (a_end - suffix, b_end - suffix);
        if prefix == a_end && prefix == b_end {
            return None;
        }
        // A difference is a range of characters: a byte that matches inside one that does not is
        // part of that one.
        let mut start = prefix;
        while !self.is_char_boundary(start) || !other.is_char_boundary(start) {
            start -= 1;
        }
        let (mut a_end, mut b_end) = (a_end, b_end);
        while !self.is_char_boundary(a_end) || !other.is_char_boundary(b_end) {
            a_end += 1;
            b_end += 1;
        }
        Some((start..a_end, start..b_end))
    }

    /// The bytes in `range`, borrowed when they lie in one chunk and copied when they do not. Unlike
    /// [`slice`](Self::slice) the ends need not be on character boundaries.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end.
    pub fn bytes(&self, range: Range<usize>) -> Cow<'_, [u8]> {
        assert!(range.start <= range.end && range.end <= self.len(), "{range:?} is not in 0..{}", self.len());
        if range.is_empty() {
            return Cow::Borrowed(&[]);
        }
        let (chunk, start) = self.chunk_containing(range.start).expect("the range starts inside the text");
        if range.end <= start + chunk.len() {
            return Cow::Borrowed(&chunk.as_bytes()[range.start - start..range.end - start]);
        }
        let mut bytes = Vec::with_capacity(range.len());
        for piece in self.byte_chunks(range) {
            bytes.extend_from_slice(piece);
        }
        Cow::Owned(bytes)
    }

    /// The text in `range` as a snapshot of its own: the chunks it covers whole are shared with
    /// this one, and only the pieces at its ends are copied.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end, or an end is not on a character boundary.
    pub fn slice_snapshot(&self, range: Range<usize>) -> Self {
        self.check(&range);
        if range.is_empty() {
            return Self::new();
        }
        let first = self.rope.seek(&ByteAndLines { bytes: range.start, lines: 0 }, Bias::Right);
        let last = self.rope.seek(&ByteAndLines { bytes: range.end, lines: 0 }, Bias::Left);
        let first_chunk = &self.rope.get(first.index).expect("a chunk holds the start");
        let last_chunk = &self.rope.get(last.index).expect("a chunk holds the end");
        let (first_start, last_start) = (first.start.bytes, last.start.bytes);
        if first.index == last.index {
            return Self::from_text(&first_chunk[range.start - first_start..range.end - first_start]);
        }
        let mut rope: SumTree<Chunk> = chunks_of(&first_chunk[range.start - first_start..]).into_iter().collect();
        rope.append(self.rope.slice(first.index + 1..last.index));
        rope.extend(chunks_of(&last_chunk[..range.end - last_start]));
        Self { rope }
    }

    /// The line feeds before `byte`, which is the zero-based number of the line it is on.
    ///
    /// # Panics
    ///
    /// Panics if `byte` is past the end.
    pub fn byte_to_line(&self, byte: usize) -> usize {
        assert!(byte <= self.len(), "{byte} is past the end, {}", self.len());
        let position = self.rope.seek(&ByteAndLines { bytes: byte, lines: 0 }, Bias::Right);
        match self.rope.get(position.index) {
            Some(chunk) => position.start.lines + count_byte(&chunk.as_bytes()[..byte - position.start.bytes], b'\n'),
            None => self.rope.summary().lines,
        }
    }

    /// The byte where line `line` starts: the end of the text for a line past the last.
    pub fn line_to_byte(&self, line: usize) -> usize {
        if line == 0 {
            return 0;
        }
        if line > self.rope.summary().lines {
            return self.len();
        }
        // The chunk that holds the `line`-th line feed.
        let position = self.rope.seek(&LinesAndBytes { lines: line, bytes: 0 }, Bias::Left);
        let chunk = self.rope.get(position.index).expect("a chunk holds that line feed");
        let mut remaining = line - position.start.lines;
        let bytes = chunk.as_bytes();
        let mut at = 0;
        loop {
            let found = find_byte(&bytes[at..], b'\n').expect("the chunk has that many line feeds");
            at += found + 1;
            remaining -= 1;
            if remaining == 0 {
                return position.start.bytes + at;
            }
        }
    }

    /// The bytes of line `line`, with its line feed if it has one; empty past the last line.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        self.line_to_byte(line)..self.line_to_byte(line + 1)
    }

    #[track_caller]
    fn check(&self, range: &Range<usize>) {
        assert!(range.start <= range.end && range.end <= self.len(), "{range:?} is not in 0..{}", self.len());
        assert!(self.is_char_boundary(range.start) && self.is_char_boundary(range.end), "{range:?} is not on character boundaries");
    }
}

impl std::fmt::Display for TextSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for piece in self.chunks(0..self.len()) {
            formatter.write_str(piece)?;
        }
        Ok(())
    }
}

impl TextSnapshot {
    /// The bytes of `range`, piece by piece, in order, each in one chunk. Unlike
    /// [`chunks`](Self::chunks) the ends need not be on character boundaries.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end.
    pub fn byte_chunks(&self, range: Range<usize>) -> ByteChunks<'_> {
        assert!(range.start <= range.end && range.end <= self.len(), "{range:?} is not in 0..{}", self.len());
        let position = self.rope.seek(&ByteAndLines { bytes: range.start, lines: 0 }, Bias::Right);
        ByteChunks { items: self.rope.iter_from(position.index), at: position.start.bytes, range }
    }
}

/// The bytes of a range of text, piece by piece.
pub struct ByteChunks<'a> {
    items: Items<'a, Chunk>,
    at: usize,
    range: Range<usize>,
}

impl<'a> Iterator for ByteChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        while self.at < self.range.end {
            let chunk = self.items.next()?.as_bytes();
            let start = self.at;
            self.at += chunk.len();
            let from = self.range.start.saturating_sub(start);
            let to = chunk.len().min(self.range.end - start);
            if from < to {
                return Some(&chunk[from..to]);
            }
        }
        None
    }
}

/// The pieces of a range of text, in order.
pub struct Chunks<'a> {
    items: Items<'a, Chunk>,
    /// Where the next chunk starts.
    at: usize,
    range: Range<usize>,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        while self.at < self.range.end {
            let chunk = self.items.next()?.as_str();
            let start = self.at;
            self.at += chunk.len();
            let from = self.range.start.saturating_sub(start);
            let to = chunk.len().min(self.range.end - start);
            if from < to {
                return Some(&chunk[from..to]);
            }
        }
        None
    }
}

/// How many bytes two runs of pieces start with in common.
fn common_prefix<'a>(a: impl Iterator<Item = &'a [u8]>, b: impl Iterator<Item = &'a [u8]>) -> usize {
    let (mut a, mut b) = (a, b);
    let (mut left, mut right): (&[u8], &[u8]) = (&[], &[]);
    let mut common = 0;
    loop {
        if left.is_empty() {
            match a.next() {
                Some(piece) => left = piece,
                None => return common,
            }
        }
        if right.is_empty() {
            match b.next() {
                Some(piece) => right = piece,
                None => return common,
            }
        }
        let n = left.len().min(right.len());
        let same = block_common_prefix(&left[..n], &right[..n]);
        common += same;
        if same < n {
            return common;
        }
        left = &left[n..];
        right = &right[n..];
    }
}

/// How many bytes `a` and `b` start with in common, a block at a time: comparing slices is one
/// `memcmp`, where a byte loop is slow in a debug build.
fn block_common_prefix(a: &[u8], b: &[u8]) -> usize {
    const BLOCK: usize = 256;
    let limit = a.len().min(b.len());
    let mut at = 0;
    while at + BLOCK <= limit && a[at..at + BLOCK] == b[at..at + BLOCK] {
        at += BLOCK;
    }
    while at < limit && a[at] == b[at] {
        at += 1;
    }
    at
}

/// How many bytes the ends of `a` (in `a_range`) and `b` (in `b_range`) have in common, a
/// bounded piece at a time from the end.
fn common_suffix(a: &TextSnapshot, a_range: Range<usize>, b: &TextSnapshot, b_range: Range<usize>) -> usize {
    const STEP: usize = 4096;
    let limit = a_range.len().min(b_range.len());
    let mut common = 0;
    while common < limit {
        let take = STEP.min(limit - common);
        let left = a.bytes(a_range.end - common - take..a_range.end - common);
        let right = b.bytes(b_range.end - common - take..b_range.end - common);
        let same = (0..take).rev().take_while(|&index| left[index] == right[index]).count();
        common += same;
        if same < take {
            break;
        }
    }
    common
}
