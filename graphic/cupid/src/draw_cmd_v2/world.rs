//! World-space queries on the retained render tree.
//!
//! A node's world origin, transform and clip all depend on every ancestor.
//! Deriving each from scratch makes a single query cost depth² or worse, and
//! the damage bookkeeping asks these questions for nearly every node of a
//! scrolling tree on every frame. Instead each query walks to the root once
//! to build the parent's [`WorldFrame`], then derives the node's frame and
//! those of its descendants top-down, so the cost is the depth plus the
//! subtree actually covered.
//!
//! The arithmetic, including the matrix multiplication order, is identical to
//! the recursive definitions this replaces: the tests compare them
//! bit-for-bit.

use smallvec::SmallVec;

use super::render_order::{NO_PARENT, ORPHAN_PARENT};
use super::*;

/// The world state a node hands to its children.
#[derive(Clone, Copy)]
pub(super) struct WorldFrame {
    /// World-space origin of the node's local coordinate space.
    pub(super) origin: (f32, f32),
    /// Cumulative transform from untransformed world space to display space.
    pub(super) transform: Mat3,
    /// The clip inherited from ancestors, including the node's own clips.
    pub(super) clip: ClipState,
}

impl WorldFrame {
    /// The state above every root: nothing offset, transformed or clipped.
    #[inline]
    fn root() -> Self {
        Self {
            origin: (0.0, 0.0),
            transform: Mat3::identity(),
            clip: ClipState::Unclipped,
        }
    }
}

/// One node's world state together with its own world-space bounds.
struct NodeWorld {
    frame: WorldFrame,
    /// `None` when the node's transform produced non-finite bounds.
    bounds: Option<Rect>,
}

impl RenderNode {
    /// Derives this node's world state from its parent's.
    #[inline]
    fn world(&self, parent: &WorldFrame) -> NodeWorld {
        let frame = self.frame(parent);
        NodeWorld {
            bounds: transform_rect(
                frame.transform,
                Rect::new(
                    frame.origin.0,
                    frame.origin.1,
                    self.bounds.width,
                    self.bounds.height,
                ),
            ),
            frame,
        }
    }

    /// The state this node hands to its children, without its own bounds.
    ///
    /// Following a chain of ancestors only needs this: the bounds of an
    /// ancestor are never read, and finding them takes four point transforms
    /// and the checks that go with them.
    #[inline]
    pub(super) fn frame(&self, parent: &WorldFrame) -> WorldFrame {
        #[cfg(test)]
        FRAME_DERIVATIONS.with(|count| count.set(count.get() + 1));
        let (x, y) = (
            parent.origin.0 + self.bounds.x,
            parent.origin.1 + self.bounds.y,
        );
        // A node without transforms of its own passes its parent's transform on
        // untouched: translating to its origin and back is the identity
        // exactly, so the four products that would rebuild it are skipped.
        // Nearly every node of a real interface is one of these.
        let presents = !self.presentation_transform.is_identity();
        let transform = if presents || !self.transform.is_identity() {
            let to_origin = Mat3::translate(x, y);
            let from_origin = Mat3::translate(-x, -y);
            let local = to_origin
                .mul(&self.presentation_transform)
                .mul(&self.transform)
                .mul(&from_origin);
            parent.transform.mul(&local)
        } else {
            parent.transform
        };

        // The clip stored on an escaping node comes from its parent geometry.
        // Drop it together with inherited clips; descendants keep their own clips.
        let mut clip = if self.escapes_clip { ClipState::Unclipped } else { parent.clip };
        if !self.escapes_clip && let Some(local_clip) = self.clip {
            clip = clip.intersect(transform_rect(transform, local_clip.translated(x, y)));
        }
        if !self.escapes_clip && let Some(local_clip) = self.animation_clip {
            // An animation clip is placed before the animation's own transform.
            let before_animation = if presents {
                parent.transform.mul(
                    &Mat3::translate(x, y)
                        .mul(&self.presentation_transform)
                        .mul(&Mat3::translate(-x, -y)),
                )
            } else {
                parent.transform
            };
            let world_clip = transform_rect(
                before_animation,
                local_clip.translated(x, y),
            );
            clip = clip.intersect(world_clip);
        }

        WorldFrame {
            origin: (x, y),
            transform,
            clip,
        }
    }
}

impl DrawCommandList {
    /// The world state that `parent`'s children inherit, or `None` if an
    /// ancestor is missing.
    ///
    /// A parent is found with one lookup; everything above it comes from the
    /// cached frames of the current geometry revision. See [`Self::frame_at`].
    pub(super) fn frame_below(&self, parent: Option<RenderNodeId>) -> Option<WorldFrame> {
        let Some(parent) = parent else {
            return Some(WorldFrame::root());
        };
        #[cfg(test)]
        NODE_LOOKUPS.with(|count| count.set(count.get() + 1));
        self.frame_at(*self.indices.get(&parent)?)
    }

