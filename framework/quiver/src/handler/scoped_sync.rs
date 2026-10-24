//! Re-synchronizing the render tree for the subtrees a rebuild replaced.
//!
//! A full synchronization collects every element of the window. That is the
//! right answer when the tree changed in a way no single subtree describes, but
//! the usual cause of a synchronization is far smaller: a stateful element
//! rebuilt its child, and the replacement has the same shape as what it
//! replaced. Everything outside that subtree is where it was.
//!
//! [`WindowRenderTree::sync_scoped`] handles that case. It recomputes the
//! inputs the replaced subtree's root was given by walking down from the root
//! of the window along the ancestor path, with the same helpers the full
//! collector uses, collects the subtree, and compares the result with what the
//! last synchronization stored. Only when the subtree has exactly the same
//! elements in the same arrangement, and its root occupies exactly the same
//! geometry, are the changed nodes updated in place. Anything else — a node
//! added or removed, a root that grew or moved, an ancestor that cannot be
//! walked — is reported as not handled, and the caller does the full
//! synchronization.

use std::collections::HashMap;

use aimer_cupid::draw_cmd_v2::NodeGeometry;
use aimer_cupid::utilities::IdBuildHasher;

use super::*;

/// What the last synchronization of the window produced, in the preorder it
/// collected the elements.
pub(super) struct SyncCache {
    element_ids: Vec<ElementId>,
    specs: Vec<RenderNodeSpec>,
    clips: Vec<Option<RenderRect>>,
    clip_radii: Vec<[f32; 4]>,
    render_ids: Vec<RenderNodeId>,
    /// Built the first time a scoped synchronization needs it.
    index: Option<SubtreeIndex>,
}

/// Where each element is, and where its subtree ends.
struct SubtreeIndex {
    /// Exclusive end of the preorder range of each element's subtree.
    end: Vec<usize>,
    position: HashMap<ElementId, usize, IdBuildHasher>,
}

impl SyncCache {
    pub(super) fn new(
        element_ids: Vec<ElementId>,
        specs: Vec<RenderNodeSpec>,
        clips: Vec<Option<RenderRect>>,
        clip_radii: Vec<[f32; 4]>,
        render_ids: Vec<RenderNodeId>,
    ) -> Self {
        Self {
            element_ids,
            specs,
            clips,
            clip_radii,
            render_ids,
            index: None,
        }
    }

    fn index(&mut self) -> &SubtreeIndex {
        self.index.get_or_insert_with(|| {
            let count = self.element_ids.len();
            let mut size = vec![1usize; count];
            for node in (1..count).rev() {
                if let Some(parent) = self.specs[node].parent_index {
                    size[parent] += size[node];
                }
            }
            let end = (0..count).map(|node| node + size[node]).collect();
            let mut position = HashMap::with_capacity_and_hasher(count, IdBuildHasher::default());
            position.extend(self.element_ids.iter().copied().zip(0..));
            SubtreeIndex { end, position }
        })
    }
}

impl WindowRenderTree {
    /// Synchronizes just the subtrees rooted at `dirty`, if that is enough.
    ///
    /// Returns `true` when every subtree was brought up to date, and `false`
    /// when the caller must synchronize the whole window instead. A `false`
    /// result may follow some subtrees already having been updated; those
    /// updates are correct, and the full synchronization that follows
    /// rewrites everything anyway.
    pub(super) fn sync_scoped(
        &mut self,
        root: &AnyElement,
        ctx: &BuildContext<'_>,
        scale: f32,
        dirty: &[ElementId],
    ) -> bool {
        if self.scoped_sync_disabled || !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        let Some(mut cache) = self.sync_cache.take() else {
            return false;
        };
        let handled = self.sync_scoped_with(&mut cache, root, ctx, scale, dirty);
        if handled {
            self.sync_cache = Some(cache);
        }
        handled
    }

    fn sync_scoped_with(
        &mut self,
        cache: &mut SyncCache,
        root: &AnyElement,
        ctx: &BuildContext<'_>,
        scale: f32,
        dirty: &[ElementId],
    ) -> bool {
        let targets = {
            let index = cache.index();
            let mut found = Vec::with_capacity(dirty.len());
            for id in dirty {
                let Some(&node) = index.position.get(id) else {
                    // A replacement whose root has no node yet is a new element.
                    return false;
                };
                found.push(node);
            }
            found.sort_unstable();
            found.dedup();
            // A replacement inside another replaced subtree is covered by it.
            let mut kept = Vec::with_capacity(found.len());
            let mut covered_until = 0;
            for node in found {
                if node >= covered_until {
                    kept.push(node);
                    covered_until = index.end[node];
                }
            }
            kept
        };

        for target in targets {
            if !self.sync_scoped_subtree(cache, root, ctx, scale, target) {
                return false;
            }
        }
        true
    }

