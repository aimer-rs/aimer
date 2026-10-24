//! World-space queries on the retained render tree.
//!
//! Two properties matter. The answers must match the original recursive
//! definitions exactly, and a query must cost time proportional to the tree
//! depth plus the subtree it covers: scrolling re-asks these for nearly every
//! node every frame, so a super-linear walk dominates the frame.

use super::*;

/// The original definitions, kept verbatim as the specification the optimized
/// queries must reproduce. They recompute every ancestor's world state for each
/// node, which is what made them super-linear.
mod reference {
    use super::*;

    pub(super) fn world_origin(tree: &DrawCommandList, id: RenderNodeId) -> Option<(f32, f32)> {
        let node = tree.node(id)?;
        let parent_origin = node
            .parent
            .and_then(|parent| world_origin(tree, parent))
            .unwrap_or((0.0, 0.0));
        Some((
            parent_origin.0 + node.bounds.x,
            parent_origin.1 + node.bounds.y,
        ))
    }

    pub(super) fn world_transform(tree: &DrawCommandList, id: RenderNodeId) -> Option<Mat3> {
        let node = tree.node(id)?;
        let parent_transform = node.parent.map_or(Some(Mat3::identity()), |parent| {
            world_transform(tree, parent)
        })?;
        let (x, y) = world_origin(tree, id)?;
        let local_transform = Mat3::translate(x, y)
            .mul(&node.presentation_transform)
            .mul(&node.transform)
            .mul(&Mat3::translate(-x, -y));
        Some(parent_transform.mul(&local_transform))
    }

    pub(super) fn world_transform_before_animation(
        tree: &DrawCommandList,
        id: RenderNodeId,
    ) -> Option<Mat3> {
        let node = tree.node(id)?;
        let parent_transform = node.parent.map_or(Some(Mat3::identity()), |parent| {
            world_transform(tree, parent)
        })?;
        let (x, y) = world_origin(tree, id)?;
        let local_transform = Mat3::translate(x, y)
            .mul(&node.presentation_transform)
            .mul(&Mat3::translate(-x, -y));
        Some(parent_transform.mul(&local_transform))
    }

    pub(super) fn world_bounds(tree: &DrawCommandList, id: RenderNodeId) -> Option<Rect> {
        let node = tree.node(id)?;
        let (x, y) = world_origin(tree, id)?;
        transform_rect(
            world_transform(tree, id)?,
            Rect::new(x, y, node.bounds.width, node.bounds.height),
        )
    }

    pub(super) fn world_clip(tree: &DrawCommandList, id: RenderNodeId) -> ClipState {
        let Some(node) = tree.node(id) else {
            return ClipState::Empty;
        };
        let Some((x, y)) = world_origin(tree, id) else {
            return ClipState::Empty;
        };
        let mut clip = ClipState::Unclipped;
        if let Some(local_clip) = node.clip {
            let Some(transform) = world_transform(tree, id) else {
                return ClipState::Empty;
            };
            clip = clip.intersect(transform_rect(transform, local_clip.translated(x, y)));
        }
        if let Some(local_clip) = node.animation_clip {
            let Some(transform) = world_transform_before_animation(tree, id) else {
                return ClipState::Empty;
            };
            clip = clip.intersect(transform_rect(transform, local_clip.translated(x, y)));
        }
        clip
    }

    pub(super) fn inherited_clip(tree: &DrawCommandList, id: RenderNodeId) -> Option<ClipState> {
        let node = tree.node(id)?;
        let parent_clip = node.parent.map_or(Some(ClipState::Unclipped), |parent| {
            inherited_clip(tree, parent)
        })?;
        Some(parent_clip.intersect_state(world_clip(tree, id)))
    }

    pub(super) fn subtree_bounds(tree: &DrawCommandList, id: RenderNodeId) -> Option<Rect> {
        let node = tree.node(id)?;
        let mut bounds = world_bounds(tree, id)?;
        for child in node.children.iter().copied() {
            bounds = bounds.union(subtree_bounds(tree, child)?);
        }
        Some(bounds)
    }

    pub(super) fn visible_node_bounds(tree: &DrawCommandList, id: RenderNodeId) -> Option<Rect> {
        let clip = inherited_clip(tree, id)?;
        clip.intersect_bounds(world_bounds(tree, id)?)
    }