    /// The frame the node at `index` hands to its children.
    ///
    /// Hit testing, damage and every viewport-relative question ask for the
    /// frames of nodes that share most of their ancestors, many times over the
    /// same geometry. The walk up stops at the nearest ancestor whose frame is
    /// already known for the current revision, and the frames it derived on the
    /// way down are kept, so a frame costs one derivation per revision however
    /// often it is needed.
    fn frame_at(&self, index: usize) -> Option<WorldFrame> {
        let topology = self.topology();
        let mut cache = self.world_frames.borrow_mut();
        if cache.len() < self.nodes.len() {
            cache.resize(self.nodes.len(), None);
        }
        let revision = self.geometry_revision;

        // The nodes whose frames are not known yet, nearest first.
        let mut pending: SmallVec<[usize; 32]> = SmallVec::new();
        let mut frame = WorldFrame::root();
        let mut at = index;
        loop {
            if let Some((cached_at, known)) = cache[at]
                && cached_at == revision
            {
                frame = known;
                break;
            }
            pending.push(at);
            match topology.parent[at] {
                NO_PARENT => break,
                ORPHAN_PARENT => return None,
                above => at = above,
            }
        }
        for &position in pending.iter().rev() {
            frame = self.nodes.get(position)?.frame(&frame);
            cache[position] = Some((revision, frame));
        }
        Some(frame)
    }

    /// The node's own world state and bounds.
    fn node_world(&self, id: RenderNodeId) -> Option<NodeWorld> {
        let node = self.node(id)?;
        Some(node.world(&self.frame_below(node.parent)?))
    }

    pub(super) fn world_bounds(&self, id: RenderNodeId) -> Option<Rect> {
        self.node_world(id)?.bounds
    }

    /// The node's bounds after clipping by every ancestor and by itself.
    pub(super) fn visible_node_bounds(&self, id: RenderNodeId) -> Option<Rect> {
        let world = self.node_world(id)?;
        world.frame.clip.intersect_bounds(world.bounds?)
    }

    /// The union of the node's and its descendants' world bounds. `None` if
    /// any of them has non-finite bounds or is missing.
    pub(super) fn subtree_bounds(&self, id: RenderNodeId) -> Option<Rect> {
        let node = self.node(id)?;
        self.subtree_bounds_from(node, &self.frame_below(node.parent)?)
    }

    fn subtree_bounds_from(&self, node: &RenderNode, parent: &WorldFrame) -> Option<Rect> {
        let world = node.world(parent);
        let mut bounds = world.bounds?;
        for child in node.children.iter().copied() {
            let child = self.node(child)?;
            bounds = bounds.union(self.subtree_bounds_from(child, &world.frame)?);
        }
        Some(bounds)
    }

    /// Where anything drawn by the node or below it can reach pixels, in the
    /// node's own local coordinates.
    ///
    /// That is the clip the node hands down, so it ignores the node's own
    /// bounds: a node whose children overflow it still shows them. Placement
    /// that is anything but a translation (scale, rotation, perspective)
    /// leaves local and world rectangles unrelated by an offset, so no region
    /// is claimed for it.
    pub(super) fn local_visible_region(&self, id: RenderNodeId) -> Option<LocalVisibleRegion> {
        let node = self.node(id)?;
        let world = node.world(&self.frame_below(node.parent)?);
        let [x_axis, y_axis, w_axis] = world.frame.transform.cols;
        // The 2x2 part must be the identity, up to the noise of undoing a
        // translation about an origin, and there must be no perspective.
        const EPSILON: f32 = 1e-5;
        let near = |value: f32, expected: f32| (value - expected).abs() <= EPSILON;
        if !(near(x_axis[0], 1.0)
            && near(x_axis[1], 0.0)
            && near(y_axis[0], 0.0)
            && near(y_axis[1], 1.0)
            && near(x_axis[2], 0.0)
            && near(y_axis[2], 0.0)
            && near(w_axis[2], 1.0))
        {
            return Some(LocalVisibleRegion::Unbounded);
        }
        Some(match world.frame.clip {
            ClipState::Unclipped => LocalVisibleRegion::Unbounded,
            ClipState::Empty => LocalVisibleRegion::Nothing,
            ClipState::Clipped(clip) => {
                // Where the node's local (0, 0) lands in the world.
                let (x, y) = world
                    .frame
                    .transform
                    .transform_point(world.frame.origin.0, world.frame.origin.1);
                LocalVisibleRegion::Within(Rect::new(
                    clip.x - x,
                    clip.y - y,
                    clip.width,
                    clip.height,
                ))
            }
        })
    }

