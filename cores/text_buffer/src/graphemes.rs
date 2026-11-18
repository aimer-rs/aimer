//! Grapheme boundaries of a text held in chunks.
//!
//! A cluster can run across the join of two chunks (a letter and the accent in the next chunk), so
//! the boundary is decided by feeding the segmenter the chunks around the position, as it asks for
//! them, and never by cutting a window out of the text. The answer is exactly the one a walk over
//! the whole text gives. Between two ASCII bytes there is a boundary (but for a carriage return
//! before a line feed) and the segmenter is not asked at all.

use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::rope::TextSnapshot;

impl TextSnapshot {
    /// Whether there is a grapheme boundary at `byte`: the start and the end of the text are
    /// boundaries.
    ///
    /// # Panics
    ///
    /// Panics if `byte` is past the end or not on a character boundary.
    pub fn is_grapheme_boundary(&self, byte: usize) -> bool {
        assert!(self.is_char_boundary(byte), "{byte} is not on a character boundary of a text of {}", self.len());
        if byte == 0 || byte == self.len() {
            return true;
        }
        if self.plain_boundary(byte) {
            return true;
        }
        let mut cursor = GraphemeCursor::new(byte, self.len(), true);
        let (mut chunk, mut chunk_start) = self.chunk_containing(byte).expect("the byte is inside the text");
        loop {
            match cursor.is_boundary(chunk, chunk_start) {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(&mut cursor, end),
                Err(GraphemeIncomplete::NextChunk) => (chunk, chunk_start) = self.chunk_after(chunk, chunk_start),
                Err(other) => unreachable!("{other:?}"),
            }
        }
    }

    /// The first boundary after `byte`: the end of the text at or after the last cluster.
    pub fn next_grapheme_boundary(&self, byte: usize) -> usize {
        assert!(self.is_char_boundary(byte), "{byte} is not on a character boundary");
        if byte >= self.len() {
            return self.len();
        }
        if byte + 1 == self.len() || self.plain_boundary(byte + 1) && self.byte_at(byte).is_some_and(|b| b < 0x80) {
            return byte + 1;
        }
        let mut cursor = GraphemeCursor::new(byte, self.len(), true);
        let (mut chunk, mut chunk_start) = self.chunk_containing(byte).expect("the byte is inside the text");
        loop {
            match cursor.next_boundary(chunk, chunk_start) {
                Ok(Some(boundary)) => return boundary,
                Ok(None) => return self.len(),
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(&mut cursor, end),
                Err(GraphemeIncomplete::NextChunk) => (chunk, chunk_start) = self.chunk_after(chunk, chunk_start),
                Err(other) => unreachable!("{other:?}"),
            }
        }
    }

    /// The last boundary before `byte`: the start of the text at or before the first cluster.
    pub fn prev_grapheme_boundary(&self, byte: usize) -> usize {
        assert!(self.is_char_boundary(byte), "{byte} is not on a character boundary");
        if byte == 0 {
            return 0;
        }
        let byte = byte.min(self.len());
        if self.plain_boundary(byte - 1) && self.byte_at(byte - 1).is_some_and(|b| b < 0x80) {
            return byte - 1;
        }
        let mut cursor = GraphemeCursor::new(byte, self.len(), true);
        let (mut chunk, mut chunk_start) = self.chunk_ending_at(byte).expect("a byte before is inside the text");
        loop {
            match cursor.prev_boundary(chunk, chunk_start) {
                Ok(Some(boundary)) => return boundary,
                Ok(None) => return 0,
                Err(GraphemeIncomplete::PreContext(end)) => self.provide_context(&mut cursor, end),
                Err(GraphemeIncomplete::PrevChunk) => {
                    (chunk, chunk_start) = self.chunk_ending_at(chunk_start).expect("there is a chunk before");
                }
                Err(other) => unreachable!("{other:?}"),
            }
        }
    }

    /// The greatest boundary at or before `byte`.
    pub fn floor_grapheme_boundary(&self, byte: usize) -> usize {
        let byte = byte.min(self.len());
        let mut byte = byte;
        while !self.is_char_boundary(byte) {
            byte -= 1;
        }
        if self.is_grapheme_boundary(byte) { byte } else { self.prev_grapheme_boundary(byte) }
    }

    /// The least boundary at or after `byte`.
    pub fn ceil_grapheme_boundary(&self, byte: usize) -> usize {
        let byte = byte.min(self.len());
        let mut byte = byte;
        while !self.is_char_boundary(byte) {
            byte += 1;
        }
        if self.is_grapheme_boundary(byte) { byte } else { self.next_grapheme_boundary(byte) }
    }

    /// Whether the bytes either side of `byte` settle it on their own: both ASCII and not a
    /// carriage return before a line feed.
    #[inline]
    fn plain_boundary(&self, byte: usize) -> bool {
        if byte == 0 {
            return false;
        }
        match (self.byte_at(byte - 1), self.byte_at(byte)) {
            (Some(before), Some(after)) => before < 0x80 && after < 0x80 && !(before == b'\r' && after == b'\n'),
            _ => false,
        }
    }

    fn provide_context(&self, cursor: &mut GraphemeCursor, end: usize) {
        let (chunk, start) = self.chunk_ending_at(end).expect("the segmenter asks only for text that exists");
        cursor.provide_context(chunk, start);
    }

    fn chunk_after(&self, chunk: &str, chunk_start: usize) -> (&str, usize) {
        self.chunk_containing(chunk_start + chunk.len()).expect("the segmenter asks only for text that exists")
    }
}