    fn sync_scoped_subtree(
        &mut self,
        cache: &mut SyncCache,
        root: &AnyElement,
        ctx: &BuildContext<'_>,
        scale: f32,
        target: usize,
    ) -> bool {
        let end = cache.index().end[target];

        // The ancestors of `target`, outermost first, ending with `target`.
        let mut chain = vec![target];
        while let Some(parent) =
            cache.specs[*chain.last().expect("chain starts non-empty")].parent_index
        {
            chain.push(parent);
        }
        chain.reverse();

        // Walk down to the target, deriving its inputs exactly as the full
        // collector derived them for its parent's children.
        let mut element: &dyn Element = root.as_ref();
        let mut current_ctx = ctx.clone();
        let mut geometry = None;
        let mut clip_radius = [0.0; 4];
        let mut origin_shift = (0.0, 0.0);
        let mut element_path = Vec::with_capacity(chain.len());
        for (depth, &node) in chain.iter().enumerate() {
            if element.id() != cache.element_ids[node] {
                return false;
            }
            let Some(&next) = chain.get(depth + 1) else {
                break;
            };
            let frame = node_frame(element, &current_ctx, scale, geometry, origin_shift);
            let mut base = ChildBaseContext::new(element, &current_ctx, frame.size, frame.position);
            let wanted = cache.element_ids[next];
            let mut child = None;
            element.visit_retained_v2_children(&mut |index, candidate| {
                if child.is_none() && candidate.id() == wanted {
                    child = Some((index, candidate));
                }
            });
            let Some((index, child)) = child else {
                return false;
            };
            let input = child_input(element, &current_ctx, &mut base, child, index);
            element_path.push(element.id());
            geometry = input.geometry;
            clip_radius = input.clip_radius;
            current_ctx = input.ctx;
            origin_shift = frame.own_outset;
            element = child;
        }

        let mut specs = Vec::with_capacity(end - target);
        let mut element_ids = Vec::with_capacity(end - target);
        let mut clips = Vec::with_capacity(end - target);
        let mut clip_radii = Vec::with_capacity(end - target);
        let mut invalid_bounds = Vec::new();
        let mut new_elements = Vec::new();
        let mut interactive = Vec::new();
        collect_render_tree_nodes(
            element,
            None,
            &current_ctx,
            scale,
            &self.element_nodes,
            &mut specs,
            &mut element_ids,
            &mut clips,
            &mut clip_radii,
            &mut invalid_bounds,
            &mut new_elements,
            &mut interactive,
            geometry,
            clip_radius,
            origin_shift,
            &mut element_path,
        );

        // The replacement must be the same subtree: the same elements, in the
        // same arrangement, with nothing new and nothing that had to be
        // collapsed.
        let range = target..end;
        if !invalid_bounds.is_empty()
            || !new_elements.is_empty()
            || element_ids != cache.element_ids[range.clone()]
            || specs.iter().enumerate().skip(1).any(|(offset, spec)| {
                spec.parent_index
                    != cache.specs[target + offset]
                        .parent_index
                        .map(|parent| parent - target)
            })
        {
            return false;
        }
        // Its root must also occupy the same geometry. If it does not, the
        // elements around it are laid out differently too.
        if specs[0].bounds != cache.specs[target].bounds
            || clips[0] != cache.clips[target]
            || clip_radii[0] != cache.clip_radii[target]
        {
            return false;
        }

        let updates = (0..specs.len())
            .filter(|&offset| {
                let node = target + offset;
                specs[offset].bounds != cache.specs[node].bounds
                    || clips[offset] != cache.clips[node]
                    || clip_radii[offset] != cache.clip_radii[node]
            })
            .map(|offset| NodeGeometry {
                node: cache.render_ids[target + offset],
                bounds: specs[offset].bounds,
                clip: clips[offset],
                clip_radius: clip_radii[offset],
            })
            .collect::<Vec<_>>();
        if self.tree.sync_geometry(&updates).is_err() {
            return false;
        }

        // Elements of the replacement are new instances, and each interactive
        // one must be handed its render node again.
        self.adopt_interaction_bounds(&interactive, &cache.render_ids[range], scale);

        for (offset, spec) in specs.iter().enumerate() {
            let node = target + offset;
            cache.specs[node].bounds = spec.bounds;
            cache.clips[node] = clips[offset];
            cache.clip_radii[node] = clip_radii[offset];
        }
        true
    }
}
