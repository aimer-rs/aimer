use std::slice;
use std::sync::Arc;

use crate::node::{Entry, Kind, Node};
use crate::{Bias, Dimension, Item};

/// Where a search landed: the item and the dimension where that item starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position<D> {
    /// The index of the item; the length of the tree when the target is past every item.
    pub index: usize,
    /// The dimension accumulated over the items before `index`.
    pub start: D,
}

pub(crate) fn seek<T: Item, D: Dimension<T::Summary>>(root: &Arc<Node<T>>, target: &D, bias: Bias) -> Position<D> {
    let mut node: &Node<T> = root;
    let mut start = D::default();
    let mut index = 0;
    'descend: loop {
        match &node.kind {
            Kind::Leaf(entries) => {
                for entry in entries {
                    let mut end = start.clone();
                    end.add_summary(&entry.summary);
                    if bias.reaches(&end, target) {
                        return Position { index, start };
                    }
                    start = end;
                    index += 1;
                }
                return Position { index, start };
            }
            Kind::Internal(children) => {
                for child in children {
                    let mut end = start.clone();
                    end.add_summary(&child.summary);
                    if bias.reaches(&end, target) {
                        node = child;
                        continue 'descend;
                    }
                    start = end;
                    index += child.count;
                }
                return Position { index, start };
            }
        }
    }
}

/// The items of a [`SumTree`] in order, from a starting index.
pub struct Items<'a, T: Item> {
    /// The nodes still to visit at each level above the current leaf: the rest of each parent's
    /// children.
    stack: Vec<slice::Iter<'a, Arc<Node<T>>>>,
    leaf: slice::Iter<'a, Entry<T>>,
}

impl<'a, T: Item> Items<'a, T> {
    pub(crate) fn new(root: &'a Arc<Node<T>>, index: usize) -> Self {
        let mut items = Self { stack: Vec::new(), leaf: [].iter() };
        if index >= root.count {
            return items;
        }
        let mut node: &'a Node<T> = root;
        let mut index = index;
        loop {
            match &node.kind {
                Kind::Leaf(leaf) => {
                    items.leaf = leaf[index..].iter();
                    return items;
                }
                Kind::Internal(children) => {
                    let mut siblings = children.iter();
                    let mut chosen = None;
                    for child in siblings.by_ref() {
                        if index < child.count {
                            chosen = Some(child);
                            break;
                        }
                        index -= child.count;
                    }
                    items.stack.push(siblings);
                    node = chosen.expect("the index is below the count");
                }
            }
        }
    }

    /// Moves to the leftmost leaf at or below `node`.
    fn descend(&mut self, mut node: &'a Node<T>) {
        loop {
            match &node.kind {
                Kind::Leaf(leaf) => {
                    self.leaf = leaf.iter();
                    return;
                }
                Kind::Internal(children) => {
                    let mut siblings = children.iter();
                    node = siblings.next().expect("an internal node has children");
                    self.stack.push(siblings);
                }
            }
        }
    }
}

impl<'a, T: Item> Iterator for Items<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            if let Some(entry) = self.leaf.next() {
                return Some(&entry.item);
            }
            let next = loop {
                let siblings = self.stack.last_mut()?;
                match siblings.next() {
                    Some(node) => break node,
                    None => {
                        self.stack.pop();
                    }
                }
            };
            self.descend(next);
        }
    }
}