    pub(super) fn visible_subtree_bounds(tree: &DrawCommandList, id: RenderNodeId) -> Option<Rect> {
        let node = tree.node(id)?;
        let parent_clip = node.parent.map_or(Some(ClipState::Unclipped), |parent| {
            inherited_clip(tree, parent)
        })?;
        visible_subtree_bounds_with_clip(tree, id, parent_clip)
    }

    fn visible_subtree_bounds_with_clip(
        tree: &DrawCommandList,
        id: RenderNodeId,
        parent_clip: ClipState,
    ) -> Option<Rect> {
        let node = tree.node(id)?;
        let clip = parent_clip.intersect_state(world_clip(tree, id));
        let mut bounds = clip.intersect_bounds(world_bounds(tree, id)?);
        for child in node.children.iter().copied() {
            if let Some(child_bounds) = visible_subtree_bounds_with_clip(tree, child, clip) {
                bounds = Some(bounds.map_or(child_bounds, |bounds| bounds.union(child_bounds)));
            }
        }
        bounds
    }
}

/// A deterministic pseudo-random number source, so failures reproduce.
pub(super) struct Lcg(pub(super) u64);

impl Lcg {
    pub(super) fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    pub(super) fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (self.next() % 10_000) as f32 / 10_000.0 * (high - low)
    }

    pub(super) fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

pub(super) fn random_transform(rng: &mut Lcg) -> Mat3 {
    match rng.next() % 3 {
        0 => Mat3::identity(),
        1 => Mat3::translate(rng.range(-20.0, 20.0), rng.range(-20.0, 20.0)),
        _ => Mat3::translate(rng.range(-10.0, 10.0), rng.range(-10.0, 10.0))
            .mul(&Mat3::scale(rng.range(0.5, 2.0), rng.range(0.5, 2.0))),
    }
}

pub(super) fn random_clip(rng: &mut Lcg) -> Rect {
    Rect::new(
        rng.range(-10.0, 20.0),
        rng.range(-10.0, 20.0),
        rng.range(5.0, 120.0),
        rng.range(5.0, 120.0),
    )
}

/// A varied tree: nested nodes with offsets, transforms, presentation
/// transforms, clips and animation clips.
pub(super) fn varied_tree(seed: u64, nodes: usize) -> (RenderTree, Vec<RenderNodeId>) {
    let tree = RenderTree::new();
    let mut rng = Lcg(seed);
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 300.0)).unwrap();
    let mut ids = vec![root];
    for _ in 1..nodes {
        let parent = ids[(rng.next() as usize) % ids.len()];
        let bounds = Rect::new(
            rng.range(-30.0, 60.0),
            rng.range(-30.0, 60.0),
            rng.range(1.0, 150.0),
            rng.range(1.0, 150.0),
        );
        ids.push(tree.add_child(parent, bounds).unwrap());
    }
    {
        let mut inner = tree.draw_cmd.borrow_mut();
        for id in &ids {
            let node = inner.node_mut(*id).unwrap();
            if rng.chance(40) {
                node.transform = random_transform(&mut rng);
            }
            if rng.chance(30) {
                node.presentation_transform = random_transform(&mut rng);
            }
            if rng.chance(45) {
                node.clip = Some(random_clip(&mut rng));
            }
            if rng.chance(15) {
                node.animation_clip = Some(random_clip(&mut rng));
            }
        }
    }
    (tree, ids)
}

#[test]
fn world_queries_match_the_original_definitions_exactly() {
    for seed in [1, 7, 42, 2024, 99_999] {
        let (tree, ids) = varied_tree(seed, 80);
        let inner = tree.draw_cmd.borrow();
        for id in ids {
            assert_eq!(
                inner.world_bounds(id),
                reference::world_bounds(&inner, id),
                "world_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.subtree_bounds(id),
                reference::subtree_bounds(&inner, id),
                "subtree_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.visible_node_bounds(id),
                reference::visible_node_bounds(&inner, id),
                "visible_node_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.visible_subtree_bounds(id),
                reference::visible_subtree_bounds(&inner, id),
                "visible_subtree_bounds, seed {seed}, {id:?}"
            );
        }
    }
}

