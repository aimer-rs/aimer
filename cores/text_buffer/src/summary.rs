use std::sync::Arc;

use aimer_simd::kernels::{count_byte, find_byte};
use aimer_sum_tree::{Dimension, Item, Summary};

/// What is known of a run of text without reading it.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct TextSummary {
    pub(crate) bytes: usize,
    /// Line feeds.
    pub(crate) lines: usize,
    /// Whether any byte is not ASCII.
    pub(crate) non_ascii: bool,
    /// Whether there is a carriage return.
    pub(crate) carriage_return: bool,
}

impl Summary for TextSummary {
    fn add(&mut self, other: &Self) {
        self.bytes += other.bytes;
        self.lines += other.lines;
        self.non_ascii |= other.non_ascii;
        self.carriage_return |= other.carriage_return;
    }
}

/// Text that chunks of a rope can be slices of, so that a text that already exists in memory
/// (a paste, a file read whole) becomes a rope without being copied.
///
/// The backing must return the same text every time and must not change while a chunk holds it.
pub type SharedText = Arc<dyn AsRef<str> + Send + Sync>;

/// A piece of the text, cut on a character boundary: its own allocation, or a slice of a text
/// that is held by many chunks.
#[derive(Clone)]
pub(crate) enum Chunk {
    Owned(Arc<str>),
    Shared { backing: SharedText, start: usize, len: usize },
}

impl Chunk {
    /// A new chunk holding a copy of `text`.
    #[inline]
    pub(crate) fn copy_of(text: &str) -> Self {
        Self::Owned(Arc::from(text))
    }

    /// The chunk that is `range` of `backing`, with no copy. `range` is on character boundaries.
    #[inline]
    pub(crate) fn slice_of(backing: &SharedText, range: std::ops::Range<usize>) -> Self {
        Self::Shared { backing: Arc::clone(backing), start: range.start, len: range.len() }
    }

    #[inline]
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Owned(text) => text,
            // The range was checked when the chunk was made, and the backing does not change.
            Self::Shared { backing, start, len } => &(**backing).as_ref()[*start..*start + *len],
        }
    }

    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Owned(text) => text.len(),
            Self::Shared { len, .. } => *len,
        }
    }

    /// Whether the two chunks are the same piece of the same allocation, which is what a snapshot
    /// made from another by an edit has of every chunk the edit left alone.
    #[inline]
    pub(crate) fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Owned(a), Self::Owned(b)) => Arc::ptr_eq(a, b),
            (Self::Shared { backing: a, start: a_start, len: a_len }, Self::Shared { backing: b, start: b_start, len: b_len }) => {
                a_start == b_start && a_len == b_len && std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
            }
            _ => false,
        }
    }
}

impl std::fmt::Debug for Chunk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Chunk({:?})", self.as_str())
    }
}

impl std::ops::Deref for Chunk {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl Item for Chunk {
    type Summary = TextSummary;

    fn summary(&self) -> TextSummary {
        let text = self.as_str();
        let bytes = text.as_bytes();
        TextSummary {
            bytes: bytes.len(),
            lines: count_byte(bytes, b'\n'),
            non_ascii: !text.is_ascii(),
            carriage_return: find_byte(bytes, b'\r').is_some(),
        }
    }
}

/// A position in bytes, remembering the line feeds before it, for a search by byte.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct ByteAndLines {
    pub(crate) bytes: usize,
    pub(crate) lines: usize,
}

impl PartialOrd for ByteAndLines {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ByteAndLines {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bytes.cmp(&other.bytes)
    }
}

impl Dimension<TextSummary> for ByteAndLines {
    fn add_summary(&mut self, summary: &TextSummary) {
        self.bytes += summary.bytes;
        self.lines += summary.lines;
    }
}

/// A position in line feeds, remembering the bytes before it, for a search by line.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub(crate) struct LinesAndBytes {
    pub(crate) lines: usize,
    pub(crate) bytes: usize,
}

impl PartialOrd for LinesAndBytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LinesAndBytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.lines.cmp(&other.lines)
    }
}

impl Dimension<TextSummary> for LinesAndBytes {
    fn add_summary(&mut self, summary: &TextSummary) {
        self.lines += summary.lines;
        self.bytes += summary.bytes;
    }
}
