#![doc = include_str!("../README.md")]

mod anchors;
mod buffer;
mod graphemes;
mod history;
mod layout_cuts;
mod layout_index;
mod layout_units;
mod rope;
mod summary;

#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod tests;

pub use anchors::{AnchorId, Anchors};
pub use aimer_sum_tree::Bias;
pub use buffer::{EditDelta, TextBuffer};
pub use layout_index::{LayoutIndex, LayoutMetrics, Reindexed};
pub use rope::{ByteChunks, Chunks, TextSnapshot};
pub use summary::SharedText;

/// The size chunks are cut to when text is put into the tree: about a page of text, small enough
/// that an edit copies little and large enough that the tree is shallow. Tests use a tiny size
/// so that a short text already has many chunks, and every boundary case comes up.
///
/// A little under 8 KiB on purpose: a chunk is an `Arc<str>`, whose allocation is the text and a
/// 16 byte header, and exactly 8 KiB of text makes that one byte class over 8 KiB, which the
/// allocator rounds up: a 53 MB text took 69 MB, and 53 MB with this.
pub(crate) const CHUNK_TARGET: usize = if cfg!(test) { 16 } else { 8 * 1024 - 64 };

/// The most bytes a chunk grows to when edits are merged into it.
pub(crate) const CHUNK_MAX: usize = 2 * CHUNK_TARGET;

/// A hard line longer than this is cut into segments, each laid out as a paragraph of its own: a
/// line of a few thousand characters is laid out whole, and one of fifty million cannot be.
pub(crate) const LINE_LIMIT: usize = if cfg!(test) { 64 } else { 32 * 1024 };

/// A segment of a long line is at least this long, and at most [`SEGMENT_MAX`].
pub(crate) const SEGMENT_MIN: usize = if cfg!(test) { 16 } else { 8 * 1024 };

/// The most bytes a segment of a long line has: it is cut at the latest here.
pub(crate) const SEGMENT_MAX: usize = if cfg!(test) { 32 } else { 16 * 1024 };
