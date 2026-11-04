//! Damage for a moved subtree.
//!
//! A page that slides or fades in is one node with thousands below it, and its
//! presentation changes on every frame of the animation. The damage it records
//! must cover every pixel the page leaves and reaches, and it must not cost a
//! walk of the page for each of the three questions asked about it.

use super::subtree_extent::{extent_computations, reset_extent_computations};
use super::world_tests::{Lcg, random_clip, random_transform, untransformed_tree, varied_tree};
use super::*;

const EPSILON: f32 = 1e-2;

/// Whether `outer` contains `inner`, allowing for float rounding.
fn contains(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x + EPSILON
        && outer.y <= inner.y + EPSILON
        && outer.x + outer.width + EPSILON >= inner.x + inner.width
        && outer.y + outer.height + EPSILON >= inner.y + inner.height
}

fn same(left: Rect, right: Rect) -> bool {
    contains(left, right) && contains(right, left)
}

/// Nested nodes with offsets, scales and translations but no clips: the case
/// where a footprint can be known exactly, not just covered.
fn plain_tree(seed: u64, nodes: usize) -> (RenderTree, Vec<RenderNodeId>) {
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
    for id in &ids {
        if rng.chance(40) {
            tree.set_transform(*id, random_transform(&mut rng)).unwrap();
        }
        if rng.chance(30) {
            tree.set_presentation(*id, random_transform(&mut rng), 1.0)
                .unwrap();
        }
    }
    (tree, ids)
}

/// A page-shaped tree: a root, `groups` containers below it, and `leaves` in
/// each. Returns the root and the last leaf, the deepest edit target.
fn page(groups: usize, leaves: usize) -> (RenderTree, RenderNodeId, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 800.0, 700.0)).unwrap();
    let mut last = root;
    for group in 0..groups {
        let container = tree
            .add_child(root, Rect::new(0.0, group as f32 * 20.0, 800.0, 20.0))
            .unwrap();
        for leaf in 0..leaves {
            last = tree
                .add_child(container, Rect::new(leaf as f32 * 10.0, 0.0, 10.0, 20.0))
                .unwrap();
        }
    }
    (tree, root, last)
}

fn assert_covers_the_exact_bounds(tree: &RenderTree, ids: &[RenderNodeId], context: &str) {
    let inner = tree.draw_cmd.borrow();
    for id in ids {
        let Some(exact) = inner.visible_subtree_bounds(*id) else {
            continue;
        };
        let footprint = inner.damage_footprint(*id);
        assert!(
            footprint.is_some_and(|footprint| contains(footprint, exact)),
            "footprint {footprint:?} does not cover {exact:?}, {id:?} {context}"
        );
        assert_eq!(
            inner.subtree_is_finite(*id),
            inner.subtree_bounds(*id).is_some(),
            "finiteness, {id:?} {context}"
        );
    }
}

#[test]
fn the_footprint_is_the_exact_bounds_when_nothing_clips() {
    for seed in [2, 9, 64, 1_001, 77_777] {
        let (tree, ids) = plain_tree(seed, 80);
        let inner = tree.draw_cmd.borrow();
        for id in ids {
            let exact = inner.visible_subtree_bounds(id);
            let footprint = inner.damage_footprint(id);
            match (footprint, exact) {
                (Some(footprint), Some(exact)) => assert!(
                    same(footprint, exact),
                    "{footprint:?} is not {exact:?}, seed {seed}, {id:?}"
                ),
                (footprint, exact) => {
                    assert_eq!(footprint, exact, "seed {seed}, {id:?}")
                }
            }
        }
    }
}

#[test]
fn the_footprint_covers_the_exact_bounds_of_clipped_trees() {
    for seed in [4, 19, 303, 4_242] {
        let (tree, ids) = varied_tree(seed, 80);
        assert_covers_the_exact_bounds(&tree, &ids, &format!("varied seed {seed}"));
        let (tree, ids) = untransformed_tree(seed, 80);
        assert_covers_the_exact_bounds(&tree, &ids, &format!("untransformed seed {seed}"));
    }
}

#[test]
fn every_edit_is_seen_by_the_next_footprint_even_with_warm_results() {
    for seed in [6, 28, 313, 8_008] {
        let (tree, ids) = untransformed_tree(seed, 60);
        let mut rng = Lcg(seed ^ 0x5eed);
        // Everything is asked once first, so a footprint kept from before an
        // edit is there to be wrongly served after it.
        assert_covers_the_exact_bounds(&tree, &ids, "initially");

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
            assert_covers_the_exact_bounds(
                &tree,
                &ids,
                &format!("seed {seed} step {step} after {what}"),
            );
        }
    }
}

