//! Which subtrees hold a node that escapes its ancestors' clips.
//!
//! The covered-clip shortcut in [`world`](super::world) says a node's footprint
//! is the clip it hands down, because everything below it is clipped to that
//! clip. That holds unless some node *below* it escapes clipping. It used to be
//! decided by the tree-wide count of escaping nodes, so a single `Scalable`
//! anywhere in the window (a pressable button escapes the clip it sits in, even
//! at rest) turned the shortcut off for every scrollable and made each frame
//! walk the whole content again. A node only has to know about its own
//! descendants; escapes above it are already part of the clip it was handed.

use super::render_order::{NO_PARENT, ORPHAN_PARENT};
use super::*;

/// For each node, whether any node strictly below it escapes clipping.
///
/// Derived in one pass over the escaping nodes, each marking the chain of
/// ancestors above it, and kept until the structure or an escape changes. Both
/// are rare next to the frames that ask: structure moves on rebuilds and page
/// changes, and an escape flips when an effect starts or ends.
#[derive(Default)]
pub(super) struct EscapeIndex {
    /// The `(topology_revision, escape_revision)` `below` was derived against.
    key: Option<(u64, u64)>,
    below: Vec<bool>,
}

impl DrawCommandList {
    /// Whether any descendant of the node at `index` escapes clipping.
    ///
    /// Constant time for a tree with no escape at all, and for every query after
    /// the first one that follows a change to structure or escapes.
    pub(super) fn escapes_below(&self, index: usize) -> bool {
        if self.clip_escape_count == 0 {
            return false;
        }
        let key = (self.topology_revision, self.escape_revision);
        let mut cache = self.escapes.borrow_mut();
        if cache.key != Some(key) || cache.below.len() != self.nodes.len() {
            let topology = self.topology();
            let parent_of = |at: usize| match topology.parent[at] {
                NO_PARENT | ORPHAN_PARENT => None,
                parent => Some(parent),
            };
            cache.below.clear();
            cache.below.resize(self.nodes.len(), false);
            for (at, node) in self.nodes.iter().enumerate() {
                if !node.escapes_clip {
                    continue;
                }
                let mut up = parent_of(at);
                // Stopping at a marked ancestor is exact: whoever marked it went
                // on to mark everything above it.
                while let Some(parent) = up {
                    if cache.below[parent] {
                        break;
                    }
                    cache.below[parent] = true;
                    up = parent_of(parent);
                }
            }
            cache.key = Some(key);
        }
        cache.below[index]
    }
}