    /// The union of the visible, clipped bounds of the node and everything
    /// below it: the footprint that actually reaches pixels.
    pub(super) fn visible_subtree_bounds(&self, id: RenderNodeId) -> Option<Rect> {
        let node = self.node(id)?;
        let parent = self.frame_below(node.parent)?;

        // When no node escapes clipping, descendants stay inside the clip this
        // node hands down. When its own visible bounds already fill that clip,
        // the footprint is bracketed from both sides
        // and equals the clip without visiting a single descendant. A
        // scrollable's content node is exactly this case on every frame it
        // moves: it is taller than the viewport it is clipped to, and its
        // thousands of descendants would otherwise be walked to rediscover the
        // viewport rectangle.
        let world = node.world(&parent);
        if let ClipState::Clipped(clip) = world.frame.clip
            && self.clip_escape_count == 0
            && let Some(bounds) = world.bounds
            && bounds.intersection(clip) == Some(clip)
        {
            return Some(clip);
        }
        self.visible_subtree_bounds_from(node, &parent)
    }

    /// [`Self::visible_subtree_bounds`] for each of `ids`, left out when a node
    /// has none, with every node's footprint worked out once however many of
    /// `ids` lie above it.
    ///
    /// A structural edit damages the footprint of each parent it touched, and a
    /// page swapped in touches a parent at every level of the new subtree. Asked
    /// one by one, each of those walked the subtree below it again, so the cost
    /// grew with the depth of what was added. The footprint of a node holds the
    /// footprints of the nodes below it, so it is kept the first time it is
    /// found and read back by every query above it.
    pub(super) fn visible_subtree_bounds_of(&self, ids: &[RenderNodeId]) -> Vec<Rect> {
        let mut known: Vec<Option<Option<Rect>>> = vec![None; self.nodes.len()];
        ids.iter()
            .filter_map(|id| {
                let index = *self.indices.get(id)?;
                let node = &self.nodes[index];
                let parent = self.frame_below(node.parent)?;
                // The same early answer as `visible_subtree_bounds`, which does
                // not visit a single descendant.
                let world = node.world(&parent);
                if let ClipState::Clipped(clip) = world.frame.clip
                    && self.clip_escape_count == 0
                    && let Some(bounds) = world.bounds
                    && bounds.intersection(clip) == Some(clip)
                {
                    return Some(clip);
                }
                self.visible_subtree_bounds_known(index, &parent, &mut known)
            })
            .collect()
    }

    fn visible_subtree_bounds_known(
        &self,
        index: usize,
        parent: &WorldFrame,
        known: &mut [Option<Option<Rect>>],
    ) -> Option<Rect> {
        if let Some(found) = known[index] {
            return found;
        }
        let node = &self.nodes[index];
        let world = node.world(parent);
        let mut bounds = world
            .bounds
            .and_then(|bounds| world.frame.clip.intersect_bounds(bounds));
        // A node without bounds hides what is below it, as it does when asked
        // alone.
        if world.bounds.is_some() {
            for child in node.children.iter().copied() {
                let Some(&child) = self.indices.get(&child) else {
                    continue;
                };
                if let Some(child_bounds) =
                    self.visible_subtree_bounds_known(child, &world.frame, known)
                {
                    bounds = Some(bounds.map_or(child_bounds, |bounds| bounds.union(child_bounds)));
                }
            }
        }
        known[index] = Some(bounds);
        bounds
    }

    /// The footprint to damage when `id` or anything inside it changes: the
    /// union of the visible bounds of the node and every descendant.
    ///
    /// Unlike [`Self::visible_subtree_bounds`], a node whose own bounds are not
    /// finite still contributes its descendants instead of hiding them, so the
    /// result never covers less than the footprints of the nodes below it.
    pub(super) fn subtree_damage_bounds(&self, id: RenderNodeId) -> Option<Rect> {
        let node = self.node(id)?;
        let parent = self.frame_below(node.parent)?;
        let world = node.world(&parent);
        if let ClipState::Clipped(clip) = world.frame.clip
            && self.clip_escape_count == 0
            && let Some(bounds) = world.bounds
            && bounds.intersection(clip) == Some(clip)
        {
            return Some(clip);
        }
        self.subtree_damage_bounds_from(node, &parent)
    }

    fn subtree_damage_bounds_from(&self, node: &RenderNode, parent: &WorldFrame) -> Option<Rect> {
        let world = node.world(parent);
        let mut bounds = world
            .bounds
            .and_then(|bounds| world.frame.clip.intersect_bounds(bounds));
        for child in node.children.iter().copied() {
            let Some(child) = self.node(child) else {
                continue;
            };
            if let Some(child_bounds) = self.subtree_damage_bounds_from(child, &world.frame) {
                bounds = Some(bounds.map_or(child_bounds, |bounds| bounds.union(child_bounds)));
            }
        }
        bounds
    }

    fn visible_subtree_bounds_from(&self, node: &RenderNode, parent: &WorldFrame) -> Option<Rect> {
        let world = node.world(parent);
        let mut bounds = world.frame.clip.intersect_bounds(world.bounds?);
        for child in node.children.iter().copied() {
            let Some(child) = self.node(child) else {
                continue;
            };
            if let Some(child_bounds) = self.visible_subtree_bounds_from(child, &world.frame) {
                bounds = Some(bounds.map_or(child_bounds, |bounds| bounds.union(child_bounds)));
            }
        }
        bounds
    }
}
