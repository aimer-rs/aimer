use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::cursor::{Items, Position, seek};
use crate::node::{Kind, Node, build, collapse, join, split_at};
use crate::{Bias, Dimension, Item};

/// A sequence of items in a B+tree whose nodes summarize what is below them.
///
/// Searching by any [`Dimension`] of the summaries, taking a range, and replacing a range are
/// O(log n). The nodes are shared: [`clone`](Clone::clone) is O(1), and an edit copies the nodes
/// on one path and leaves every other clone of the tree exactly as it was.
///
/// # Examples
///
/// ```
/// use aimer_sum_tree::{Item, SumTree, Summary};
///
/// #[derive(Clone, Default, Debug, PartialEq)]
/// struct Total(u64);
/// impl Summary for Total {
///     fn add(&mut self, other: &Self) { self.0 += other.0; }
/// }
/// #[derive(Clone)]
/// struct Number(u64);
/// impl Item for Number {
///     type Summary = Total;
///     fn summary(&self) -> Total { Total(self.0) }
/// }
///
/// let mut tree: SumTree<Number> = (1..=4).map(Number).collect();
/// let before = tree.clone();
/// tree.replace_range(1..3, [Number(10)]);
/// assert_eq!(tree.summary(), &Total(1 + 10 + 4));
/// assert_eq!(before.summary(), &Total(10), "the clone did not change");
/// ```
pub struct SumTree<T: Item> {
    root: Arc<Node<T>>,
}

impl<T: Item> Clone for SumTree<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self { root: Arc::clone(&self.root) }
    }
}

impl<T: Item> Default for SumTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Item> fmt::Debug for SumTree<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SumTree").field("len", &self.len()).field("height", &self.root.height).finish()
    }
}

impl<T: Item> FromIterator<T> for SumTree<T> {
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        Self { root: build(items.into_iter().collect()) }
    }
}

impl<T: Item> SumTree<T> {
    /// An empty tree.
    pub fn new() -> Self {
        Self { root: Node::empty() }
    }

    /// How many items the tree holds.
    #[inline]
    pub fn len(&self) -> usize {
        self.root.count
    }

