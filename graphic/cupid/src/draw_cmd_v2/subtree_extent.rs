//! Footprints of whole subtrees, for damage that must not walk them.
//!
//! Presenting a node (sliding a page, fading it) damages where its subtree was
//! and where it goes. Asking for those footprints used to visit every node
//! below it, three times, on every frame of the animation, although nothing
//! inside the subtree had changed: only the matrix applied to it had.
//!
//! The part that does not change is what a subtree covers in its own content
//! space, before its own transform and presentation. [`Extent`] keeps that per
//! node, built from the extents of its children, and a node's footprint is then
//! one matrix applied to one rectangle. An edit marks the edited node and the
//! chain above it [`Slot::Stale`]; nothing else is measured again.
//!
//! A footprint here is a *cover*, not the exact union the walk computes: it is
//! the exact union for translations and scales without clips cutting the
//! content, and never smaller otherwise. That is all damage needs. Subtrees the
//! summary cannot describe ([`Extent::Opaque`]) fall back to the walk.

use super::render_order::{NO_PARENT, ORPHAN_PARENT, Topology};
use super::world::WorldFrame;
use super::*;

// Counts subtrees measured from their children, so tests can show an edit
// measures the path above it and nothing else. Compiled out of every non-test
// build.
#[cfg(test)]
thread_local! {
    static EXTENT_COMPUTATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn extent_computations() -> usize {
    EXTENT_COMPUTATIONS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn reset_extent_computations() {
    EXTENT_COMPUTATIONS.with(|count| count.set(0));
}

/// What a node and everything below it covers, in the node's own content space:
/// the node's origin is `(0, 0)`, and its own transform, presentation and the
/// clips of its ancestors are not applied.
#[derive(Clone, Copy)]
pub(super) enum Extent {
    /// Not described: the subtree escapes clipping, animates a clip, or places
    /// something with more than a scale and a translation.
    Opaque,
    Known {
        /// Where anything can reach pixels once the clips inside the subtree
        /// have cut it, or `None` when they cut all of it away.
        visible: Option<Rect>,
        /// Every node's rectangle with no clip applied: a single non-finite
        /// bound anywhere below makes the mapped total non-finite as well.
        total: Rect,
    },
}

#[derive(Clone, Copy)]
enum Slot {
    /// Must be measured again before it is read.
    Stale,
    Measured(Extent),
}

/// The extent of every node, valid for one structure of the tree.
///
/// A `Measured` slot depends only on nodes that were visited to measure it,
/// and every edit marks the chain above the edited node, so a slot that is
/// `Measured` is current. A `Stale` slot has only `Stale` ancestors, which lets
/// marking stop at the first one it meets.
#[derive(Default)]
pub(super) struct ExtentCache {
    /// The `topology_revision` the slots were laid out against.
    revision: Option<u64>,
    slots: Vec<Slot>,
}

impl ExtentCache {
    /// Discards every slot unless they belong to this structure.
    fn align(&mut self, revision: u64, nodes: usize) {
        if self.revision != Some(revision) || self.slots.len() != nodes {
            self.slots.clear();
            self.slots.resize(nodes, Slot::Stale);
            self.revision = Some(revision);
        }
    }
}

/// Whether `placement` keeps rectangles rectangles: a scale and a translation,
/// no rotation, shear or perspective.
#[inline]
fn is_axis_aligned(placement: &Mat3) -> bool {
    let [x, y, w] = &placement.cols;
    x[1] == 0.0 && y[0] == 0.0 && x[2] == 0.0 && y[2] == 0.0 && w[2] == 1.0
}

impl DrawCommandList {
    /// Marks the extents that an edit to the node at `index` can change.
    ///
    /// `include_self` is `false` for an edit to the node's own transform or
    /// presentation, which its extent leaves out; its parent's extent places it
    /// with them, so everything above is still marked.
    pub(super) fn mark_extent_stale(&mut self, index: usize, include_self: bool) {
        let cache = self.extents.get_mut();
        if cache.revision != Some(self.topology_revision) {
            // Nothing was measured against this structure.
            return;
        }
        let parents = &self.topology.get_mut().parent;
        let parent_of = |at: usize| match parents[at] {
            NO_PARENT | ORPHAN_PARENT => None,
            parent => Some(parent),
        };
        let mut at = if include_self {
            Some(index)
        } else {
            parent_of(index)
        };
        while let Some(index) = at {
            if matches!(cache.slots[index], Slot::Stale) {
                break;
            }
            cache.slots[index] = Slot::Stale;
            at = parent_of(index);
        }
    }

    fn extent_of(&self, index: usize) -> Extent {
        let topology = self.topology();
        let mut cache = self.extents.borrow_mut();
        cache.align(self.topology_revision, self.nodes.len());
        self.measure(index, &topology, &mut cache)
    }

    fn measure(&self, index: usize, topology: &Topology, cache: &mut ExtentCache) -> Extent {
        if let Slot::Measured(extent) = cache.slots[index] {
            return extent;
        }
        #[cfg(test)]
        EXTENT_COMPUTATIONS.with(|count| count.set(count.get() + 1));
        let extent = self.measure_node(index, topology, cache);
        cache.slots[index] = Slot::Measured(extent);
        extent
    }

    fn measure_node(&self, index: usize, topology: &Topology, cache: &mut ExtentCache) -> Extent {
        let node = &self.nodes[index];
        if node.escapes_clip || node.animation_clip.is_some() {
            return Extent::Opaque;
        }
        let own = Rect::new(0.0, 0.0, node.bounds.width, node.bounds.height);
        if !own.is_valid() {
            return Extent::Opaque;
        }
        let mut total = own;
        let mut visible = Some(own);
        for &child_index in topology.children_of(index) {
            let Extent::Known {
                visible: child_visible,
                total: child_total,
            } = self.measure(child_index, topology, cache)
            else {
                return Extent::Opaque;
            };
            let child = &self.nodes[child_index];
            // Where a point of the child lands among its parent's: the child's
            // transform, then its presentation, about its own origin, and then
            // the offset of that origin.
            let placement = if child.presentation_transform.is_identity() {
                child.transform
            } else {
                child.presentation_transform.mul(&child.transform)
            };
            if !is_axis_aligned(&placement) {
                return Extent::Opaque;
            }
            let place = |rect: Rect| {
                transform_rect(placement, rect).map(|rect| {
                    rect.translated(child.bounds.x, child.bounds.y)
                })
            };
            let Some(placed_total) = place(child_total) else {
                return Extent::Opaque;
            };
            total = total.union(placed_total);
            if let Some(child_visible) = child_visible {
                let Some(placed) = place(child_visible) else {
                    return Extent::Opaque;
                };
                visible = Some(visible.map_or(placed, |visible| visible.union(placed)));
            }
        }
        // The clip cuts the node's own rectangle and everything below it.
        if let Some(clip) = node.clip {
            visible = visible.and_then(|visible| visible.intersection(clip));
        }
        Extent::Known { visible, total }
    }

    /// The node's frame together with its extent, when the extent describes
    /// the subtree and the subtree can be mapped to finite bounds.
    fn mapped_extent(&self, id: RenderNodeId) -> Option<(WorldFrame, Option<Rect>)> {
        let index = *self.indices.get(&id)?;
        let node = self.nodes.get(index)?;
        let parent = self.frame_below(node.parent)?;
        let Extent::Known { visible, total } = self.extent_of(index) else {
            return None;
        };
        let frame = node.frame(&parent);
        let (x, y) = frame.origin;
        // A bound that does not map to a finite rectangle is for the walk to
        // judge, node by node: some of the nodes below may still be finite.
        transform_rect(frame.transform, total.translated(x, y))?;
        let visible = match visible {
            Some(visible) => Some(transform_rect(frame.transform, visible.translated(x, y))?),
            None => None,
        };
        Some((parent, visible))
    }

    /// A rectangle covering [`Self::visible_subtree_bounds`] of `id`.
    ///
    /// Equal to it when nothing clips the content and every placement is a
    /// scale and a translation, and larger only where it must be. Costs the
    /// depth of the node, not the size of its subtree, once the subtree has
    /// been measured.
    pub(super) fn damage_footprint(&self, id: RenderNodeId) -> Option<Rect> {
        match self.mapped_extent(id) {
            Some((parent, visible)) => visible.and_then(|visible| parent.clip.intersect_bounds(visible)),
            None => self.visible_subtree_bounds(id),
        }
    }

    /// Whether every node of the subtree has finite world bounds, which is
    /// exactly when [`Self::subtree_bounds`] has an answer.
    pub(super) fn subtree_is_finite(&self, id: RenderNodeId) -> bool {
        self.mapped_extent(id).is_some() || self.subtree_bounds(id).is_some()
    }
}