/// Asking for many nodes at once gives each the answer it would have been given
/// alone. A structural edit damages the footprint of every parent it touched,
/// and those subtrees nest: the answer for a node already holds the answers for
/// the nodes below it, so asking for them together reuses it.
#[test]
fn the_footprints_of_many_nodes_equal_asking_for_each_alone() {
    for seed in [3, 11, 58, 2025, 777_777] {
        for (tree, ids) in [varied_tree(seed, 90), untransformed_tree(seed, 120)] {
            let inner = tree.draw_cmd.borrow();
            // Parents and children together, out of order, one of them twice.
            let mut asked = ids.clone();
            asked.reverse();
            asked.push(ids[ids.len() / 2]);

            let together = inner.visible_subtree_bounds_of(&asked);
            let alone: Vec<Rect> = asked
                .iter()
                .filter_map(|id| inner.visible_subtree_bounds(*id))
                .collect();

            assert_eq!(together, alone, "seed {seed}");
        }
    }
}

/// A node that does not exist has no footprint, as when asked alone.
#[test]
fn a_node_that_is_not_in_the_tree_has_no_footprint_among_many() {
    let (tree, ids) = varied_tree(5, 20);
    let inner = tree.draw_cmd.borrow();
    let gone = RenderNodeId(u64::MAX);

    let together = inner.visible_subtree_bounds_of(&[ids[1], gone, ids[2]]);

    let alone: Vec<Rect> = [ids[1], ids[2]]
        .iter()
        .filter_map(|id| inner.visible_subtree_bounds(*id))
        .collect();
    assert_eq!(together, alone);
}

/// A tree like a real interface: almost no node carries a transform, a share
/// of them clip, and a few animate. The identity shortcut takes every node
/// that has no transform, so this is the shape that exercises it hardest.
pub(super) fn untransformed_tree(seed: u64, nodes: usize) -> (RenderTree, Vec<RenderNodeId>) {
    let tree = RenderTree::new();
    let mut rng = Lcg(seed);
    let root = tree.add_root(Rect::new(0.0, 0.0, 600.0, 400.0)).unwrap();
    let mut ids = vec![root];
    for _ in 1..nodes {
        // Deep nesting: bias toward the most recently added nodes.
        let recent = ids.len().saturating_sub(6);
        let parent = ids[recent + (rng.next() as usize) % (ids.len() - recent)];
        let bounds = Rect::new(
            rng.range(-20.0, 90.0),
            rng.range(-20.0, 90.0),
            rng.range(1.0, 200.0),
            rng.range(1.0, 200.0),
        );
        ids.push(tree.add_child(parent, bounds).unwrap());
    }
    {
        let mut inner = tree.draw_cmd.borrow_mut();
        for id in &ids {
            let node = inner.node_mut(*id).unwrap();
            if rng.chance(4) {
                node.transform = random_transform(&mut rng);
            }
            if rng.chance(3) {
                node.presentation_transform = random_transform(&mut rng);
            }
            if rng.chance(30) {
                node.clip = Some(random_clip(&mut rng));
            }
            if rng.chance(8) {
                node.animation_clip = Some(random_clip(&mut rng));
            }
        }
    }
    (tree, ids)
}

#[test]
fn world_queries_match_the_definitions_on_mostly_untransformed_trees() {
    for seed in [2, 13, 77, 1234, 424_242] {
        let (tree, ids) = untransformed_tree(seed, 120);
        let inner = tree.draw_cmd.borrow();
        for id in ids {
            assert_eq!(
                inner.world_bounds(id),
                reference::world_bounds(&inner, id),
                "world_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.visible_node_bounds(id),
                reference::visible_node_bounds(&inner, id),
                "visible_node_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.subtree_bounds(id),
                reference::subtree_bounds(&inner, id),
                "subtree_bounds, seed {seed}, {id:?}"
            );
            assert_eq!(
                inner.visible_subtree_bounds(id),
                reference::visible_subtree_bounds(&inner, id),
                "visible_subtree_bounds, seed {seed}, {id:?}"
            );
        }
    }
}

#[test]
fn an_untransformed_chain_keeps_the_exact_translation_of_its_origin() {
    // Translating to a node's origin and back must leave the inherited
    // transform exactly as it was, however far from zero the origin is.
    let tree = RenderTree::new();
    let mut id = tree.add_root(Rect::new(0.1, 0.3, 50.0, 50.0)).unwrap();
    for level in 0..40 {
        id = tree
            .add_child(
                id,
                Rect::new(777.7 + level as f32 * 0.37, 1234.1, 50.0, 50.0),
            )
            .unwrap();
    }
    let inner = tree.draw_cmd.borrow();
    assert_eq!(inner.world_bounds(id), reference::world_bounds(&inner, id));
    assert_eq!(
        inner.visible_node_bounds(id),
        reference::visible_node_bounds(&inner, id)
    );
}

