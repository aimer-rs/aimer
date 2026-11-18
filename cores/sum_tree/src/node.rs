//! The nodes of the tree, and putting trees together and taking them apart.
//!
//! Every leaf is at the same depth. A node holds at most [`MAX`] items or children and may hold
//! fewer: edits keep the shape valid, and neighbours that fit together are merged when two trees
//! are joined, but a node is not rebalanced to a minimum fill.

use std::sync::Arc;

use crate::{Item, Summary};

/// The most items a leaf, or children an internal node, holds.
pub(crate) const MAX: usize = 16;

pub(crate) struct Node<T: Item> {
    /// 0 for a leaf.
    pub(crate) height: u8,
    /// How many items are at or below this node.
    pub(crate) count: usize,
    pub(crate) summary: T::Summary,
    pub(crate) kind: Kind<T>,
}

/// An item and its summary, worked out once when the item is put in the tree: a search reads the
/// summary of many items on its way down, and an item can be expensive to summarize (a chunk of
/// text is counted by what is in it).
pub(crate) struct Entry<T: Item> {
    pub(crate) item: T,
    pub(crate) summary: T::Summary,
}

impl<T: Item> Clone for Entry<T> {
    fn clone(&self) -> Self {
        Self { item: self.item.clone(), summary: self.summary.clone() }
    }
}

impl<T: Item> Entry<T> {
    pub(crate) fn new(item: T) -> Self {
        let summary = item.summary();
        Self { item, summary }
    }
}

pub(crate) enum Kind<T: Item> {
    Leaf(Vec<Entry<T>>),
    Internal(Vec<Arc<Node<T>>>),
}

impl<T: Item> Clone for Node<T> {
    fn clone(&self) -> Self {
        Self {
            height: self.height,
            count: self.count,
            summary: self.summary.clone(),
            kind: match &self.kind {
                Kind::Leaf(items) => Kind::Leaf(items.clone()),
                Kind::Internal(children) => Kind::Internal(children.clone()),
            },
        }
    }
}

impl<T: Item> Node<T> {
    pub(crate) fn empty() -> Arc<Self> {
        Arc::new(Self::leaf(Vec::new()))
    }

    pub(crate) fn leaf(items: Vec<T>) -> Self {
        Self::leaf_of(items.into_iter().map(Entry::new).collect())
    }

    /// A leaf of entries whose summaries are already worked out.
    pub(crate) fn leaf_of(entries: Vec<Entry<T>>) -> Self {
        let mut node = Self { height: 0, count: 0, summary: T::Summary::default(), kind: Kind::Leaf(entries) };
        node.recompute();
        node
    }

    pub(crate) fn internal(children: Vec<Arc<Node<T>>>) -> Self {
        let height = children.first().map_or(1, |child| child.height + 1);
        let mut node = Self { height, count: 0, summary: T::Summary::default(), kind: Kind::Internal(children) };
        node.recompute();
        node
    }

    /// Works `count` and `summary` out again from what the node holds.
    pub(crate) fn recompute(&mut self) {
        let mut summary = T::Summary::default();
        match &self.kind {
            Kind::Leaf(entries) => {
                self.count = entries.len();
                for entry in entries {
                    summary.add(&entry.summary);
                }
            }
            Kind::Internal(children) => {
                self.count = children.iter().map(|child| child.count).sum();
                for child in children {
                    summary.add(&child.summary);
                }
            }
        }
        self.summary = summary;
    }
}

/// A node, or two when the result did not fit in one, both of the same height.
type Joined<T> = (Arc<Node<T>>, Option<Arc<Node<T>>>);

/// One internal node for `children`, or two halves of them when there are more than fit.
fn internal_nodes<T: Item>(mut children: Vec<Arc<Node<T>>>) -> Joined<T> {
    if children.len() <= MAX {
        return (Arc::new(Node::internal(children)), None);
    }
    let second = children.split_off(children.len() / 2);
    (Arc::new(Node::internal(children)), Some(Arc::new(Node::internal(second))))
}

fn leaf_nodes<T: Item>(mut entries: Vec<Entry<T>>) -> Joined<T> {
    if entries.len() <= MAX {
        return (Arc::new(Node::leaf_of(entries)), None);
    }
    let second = entries.split_off(entries.len() / 2);
    (Arc::new(Node::leaf_of(entries)), Some(Arc::new(Node::leaf_of(second))))
}