    /// Whether the tree holds no items.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.root.count == 0
    }

    /// Whether `other` is this very tree: the same nodes, which is so of a clone and of nothing
    /// else. Cheap, and false for a tree equal to this one but made apart.
    #[inline]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.root, &other.root)
    }

    /// The summary of every item.
    #[inline]
    pub fn summary(&self) -> &T::Summary {
        &self.root.summary
    }

    /// The item at `index`, if there is one.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len() {
            return None;
        }
        let mut node = &*self.root;
        let mut index = index;
        loop {
            match &node.kind {
                Kind::Leaf(entries) => return entries.get(index).map(|entry| &entry.item),
                Kind::Internal(children) => {
                    let mut chosen = None;
                    for child in children {
                        if index < child.count {
                            chosen = Some(child);
                            break;
                        }
                        index -= child.count;
                    }
                    node = chosen?;
                }
            }
        }
    }

    /// Puts `other` after this tree's items. O(log n) in the sizes of both.
    pub fn append(&mut self, other: Self) {
        self.root = join(Arc::clone(&self.root), other.root);
    }

    /// Puts one item after the last.
    pub fn push(&mut self, item: T) {
        self.append(std::iter::once(item).collect());
    }

    /// Puts the items of `items` after the last.
    pub fn extend(&mut self, items: impl IntoIterator<Item = T>) {
        self.append(items.into_iter().collect());
    }

    /// Splits into the first `index` items and the rest. Both share their nodes with this tree
    /// and with each other where they can.
    pub fn split_at(&self, index: usize) -> (Self, Self) {
        let (left, right) = split_at(&self.root, index.min(self.len()));
        (Self { root: collapse(left) }, Self { root: collapse(right) })
    }

    /// The items in `range`, as a tree.
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(range.start <= range.end && range.end <= self.len(), "{range:?} is not in 0..{}", self.len());
        let (_, rest) = self.split_at(range.start);
        let (middle, _) = rest.split_at(range.end - range.start);
        middle
    }

    /// Replaces the items in `range` with `items`: O(log n + the number of new items).
    ///
    /// # Panics
    ///
    /// Panics if `range` is reversed or past the end.
    pub fn replace_range(&mut self, range: Range<usize>, items: impl IntoIterator<Item = T>) {
        assert!(range.start <= range.end && range.end <= self.len(), "{range:?} is not in 0..{}", self.len());
        let (head, rest) = self.split_at(range.start);
        let (_, tail) = rest.split_at(range.end - range.start);
        let mut edited = head;
        edited.extend(items);
        edited.append(tail);
        *self = edited;
    }

    /// Changes the item at `index` with `change`, copying only the nodes on its path (and not
    /// those at all when nothing else shares them). Returns whether there is such an item.
    pub fn update(&mut self, index: usize, change: impl FnOnce(&mut T)) -> bool {
        if index >= self.len() {
            return false;
        }
        update_node(&mut self.root, index, change);
        true
    }

    /// The items in order.
    pub fn iter(&self) -> Items<'_, T> {
        self.iter_from(0)
    }

    /// The items from `index` on, in order.
    pub fn iter_from(&self, index: usize) -> Items<'_, T> {
        Items::new(&self.root, index)
    }

    /// The first item whose end, in dimension `D`, is at or past `target` (see [`Bias`]), with
    /// the dimension where it starts. Past the end this is the length and the total.
    pub fn seek<D: Dimension<T::Summary>>(&self, target: &D, bias: Bias) -> Position<D> {
        seek(&self.root, target, bias)
    }

    /// The dimension `D` accumulated over the first `index` items: where item `index` starts.
    /// The whole total when `index` is the length or more. O(log n).
    pub fn dimension_before<D: Dimension<T::Summary>>(&self, index: usize) -> D {
        let mut total = D::default();
        let mut node: &Node<T> = &self.root;
        let mut index = index.min(self.len());
        loop {
            match &node.kind {
                Kind::Leaf(entries) => {
                    for entry in &entries[..index] {
                        total.add_summary(&entry.summary);
                    }
                    return total;
                }
                Kind::Internal(children) => {
                    let mut next = None;
                    for child in children {
                        if index >= child.count {
                            total.add_summary(&child.summary);
                            index -= child.count;
                        } else {
                            next = Some(child);
                            break;
                        }
                    }
                    match next {
                        Some(child) => node = child,
                        None => return total,
                    }
                }
            }
        }
    }

    /// How deep the tree is: 0 for a single leaf.
    pub fn height(&self) -> usize {
        usize::from(self.root.height)
    }

    /// Panics unless every node of the tree holds what it says: counts, summaries, equal depth
    /// of every leaf, no empty children, no node over capacity. For tests.
    #[doc(hidden)]
    pub fn check_invariants(&self)
    where
        T::Summary: PartialEq + fmt::Debug,
    {
        fn check<T: Item>(node: &Node<T>, root: bool)
        where
            T::Summary: PartialEq + fmt::Debug,
        {
            let mut again = Node::<T> { height: node.height, count: 0, summary: Default::default(), kind: node.kind.clone_shallow() };
            again.recompute();
            assert_eq!(again.count, node.count, "count");
            assert_eq!(again.summary, node.summary, "summary");
            if let Kind::Internal(children) = &node.kind {
                assert!(!children.is_empty() && children.len() <= crate::node::MAX, "{} children", children.len());
                for child in children {
                    assert_eq!(child.height + 1, node.height, "a child is one level down");
                    assert!(child.count > 0, "a child is empty");
                    check(child, false);
                }
            } else if let Kind::Leaf(entries) = &node.kind {
                assert!(entries.len() <= crate::node::MAX, "{} items", entries.len());
                assert!(root || !entries.is_empty(), "an empty leaf");
                for entry in entries {
                    assert_eq!(entry.summary, entry.item.summary(), "an entry's summary");
                }
            }
        }
        check(&self.root, true);
    }
}

impl<T: Item> Kind<T> {
    fn clone_shallow(&self) -> Self {
        match self {
            Kind::Leaf(entries) => Kind::Leaf(entries.clone()),
            Kind::Internal(children) => Kind::Internal(children.clone()),
        }
    }
}

fn update_node<T: Item>(node: &mut Arc<Node<T>>, index: usize, change: impl FnOnce(&mut T)) {
    let node = Arc::make_mut(node);
    match &mut node.kind {
        Kind::Leaf(entries) => {
            let entry = &mut entries[index];
            change(&mut entry.item);
            entry.summary = entry.item.summary();
        }
        Kind::Internal(children) => {
            let mut index = index;
            for child in children.iter_mut() {
                if index < child.count {
                    update_node(child, index, change);
                    break;
                }
                index -= child.count;
            }
        }
    }
    node.recompute();
}

impl<'a, T: Item> IntoIterator for &'a SumTree<T> {
    type Item = &'a T;
    type IntoIter = Items<'a, T>;

    fn into_iter(self) -> Items<'a, T> {
        self.iter()
    }
}