/// A single chain `depth` deep, each node nested in the previous one.
fn chain(depth: usize) -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let mut id = tree.add_root(Rect::new(0.0, 0.0, 500.0, 500.0)).unwrap();
    for level in 0..depth {
        id = tree
            .add_child(id, Rect::new(1.0, 1.0, 400.0 - level as f32 * 0.5, 400.0))
            .unwrap();
    }
    {
        let mut inner = tree.draw_cmd.borrow_mut();
        let ids: Vec<_> = inner.nodes.iter().map(|node| node.id).collect();
        for id in ids {
            inner.node_mut(id).unwrap().clip = Some(Rect::new(0.0, 0.0, 450.0, 450.0));
        }
    }
    (tree, id)
}

fn lookups_for(depth: usize, query: impl Fn(&DrawCommandList, RenderNodeId)) -> usize {
    let (tree, leaf) = chain(depth);
    let inner = tree.draw_cmd.borrow();
    reset_node_lookups();
    query(&inner, leaf);
    node_lookups()
}

#[test]
fn a_visibility_query_costs_time_proportional_to_depth() {
    const DEPTH: usize = 150;
    // One walk to the root plus the node itself, with generous headroom. The
    // recursive definitions cost on the order of depth cubed (millions here).
    let budget = DEPTH * 8;

    let lookups = lookups_for(DEPTH, |tree, leaf| {
        let _ = tree.visible_subtree_bounds(leaf);
    });
    assert!(
        lookups <= budget,
        "visible_subtree_bounds took {lookups} lookups (budget {budget})"
    );

    let lookups = lookups_for(DEPTH, |tree, leaf| {
        let _ = tree.visible_node_bounds(leaf);
    });
    assert!(
        lookups <= budget,
        "visible_node_bounds took {lookups} lookups (budget {budget})"
    );

    let lookups = lookups_for(DEPTH, |tree, leaf| {
        let _ = tree.world_bounds(leaf);
    });
    assert!(
        lookups <= budget,
        "world_bounds took {lookups} lookups (budget {budget})"
    );
}

#[test]
fn a_query_resolves_its_ancestors_without_looking_each_one_up() {
    // The ancestors of a node are found from the structure the tree already
    // keeps, so the cost of a lookup does not grow with depth.
    const DEPTH: usize = 150;
    const BUDGET: usize = 8;

    for (what, lookups) in [
        (
            "world_bounds",
            lookups_for(DEPTH, |tree, leaf| {
                let _ = tree.world_bounds(leaf);
            }),
        ),
        (
            "visible_node_bounds",
            lookups_for(DEPTH, |tree, leaf| {
                let _ = tree.visible_node_bounds(leaf);
            }),
        ),
        (
            "local_visible_region",
            lookups_for(DEPTH, |tree, leaf| {
                let _ = tree.local_visible_region(leaf);
            }),
        ),
        (
            "visible_subtree_bounds",
            lookups_for(DEPTH, |tree, leaf| {
                let _ = tree.visible_subtree_bounds(leaf);
            }),
        ),
    ] {
        assert!(
            lookups <= BUDGET,
            "{what} took {lookups} node lookups for a chain {DEPTH} deep"
        );
    }
}

/// Compares every node of `ids` against the original definitions.
fn assert_matches_reference(tree: &RenderTree, ids: &[RenderNodeId], context: &str) {
    let inner = tree.draw_cmd.borrow();
    for id in ids {
        assert_eq!(
            inner.world_bounds(*id),
            reference::world_bounds(&inner, *id),
            "world_bounds {id:?} {context}"
        );
        assert_eq!(
            inner.visible_node_bounds(*id),
            reference::visible_node_bounds(&inner, *id),
            "visible_node_bounds {id:?} {context}"
        );
        assert_eq!(
            inner.subtree_bounds(*id),
            reference::subtree_bounds(&inner, *id),
            "subtree_bounds {id:?} {context}"
        );
        assert_eq!(
            inner.visible_subtree_bounds(*id),
            reference::visible_subtree_bounds(&inner, *id),
            "visible_subtree_bounds {id:?} {context}"
        );
    }
}