#[test]
fn a_node_added_after_a_footprint_is_inside_the_next_one() {
    let (tree, ids) = plain_tree(12, 30);
    assert_covers_the_exact_bounds(&tree, &ids, "initially");
    let added = tree
        .add_child(ids[0], Rect::new(500.0, 500.0, 40.0, 40.0))
        .unwrap();
    let footprint = tree.draw_cmd.borrow().damage_footprint(ids[0]).unwrap();
    let added_bounds = tree.draw_cmd.borrow().visible_node_bounds(added).unwrap();
    assert!(contains(footprint, added_bounds), "{footprint:?} misses {added_bounds:?}");
}

#[test]
fn a_node_that_escapes_clipping_is_still_covered() {
    let (tree, ids) = untransformed_tree(55, 40);
    {
        let mut inner = tree.draw_cmd.borrow_mut();
        let escaping = ids[ids.len() / 2];
        inner.node_mut(escaping).unwrap().escapes_clip = true;
        inner.clip_escape_count += 1;
    }
    assert_covers_the_exact_bounds(&tree, &ids, "with an escaping node");
}

#[test]
fn presenting_a_page_records_damage_for_where_it_was_and_where_it_goes() {
    let (tree, root, _) = page(6, 5);
    let _ = tree.take_damage();
    tree.set_presentation(root, Mat3::translate(100.0, 0.0), 1.0)
        .unwrap();
    let damage = tree.take_damage();
    let damage = damage.into_iter().reduce(Rect::union).expect("damage");
    assert!(contains(damage, Rect::new(0.0, 0.0, 800.0, 700.0)), "{damage:?}");
    assert!(contains(damage, Rect::new(100.0, 0.0, 800.0, 700.0)), "{damage:?}");
}

#[test]
fn a_transform_that_overflows_the_subtree_is_refused_and_leaves_it_as_it_was() {
    let (tree, root, last) = page(3, 3);
    tree.set_transform(last, Mat3::scale(1.0e30, 1.0e30)).unwrap();
    let before = tree.draw_cmd.borrow().subtree_bounds(root);
    assert_eq!(
        tree.set_presentation(root, Mat3::scale(1.0e30, 1.0e30), 1.0),
        Err(RenderTreeError::InvalidTransform)
    );
    assert_eq!(tree.draw_cmd.borrow().subtree_bounds(root), before);
    assert!(tree.draw_cmd.borrow().subtree_is_finite(root));
}

#[test]
fn presenting_a_large_page_does_not_walk_it() {
    const PRESENTATIONS: usize = 20;
    let (tree, root, _) = page(30, 50);
    let nodes = tree.draw_cmd.borrow().nodes.len();
    // The first presentation pays for knowing the page; a frame of an animation
    // is every one after it.
    tree.set_presentation(root, Mat3::translate(1.0, 0.0), 1.0)
        .unwrap();

    reset_node_lookups();
    for step in 0..PRESENTATIONS {
        tree.set_presentation(root, Mat3::translate(2.0 + step as f32, 0.0), 0.5)
            .unwrap();
    }
    let derived = frame_derivations();

    assert!(
        derived <= PRESENTATIONS * 16,
        "{derived} frames were derived for {PRESENTATIONS} presentations of {nodes} nodes"
    );
}

#[test]
fn an_edit_below_a_page_measures_only_the_path_up_to_it() {
    let (tree, root, leaf) = page(30, 50);
    let nodes = tree.draw_cmd.borrow().nodes.len();
    assert!(tree.draw_cmd.borrow().damage_footprint(root).is_some());

    tree.set_bounds(leaf, Rect::new(3.0, 0.0, 400.0, 20.0)).unwrap();
    reset_extent_computations();
    let footprint = tree.draw_cmd.borrow().damage_footprint(root).unwrap();
    let measured = extent_computations();

    assert!(measured >= 1, "the changed path was not measured again");
    assert!(
        measured <= 3,
        "{measured} of {nodes} nodes were measured again after one leaf changed"
    );
    let exact = tree.draw_cmd.borrow().visible_subtree_bounds(root).unwrap();
    assert!(same(footprint, exact), "{footprint:?} is not {exact:?}");
}

#[test]
fn presenting_a_nested_page_follows_the_inner_presentation() {
    let (tree, root, _) = page(4, 4);
    let inner_container = tree.draw_cmd.borrow().nodes[1].id;
    tree.set_presentation(root, Mat3::translate(10.0, 0.0), 1.0)
        .unwrap();
    // Warm, then move only a container inside the page.
    assert!(tree.draw_cmd.borrow().damage_footprint(root).is_some());
    tree.set_presentation(inner_container, Mat3::translate(900.0, 0.0), 1.0)
        .unwrap();

    let inner = tree.draw_cmd.borrow();
    let footprint = inner.damage_footprint(root).unwrap();
    let exact = inner.visible_subtree_bounds(root).unwrap();
    assert!(same(footprint, exact), "{footprint:?} is not {exact:?}");
    assert!(footprint.x + footprint.width > 900.0, "{footprint:?}");
}