/// The trees `left` then `right` as one tree, at the height of the taller. Joined: one node
/// when everything fits, otherwise two for the caller to put under a new root.
pub(crate) fn concat<T: Item>(left: Arc<Node<T>>, right: Arc<Node<T>>) -> Joined<T> {
    if left.count == 0 {
        return (right, None);
    }
    if right.count == 0 {
        return (left, None);
    }
    match left.height.cmp(&right.height) {
        std::cmp::Ordering::Equal => match (&left.kind, &right.kind) {
            (Kind::Leaf(a), Kind::Leaf(b)) => leaf_nodes(a.iter().chain(b).cloned().collect()),
            (Kind::Internal(a), Kind::Internal(b)) => internal_nodes(a.iter().chain(b).cloned().collect()),
            _ => unreachable!("nodes of one height are of one kind"),
        },
        std::cmp::Ordering::Greater => {
            let Kind::Internal(children) = &left.kind else { unreachable!("a taller node is internal") };
            let (last, before) = children.split_last().expect("an internal node has children");
            let (joined, overflow) = concat(Arc::clone(last), right);
            let mut children: Vec<_> = before.to_vec();
            children.push(joined);
            children.extend(overflow);
            internal_nodes(children)
        }
        std::cmp::Ordering::Less => {
            let Kind::Internal(children) = &right.kind else { unreachable!("a taller node is internal") };
            let (first, after) = children.split_first().expect("an internal node has children");
            let (joined, overflow) = concat(left, Arc::clone(first));
            let mut children = vec![joined];
            children.extend(overflow);
            children.extend(after.iter().cloned());
            internal_nodes(children)
        }
    }
}

/// `left` then `right` as one root: two nodes from [`concat`] go under a new one.
pub(crate) fn join<T: Item>(left: Arc<Node<T>>, right: Arc<Node<T>>) -> Arc<Node<T>> {
    match concat(left, right) {
        (node, None) => node,
        (first, Some(second)) => Arc::new(Node::internal(vec![first, second])),
    }
}

/// An internal node with one child is that child.
pub(crate) fn collapse<T: Item>(mut node: Arc<Node<T>>) -> Arc<Node<T>> {
    loop {
        let only = match &node.kind {
            Kind::Internal(children) if children.len() == 1 => Arc::clone(&children[0]),
            _ => return node,
        };
        node = only;
    }
}

/// The first `index` items of `node` and the rest, each a tree of its own.
///
/// The two halves are put together from the pieces on either side of the cut with [`join`], not
/// by wrapping them in nodes of the old height: a half that is a single piece is that piece, so
/// a cut near an end does not leave a spine of nodes with one child each, which edits after edits
/// would pile up into a tall, thin tree.
pub(crate) fn split_at<T: Item>(node: &Arc<Node<T>>, index: usize) -> (Arc<Node<T>>, Arc<Node<T>>) {
    if index == 0 {
        return (Node::empty(), Arc::clone(node));
    }
    if index >= node.count {
        return (Arc::clone(node), Node::empty());
    }
    match &node.kind {
        Kind::Leaf(entries) => (
            Arc::new(Node::leaf_of(entries[..index].to_vec())),
            Arc::new(Node::leaf_of(entries[index..].to_vec())),
        ),
        Kind::Internal(children) => {
            let mut before = 0;
            for (position, child) in children.iter().enumerate() {
                if before + child.count > index {
                    let (inner_left, inner_right) = split_at(child, index - before);
                    // The children on each side of the one that was cut, as a node of their own.
                    let siblings = |range: std::ops::Range<usize>| {
                        let group = &children[range];
                        match group {
                            [] => Node::empty(),
                            [only] => Arc::clone(only),
                            many => Arc::new(Node::internal(many.to_vec())),
                        }
                    };
                    let left = join(siblings(0..position), inner_left);
                    let right = join(inner_right, siblings(position + 1..children.len()));
                    return (left, right);
                }
                before += child.count;
            }
            unreachable!("index {index} is below the count {}", node.count)
        }
    }
}

/// A tree of `items`, built bottom up: leaves of [`MAX`] items, then parents of [`MAX`] nodes.
/// Items are moved into the leaves, not copied, so building a tree of millions holds them once.
pub(crate) fn build<T: Item>(items: Vec<T>) -> Arc<Node<T>> {
    if items.is_empty() {
        return Node::empty();
    }
    let mut entries = items.into_iter().map(Entry::new);
    let mut level: Vec<Arc<Node<T>>> = Vec::new();
    loop {
        let leaf: Vec<Entry<T>> = entries.by_ref().take(MAX).collect();
        if leaf.is_empty() {
            break;
        }
        level.push(Arc::new(Node::leaf_of(leaf)));
    }
    while level.len() > 1 {
        let mut parents = Vec::with_capacity(level.len() / MAX + 1);
        let mut nodes = level.into_iter();
        loop {
            let group: Vec<Arc<Node<T>>> = nodes.by_ref().take(MAX).collect();
            if group.is_empty() {
                break;
            }
            parents.push(Arc::new(Node::internal(group)));
        }
        level = parents;
    }
    level.pop().expect("one node is left")
}