#[test]
fn every_edit_is_seen_by_the_next_query_even_with_warm_results() {
    for seed in [5, 31, 202, 9_001] {
        let (tree, ids) = untransformed_tree(seed, 60);
        let mut rng = Lcg(seed ^ 0xabcd);
        // Everything is asked once first, so any result kept from before an
        // edit is there to be wrongly served after it.
        assert_matches_reference(&tree, &ids, "initially");

        for step in 0..40 {
            let id = ids[(rng.next() as usize) % ids.len()];
            let what = match rng.next() % 6 {
                0 => {
                    let bounds = Rect::new(
                        rng.range(-30.0, 60.0),
                        rng.range(-30.0, 60.0),
                        rng.range(1.0, 150.0),
                        rng.range(1.0, 150.0),
                    );
                    let _ = tree.set_bounds(id, bounds);
                    "set_bounds"
                }
                1 => {
                    let _ = tree.set_transform(id, random_transform(&mut rng));
                    "set_transform"
                }
                2 => {
                    let clip = rng.chance(70).then(|| random_clip(&mut rng));
                    let _ = tree.set_clip(id, clip);
                    "set_clip"
                }
                3 => {
                    let bounds = Rect::new(
                        rng.range(-30.0, 60.0),
                        rng.range(-30.0, 60.0),
                        rng.range(1.0, 150.0),
                        rng.range(1.0, 150.0),
                    );
                    let clip = rng.chance(50).then(|| random_clip(&mut rng));
                    let _ = tree.set_geometry(id, bounds, clip);
                    "set_geometry"
                }
                4 => {
                    let clip = rng.chance(50).then(|| random_clip(&mut rng));
                    let _ =
                        tree.set_compositor_animation(id, random_transform(&mut rng), 1.0, clip);
                    "set_compositor_animation"
                }
                _ => {
                    let _ = tree.set_presentation(id, random_transform(&mut rng), 1.0);
                    "set_presentation"
                }
            };
            assert_matches_reference(
                &tree,
                &ids,
                &format!("seed {seed} step {step} after {what}"),
            );
        }
    }
}

#[test]
fn growing_and_restructuring_the_tree_is_seen_by_warm_results() {
    let (tree, mut ids) = untransformed_tree(77, 30);
    assert_matches_reference(&tree, &ids, "initially");

    let added = tree
        .add_child(ids[3], Rect::new(4.0, 5.0, 30.0, 30.0))
        .unwrap();
    ids.push(added);
    assert_matches_reference(&tree, &ids, "after add_child");

    let added = tree.add_root(Rect::new(9.0, 9.0, 50.0, 50.0)).unwrap();
    ids.push(added);
    assert_matches_reference(&tree, &ids, "after add_root");
}

#[test]
fn a_frame_is_derived_once_however_many_queries_share_it() {
    // A chain makes every node an ancestor of the next. Asking about each node
    // used to re-derive all of its ancestors, which is quadratic in depth.
    const DEPTH: usize = 300;
    let (tree, leaf) = chain(DEPTH);
    let inner = tree.draw_cmd.borrow();
    let ids = inner.nodes.iter().map(|node| node.id).collect::<Vec<_>>();

    reset_node_lookups();
    for id in ids.iter().rev() {
        let _ = inner.world_bounds(*id);
    }
    let _ = inner.visible_node_bounds(leaf);
    let _ = inner.local_visible_region(leaf);
    let derived = frame_derivations();

    assert!(
        derived <= 3 * DEPTH,
        "{derived} frames were derived for {} nodes",
        DEPTH + 1
    );
}

#[test]
fn an_edit_makes_the_next_query_derive_its_frames_again() {
    const DEPTH: usize = 40;
    let (tree, leaf) = chain(DEPTH);
    {
        let inner = tree.draw_cmd.borrow();
        let _ = inner.world_bounds(leaf);
        reset_node_lookups();
        let _ = inner.world_bounds(leaf);
        assert!(
            frame_derivations() <= 2,
            "a repeated query re-derived its ancestors"
        );
    }

    let root = tree.draw_cmd.borrow().roots[0];
    tree.set_bounds(root, Rect::new(3.0, 3.0, 500.0, 500.0))
        .unwrap();

    let inner = tree.draw_cmd.borrow();
    reset_node_lookups();
    let moved = inner.world_bounds(leaf);
    assert!(
        frame_derivations() >= DEPTH,
        "the edit did not invalidate the cached frames"
    );
    assert_eq!(moved, reference::world_bounds(&inner, leaf));
}

