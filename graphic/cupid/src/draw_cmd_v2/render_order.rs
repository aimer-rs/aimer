use super::*;

/// Marks a root in [`Topology::parent`].
pub(super) const NO_PARENT: usize = usize::MAX;

/// Marks a node whose parent identity names no node in [`Topology::parent`].
pub(super) const ORPHAN_PARENT: usize = usize::MAX - 1;

/// Node positions for every parent and child link, resolved once per topology.
///
/// A link is stored as an element identity, and turning it into a position in
/// the node array costs a hash lookup. A scrolling frame plans every node of
/// the tree while the structure stays the same, so those lookups were repeated
/// several times per node per frame for answers that had not changed. This
/// resolves them when [`DrawCommandList::topology_revision`] moves and lets
/// the per-frame passes, and every question about a node's ancestors, index
/// plain slices.
#[derive(Default)]
pub(super) struct Topology {
    revision: Option<u64>,
    /// Positions of the roots, in paint order.
    pub(super) roots: Vec<usize>,
    /// Position of each node's parent, [`NO_PARENT`] for a root or
    /// [`ORPHAN_PARENT`] when the parent is missing.
    pub(super) parent: Vec<usize>,
    /// `children[child_start[i]..child_start[i + 1]]` are node `i`'s children,
    /// in paint order. One extra entry closes the last node's range.
    child_start: Vec<usize>,
    children: Vec<usize>,
}

impl Topology {
    /// Whether this was resolved against the tree's current structure.
    #[inline]
    fn is_current(&self, tree: &DrawCommandList) -> bool {
        self.revision == Some(tree.topology_revision) && self.parent.len() == tree.nodes.len()
    }

    fn refresh(&mut self, tree: &DrawCommandList) {
        if self.is_current(tree) {
            return;
        }
        self.roots.clear();
        self.roots.extend(
            tree.roots
                .iter()
                .filter_map(|root| tree.indices.get(root).copied()),
        );
        self.parent.clear();
        self.child_start.clear();
        self.children.clear();
        for node in &tree.nodes {
            self.parent.push(match node.parent {
                None => NO_PARENT,
                Some(parent) => tree.indices.get(&parent).copied().unwrap_or(ORPHAN_PARENT),
            });
            self.child_start.push(self.children.len());
            self.children.extend(
                node.children
                    .iter()
                    .filter_map(|child| tree.indices.get(child).copied()),
            );
        }
        self.child_start.push(self.children.len());
        self.revision = Some(tree.topology_revision);
    }

    #[inline]
    pub(super) fn children_of(&self, index: usize) -> &[usize] {
        &self.children[self.child_start[index]..self.child_start[index + 1]]
    }
}

impl DrawCommandList {
    /// The parent and child positions of the current structure, resolved
    /// again only when the structure has changed since they were last read.
    pub(super) fn topology(&self) -> std::cell::Ref<'_, Topology> {
        // A shared look first: a caller that already holds the topology must
        // be able to ask again without needing it exclusively.
        if !self.topology.borrow().is_current(self) {
            self.topology.borrow_mut().refresh(self);
        }
        self.topology.borrow()
    }
}

#[derive(Default)]
pub(super) struct RenderWorkspace {
    /// The structure and geometry revisions, and node count, that `states`
    /// were derived for. A plan made while they still hold has nothing to
    /// derive: paint, opacity and the recorded lists are read from the nodes
    /// directly, and none of them feed the states.
    prepared_for: Option<(u64, u64, usize)>,
    /// How many operations the previous plan held: a tree that is drawn every
    /// frame plans about the same amount each time, so the next output is
    /// sized for it instead of growing by doubling and copying.
    output_hint: usize,
    states: Vec<NodeFrameState>,
    visit_stack: Vec<usize>,
    node_order: Vec<usize>,
    render_stack: Vec<RenderTraversal>,
}

#[derive(Clone, Copy)]
struct NodeFrameState {
    origin: (f32, f32),
    transform: Mat3,
    bounds: Option<Rect>,
    clip: ClipState,
    clip_radius: [f32; 4],
    visible_subtree_bounds: Option<Rect>,
}

impl Default for NodeFrameState {
    fn default() -> Self {
        Self {
            origin: (0.0, 0.0),
            transform: Mat3::identity(),
            bounds: None,
            clip: ClipState::Unclipped,
            clip_radius: [0.0; 4],
            visible_subtree_bounds: None,
        }
    }
}

