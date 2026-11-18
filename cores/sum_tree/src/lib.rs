#![doc = include_str!("../README.md")]

mod cursor;
mod node;
mod tree;

#[cfg(test)]
mod tests;

use std::cmp::Ordering;

pub use cursor::{Items, Position};
pub use tree::SumTree;

/// What is known of a run of items without looking at them: a monoid, so the summary of two
/// runs side by side is `add` of their summaries.
///
/// `Default` is the summary of no items, and `add` must be associative. Typical summaries are
/// counts (bytes, lines), sums (heights) and maxima.
pub trait Summary: Clone + Default {
    /// Adds the summary of the run that follows this one.
    fn add(&mut self, other: &Self);
}

/// A value a [`SumTree`] holds, and how it summarizes itself.
///
/// Items are cloned when the tree changes shape, so they are best small or shared (an `Arc`
/// inside); a text chunk is an `Arc<str>` and a layout line a few numbers.
pub trait Item: Clone {
    /// The summary of one item.
    type Summary: Summary;

    /// Summarizes this item alone.
    fn summary(&self) -> Self::Summary;
}

/// One number worked out from summaries, which the tree can be searched by.
///
/// A dimension accumulates over the summaries of the items before a position (bytes, lines,
/// pixels) and must grow monotonically with them: adding a summary never makes it smaller.
pub trait Dimension<S: Summary>: Clone + Default + Ord {
    /// Adds what `summary` contributes to this dimension.
    fn add_summary(&mut self, summary: &S);
}

/// Which item a search picks when the target is exactly where one item ends and the next
/// begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bias {
    /// The item that ends at the target.
    Left,
    /// The item that starts at the target.
    Right,
}

impl Bias {
    /// Whether an item whose dimension ends at `end` is at or past `target`.
    #[inline]
    pub(crate) fn reaches<D: Ord>(self, end: &D, target: &D) -> bool {
        match (self, end.cmp(target)) {
            (_, Ordering::Greater) => true,
            (Bias::Left, Ordering::Equal) => true,
            _ => false,
        }
    }
}