#[test]
fn queries_follow_the_structure_after_it_changes() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 300.0, 300.0)).unwrap();
    let left = tree
        .add_child(root, Rect::new(10.0, 10.0, 100.0, 100.0))
        .unwrap();
    let right = tree
        .add_child(root, Rect::new(150.0, 20.0, 100.0, 100.0))
        .unwrap();
    let leaf = tree
        .add_child(left, Rect::new(5.0, 5.0, 20.0, 20.0))
        .unwrap();
    let before = tree.draw_cmd.borrow().world_bounds(leaf).unwrap();
    assert_eq!((before.x, before.y), (15.0, 15.0));

    // Moving the leaf under the other branch must change its world position,
    // even though the tree answered a query about it a moment ago.
    tree.sync_structure(&[
        RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 300.0, 300.0)),
        RenderNodeSpec::new(Some(left), Some(0), Rect::new(10.0, 10.0, 100.0, 100.0)),
        RenderNodeSpec::new(Some(right), Some(0), Rect::new(150.0, 20.0, 100.0, 100.0)),
        RenderNodeSpec::new(Some(leaf), Some(2), Rect::new(5.0, 5.0, 20.0, 20.0)),
    ])
    .unwrap();

    let inner = tree.draw_cmd.borrow();
    let after = inner.world_bounds(leaf).unwrap();
    assert_eq!((after.x, after.y), (155.0, 25.0));
    assert_eq!(
        inner.world_bounds(leaf),
        reference::world_bounds(&inner, leaf)
    );
}

#[test]
fn a_node_added_after_a_query_is_found_by_the_next_one() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(5.0, 6.0, 100.0, 100.0)).unwrap();
    let _ = tree.draw_cmd.borrow().world_bounds(root);

    let child = tree
        .add_child(root, Rect::new(10.0, 20.0, 30.0, 40.0))
        .unwrap();
    let grandchild = tree
        .add_child(child, Rect::new(1.0, 2.0, 3.0, 4.0))
        .unwrap();

    let inner = tree.draw_cmd.borrow();
    let bounds = inner.world_bounds(grandchild).unwrap();
    assert_eq!((bounds.x, bounds.y), (16.0, 28.0));
    assert_eq!(
        inner.world_bounds(grandchild),
        reference::world_bounds(&inner, grandchild)
    );
}

#[test]
fn querying_a_whole_subtree_does_not_multiply_by_depth() {
    // From the root of a chain every node is visited once, so the cost is the
    // node count, not node count times depth.
    const DEPTH: usize = 150;
    let (tree, _) = chain(DEPTH);
    let inner = tree.draw_cmd.borrow();
    let root = inner.roots[0];
    reset_node_lookups();
    let _ = inner.visible_subtree_bounds(root);
    let lookups = node_lookups();
    assert!(
        lookups <= (DEPTH + 1) * 8,
        "visible_subtree_bounds(root) took {lookups} lookups for {} nodes",
        DEPTH + 1
    );
}

/// A scrollable's content node: its bounds cover the viewport clip, and it owns
/// `rows` descendants that are all clipped to that viewport.
fn viewport_with_rows(rows: usize, content_height: f32) -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, content_height))
        .unwrap();
    for row in 0..rows {
        tree.add_child(content, Rect::new(0.0, row as f32 * 10.0, 100.0, 10.0))
            .unwrap();
    }
    tree.draw_cmd.borrow_mut().node_mut(content).unwrap().clip =
        Some(Rect::new(0.0, 0.0, 100.0, 100.0));
    (tree, content)
}