/// What a root inherits: nothing offset, transformed or clipped.
const ROOT_STATE: NodeFrameState = NodeFrameState {
    origin: (0.0, 0.0),
    transform: Mat3::identity(),
    bounds: None,
    clip: ClipState::Unclipped,
    clip_radius: [0.0; 4],
    visible_subtree_bounds: None,
};

/// Maps a clip's local corner radii into world space.
///
/// Radii scale with the clip's size change under the node transform; for the
/// translation and uniform scale transforms retained nodes use, the result is
/// exact, and otherwise it matches the axis-aligned approximation already used
/// for the clip rectangle.
#[inline]
fn world_clip_radius(radius: [f32; 4], local: Rect, world: Rect) -> [f32; 4] {
    if radius == [0.0; 4] || local.width <= 0.0 || local.height <= 0.0 {
        return radius;
    }
    let scale = (world.width / local.width + world.height / local.height) * 0.5;
    if scale == 1.0 {
        return radius;
    }
    radius.map(|corner| corner * scale)
}

enum RenderTraversal {
    Node(usize),
    /// A child of a morph node: wrapped in a source group of its own, then
    /// planned as the ordinary node it is.
    MorphSource(usize, MorphGroup),
    EndOpacityGroup(RenderNodeId),
}

impl RenderWorkspace {
    /// Drops the geometry kept from the last plan, so the next one derives it.
    #[cfg(test)]
    fn forget_geometry(&mut self) {
        self.prepared_for = None;
    }

    fn prepare(&mut self, tree: &DrawCommandList) {
        let revisions = (
            tree.geometry_revision,
            tree.topology_revision,
            tree.nodes.len(),
        );
        if self.prepared_for == Some(revisions) {
            return;
        }
        #[cfg(test)]
        PLAN_GEOMETRY_RUNS.with(|count| count.set(count.get() + 1));
        let topology = tree.topology();
        self.states
            .resize_with(tree.nodes.len(), NodeFrameState::default);
        self.visit_stack.clear();
        self.node_order.clear();

        // Plain index loops: the adapter chains these replace cost several
        // calls per element in an unoptimized build, for a body that is a push.
        let roots = &topology.roots;
        let mut next = roots.len();
        while next > 0 {
            next -= 1;
            self.visit_stack.push(roots[next]);
        }

        while let Some(index) = self.visit_stack.pop() {
            let Some(node) = tree.nodes.get(index) else {
                continue;
            };
            // Read in place: copying a parent's frame out only to read three
            // fields of it costs more than the reads.
            let parent = self
                .states
                .get(topology.parent[index])
                .unwrap_or(&ROOT_STATE);
            let origin = (
                parent.origin.0 + node.bounds.x,
                parent.origin.1 + node.bounds.y,
            );
            // A node without its own transforms leaves the parent's transform
            // untouched: translating to the origin and back is the identity,
            // so the four products that would rebuild it are skipped.
            let moves_itself =
                !node.presentation_transform.is_identity() || !node.transform.is_identity();
            let transform = if moves_itself {
                parent
                    .transform
                    .mul(&Mat3::translate(origin.0, origin.1))
                    .mul(&node.presentation_transform)
                    .mul(&node.transform)
                    .mul(&Mat3::translate(-origin.0, -origin.1))
            } else {
                parent.transform
            };
            let Some(bounds) = transform_rect(
                transform,
                Rect::new(origin.0, origin.1, node.bounds.width, node.bounds.height),
            ) else {
                continue;
            };

            let mut clip = if node.escapes_clip { ClipState::Unclipped } else { parent.clip };
            let mut clip_radius = if node.escapes_clip { [0.0; 4] } else { parent.clip_radius };
            if !node.escapes_clip && let Some(local_clip) = node.clip {
                let world_clip =
                    transform_rect(transform, local_clip.translated(origin.0, origin.1));
                clip = clip.intersect(world_clip);
                if let Some(world_clip) = world_clip {
                    clip_radius = world_clip_radius(node.clip_radius, local_clip, world_clip);
                }
            }
            if !node.escapes_clip && let Some(local_clip) = node.animation_clip {
                // An animation clip is placed before the animation's own
                // transform, and only nodes that have one need this matrix.
                let transform_before_animation = if node.presentation_transform.is_identity() {
                    parent.transform
                } else {
                    parent
                        .transform
                        .mul(&Mat3::translate(origin.0, origin.1))
                        .mul(&node.presentation_transform)
                        .mul(&Mat3::translate(-origin.0, -origin.1))
                };
                let world_clip = transform_rect(
                    transform_before_animation,
                    local_clip.translated(origin.0, origin.1),
                );
                clip = clip.intersect(world_clip);
            }

            self.states[index] = NodeFrameState {
                origin,
                transform,
                bounds: Some(bounds),
                clip,
                clip_radius,
                visible_subtree_bounds: clip.intersect_bounds(bounds),
            };
            self.node_order.push(index);

            let children = topology.children_of(index);
            let mut next = children.len();
            while next > 0 {
                next -= 1;
                self.visit_stack.push(children[next]);
            }
        }

        let mut next = self.node_order.len();
        while next > 0 {
            next -= 1;
            let index = self.node_order[next];
            let child_visible_bounds = self.states[index].visible_subtree_bounds;
            let Some(parent_state) = self.states.get_mut(topology.parent[index]) else {
                continue;
            };
            union_bounds(
                &mut parent_state.visible_subtree_bounds,
                child_visible_bounds,
            );
        }
        self.prepared_for = Some(revisions);
    }