#[test]
fn a_clip_that_the_node_covers_bounds_the_subtree_without_visiting_it() {
    const ROWS: usize = 500;
    let (tree, content) = viewport_with_rows(ROWS, 5000.0);
    let inner = tree.draw_cmd.borrow();

    reset_node_lookups();
    let bounds = inner.visible_subtree_bounds(content);
    let lookups = node_lookups();

    assert_eq!(bounds, Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    assert!(
        lookups < ROWS / 10,
        "visible_subtree_bounds took {lookups} lookups for {ROWS} clipped descendants"
    );
}

#[test]
fn a_clip_that_the_node_does_not_cover_still_measures_its_descendants() {
    // The content is shorter than the viewport, so the clip alone would
    // overstate the footprint and the descendants have to be measured.
    let (tree, content) = viewport_with_rows(3, 30.0);
    let inner = tree.draw_cmd.borrow();

    assert_eq!(
        inner.visible_subtree_bounds(content),
        reference::visible_subtree_bounds(&inner, content)
    );
    assert_eq!(
        inner.visible_subtree_bounds(content),
        Some(Rect::new(0.0, 0.0, 100.0, 30.0))
    );
}

#[test]
fn the_covered_clip_shortcut_matches_the_reference_on_varied_trees() {
    // Cover the clip of every clipped node so the shortcut is exercised on the
    // transforms, nested clips and animation clips `varied_tree` produces.
    for seed in [3, 11, 58, 777, 31_415] {
        let (tree, ids) = varied_tree(seed, 80);
        {
            let mut inner = tree.draw_cmd.borrow_mut();
            for id in &ids {
                let node = inner.node_mut(*id).unwrap();
                if let Some(clip) = node.clip {
                    node.bounds = Rect::new(
                        node.bounds.x,
                        node.bounds.y,
                        clip.x + clip.width + 200.0,
                        clip.y + clip.height + 200.0,
                    );
                }
            }
        }
        let inner = tree.draw_cmd.borrow();
        for id in ids {
            assert_eq!(
                inner.visible_subtree_bounds(id),
                reference::visible_subtree_bounds(&inner, id),
                "visible_subtree_bounds, seed {seed}, {id:?}"
            );
        }
    }
}

/// Callers cache a node's world rectangle against the tree's geometry revision,
/// so every mutation that can move a node has to advance it, and a pure read
/// must not.
mod geometry_revision {
    use super::*;

    fn tree_with_child() -> (RenderTree, RenderNodeId, RenderNodeId) {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(10.0, 10.0, 20.0, 20.0))
            .unwrap();
        (tree, root, child)
    }

    fn advanced_by(tree: &RenderTree, mutate: impl FnOnce()) -> bool {
        let before = tree.geometry_revision();
        mutate();
        tree.geometry_revision() != before
    }

    #[test]
    fn reads_do_not_advance_it() {
        let (tree, _root, child) = tree_with_child();
        assert!(!advanced_by(&tree, || {
            let _ = tree.element_bounds(child);
            let _ = tree.parent_of(child);
        }));
    }

    #[test]
    fn adding_a_node_advances_it() {
        let (tree, root, _child) = tree_with_child();
        assert!(advanced_by(&tree, || {
            tree.add_child(root, Rect::new(0.0, 0.0, 5.0, 5.0)).unwrap();
        }));
        assert!(advanced_by(&tree, || {
            tree.add_root(Rect::new(0.0, 0.0, 5.0, 5.0)).unwrap();
        }));
    }

    #[test]
    fn every_geometry_mutator_advances_it() {
        let (tree, root, child) = tree_with_child();
        assert!(advanced_by(&tree, || {
            tree.set_bounds(child, Rect::new(11.0, 10.0, 20.0, 20.0))
                .unwrap();
        }));
        assert!(advanced_by(&tree, || {
            tree.set_geometry(child, Rect::new(12.0, 10.0, 20.0, 20.0), None)
                .unwrap();
        }));
        assert!(advanced_by(&tree, || {
            tree.set_transform(child, Mat3::translate(3.0, 0.0))
                .unwrap();
        }));
        assert!(advanced_by(&tree, || {
            tree.set_compositor_animation(child, Mat3::translate(4.0, 0.0), 1.0, None)
                .unwrap();
        }));
        assert!(advanced_by(&tree, || {
            let specs = [
                RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 100.0, 100.0)),
                RenderNodeSpec::new(Some(child), Some(0), Rect::new(30.0, 10.0, 20.0, 20.0)),
            ];
            tree.sync_structure(&specs).unwrap();
        }));
    }

    #[test]
    fn a_moved_ancestor_is_visible_through_the_revision() {
        let (tree, root, child) = tree_with_child();
        let before = tree.element_bounds(child).unwrap();

        let revision = tree.geometry_revision();
        tree.set_bounds(root, Rect::new(50.0, 0.0, 100.0, 100.0))
            .unwrap();

        assert_ne!(tree.geometry_revision(), revision);
        assert_eq!(tree.element_bounds(child).unwrap().x, before.x + 50.0);
    }
}