    fn collect(&mut self, tree: &DrawCommandList, damage: Option<&[Rect]>) -> Vec<RenderOp> {
        let topology = tree.topology();
        let mut output = Vec::with_capacity(self.output_hint);
        self.render_stack.clear();
        let roots = &topology.roots;
        let mut next = roots.len();
        while next > 0 {
            next -= 1;
            self.render_stack.push(RenderTraversal::Node(roots[next]));
        }

        while let Some(item) = self.render_stack.pop() {
            let index = match item {
                RenderTraversal::EndOpacityGroup(element) => {
                    output.push(RenderOp::EndOpacityGroup { element });
                    continue;
                }
                RenderTraversal::Node(index) => index,
                RenderTraversal::MorphSource(index, role) => {
                    let Some(node) = tree.nodes.get(index) else {
                        continue;
                    };
                    let state = &self.states[index];
                    let (Some(_), Some(visible_subtree)) =
                        (state.bounds, state.visible_subtree_bounds)
                    else {
                        continue;
                    };
                    // A source the damage misses paints nothing inside it, so
                    // the renderer treats the absent source as transparent.
                    if visible_subtree.width == 0.0
                        || visible_subtree.height == 0.0
                        || damage.is_some_and(|damage| {
                            !damage
                                .iter()
                                .any(|region| visible_subtree.intersection(*region).is_some())
                        })
                    {
                        continue;
                    }
                    output.push(RenderOp::BeginOpacityGroup {
                        element: node.id,
                        bounds: visible_subtree,
                        opacity: 1.0,
                        // The source's descendants keep their own container clips.
                        clip: None,
                        masked: false,
                        fade: false,
                        morph: Some(role),
                    });
                    self.render_stack
                        .push(RenderTraversal::EndOpacityGroup(node.id));
                    self.render_stack.push(RenderTraversal::Node(index));
                    continue;
                }
            };
            let Some(node) = tree.nodes.get(index) else {
                continue;
            };
            let state = &self.states[index];
            let Some(bounds) = state.bounds else {
                continue;
            };
            let Some(visible_subtree) = state.visible_subtree_bounds else {
                continue;
            };
            if visible_subtree.width == 0.0 || visible_subtree.height == 0.0 {
                continue;
            }
            if damage.is_some_and(|damage| {
                !damage
                    .iter()
                    .any(|region| visible_subtree.intersection(*region).is_some())
            }) {
                continue;
            }

            let id = node.id;
            let opacity = node.presentation_opacity * node.opacity;
            let has_children = !node.children.is_empty();
            let (masked, fade, morph) = {
                let draw_list = node.draw_list.borrow();
                // A morph needs both pictures; with any other child count the
                // marker is inert and the node paints as an ordinary one.
                (
                    draw_list.masks_descendants,
                    draw_list.fades_descendants,
                    draw_list.morph.filter(|_| topology.children_of(index).len() == 2),
                )
            };
            if (masked || fade) && !has_children {
                // Masking by nothing shows nothing, and the mask of a fade is
                // never shown on its own.
                continue;
            }
            // Child draw items keep their individual container clips.
            // Applying an ancestor's clip to the whole opacity
            // group would clip an overflowing descendant again.
            let group_clip = if tree.clip_escape_count != 0
                && state.clip.intersect_bounds(visible_subtree) != Some(visible_subtree)
            {
                None
            } else {
                state.clip.as_option()
            };
            if let Some(params) = morph {
                // The host owns no pixels: the marker draws nothing, so only its
                // two source groups are planned inside it.
                output.push(RenderOp::BeginOpacityGroup {
                    element: id,
                    bounds: visible_subtree,
                    opacity,
                    masked: false,
                    fade: false,
                    clip: group_clip,
                    morph: Some(MorphGroup::Host(params)),
                });
                self.render_stack.push(RenderTraversal::EndOpacityGroup(id));
                let &[from, to] = topology.children_of(index) else {
                    unreachable!("a morph host has exactly two children");
                };
                self.render_stack
                    .push(RenderTraversal::MorphSource(to, MorphGroup::To));
                self.render_stack
                    .push(RenderTraversal::MorphSource(from, MorphGroup::From));
                continue;
            }
            let groups_opacity = (opacity < 1.0 || masked || fade) && has_children;
            if groups_opacity {
                output.push(RenderOp::BeginOpacityGroup {
                    element: id,
                    bounds: visible_subtree,
                    opacity,
                    masked,
                    fade,
                    clip: group_clip,
                    morph: None,
                });
            }

            let visible_bounds = state.clip.intersect_bounds(bounds);
            if let Some(visible_bounds) = visible_bounds
                && damage.is_none_or(|damage| {
                    damage
                        .iter()
                        .any(|region| visible_bounds.intersection(*region).is_some())
                })
            {
                output.push(RenderOp::Draw(RenderItem {
                    element: id,
                    bounds,
                    origin: state.origin,
                    transform: state.transform,
                    clip: state.clip.as_option(),
                    clip_radius: state.clip_radius,
                    opacity: if groups_opacity { 1.0 } else { opacity },
                    paint_source: node.paint_source,
                    draw_list: node.draw_list.clone(),
                }));
            }

            if groups_opacity {
                self.render_stack.push(RenderTraversal::EndOpacityGroup(id));
            }
            // A node without a clip may have children that extend beyond its bounds.
            let children = topology.children_of(index);
            let mut next = children.len();
            while next > 0 {
                next -= 1;
                self.render_stack
                    .push(RenderTraversal::Node(children[next]));
            }
        }

        self.output_hint = output.len();
        output
    }
}

fn union_bounds(bounds: &mut Option<Rect>, other: Option<Rect>) {
    if let Some(other) = other {
        *bounds = Some(bounds.map_or(other, |bounds| bounds.union(other)));
    }
}

impl RenderTree {
    /// Returns visible elements in parent-first paint order for these damage regions.
    ///
    /// An empty damage slice means no pixels are dirty. The plan contains every
    /// intersecting node, including clean nodes behind a dirty transparent node,
    /// so the renderer can reconstruct the damaged pixels in the right order.
    pub fn render_order(&self, damage: &[Rect]) -> Vec<RenderOp> {
        if damage.is_empty() {
            return Vec::new();
        }
        let tree = self.draw_cmd.borrow();
        let mut workspace = tree.render_workspace.borrow_mut();
        workspace.prepare(&tree);
        workspace.collect(&tree, Some(damage))
    }

    /// Returns every render node in paint order, without damage culling.
    pub fn render_all(&self) -> Vec<RenderOp> {
        let tree = self.draw_cmd.borrow();
        let mut workspace = tree.render_workspace.borrow_mut();
        workspace.prepare(&tree);
        workspace.collect(&tree, None)
    }

    /// Plans every node after forgetting the geometry kept from earlier plans.
    #[cfg(test)]
    pub(super) fn render_all_from_scratch(&self) -> Vec<RenderOp> {
        self.draw_cmd
            .borrow()
            .render_workspace
            .borrow_mut()
            .forget_geometry();
        self.render_all()
    }

    /// Takes pending damage and creates its parent-first composition plan.
    pub fn render_pending(&self) -> RenderFrame {
        let damage = self.take_damage();
        let operations = self.render_order(&damage);
        RenderFrame { damage, operations }
    }
}
