//! Damage recorded by a geometry-only synchronization.
//!
//! A window resize changes the bounds of nearly every node at once. The damage
//! it records must cover every pixel that moved, and it must cost time
//! proportional to the tree, not to the sum of every node's subtree size: a
//! deep interface pays that sum twice per node, every step of a resize drag.

use super::world_tests::Lcg;
use super::*;

/// Whether `outer` contains `inner`, allowing for float rounding.
fn contains(outer: Rect, inner: Rect) -> bool {
    const EPSILON: f32 = 1e-3;
    outer.x <= inner.x + EPSILON
        && outer.y <= inner.y + EPSILON
        && outer.x + outer.width + EPSILON >= inner.x + inner.width
        && outer.y + outer.height + EPSILON >= inner.y + inner.height
}

fn chain_specs(ids: &[Option<RenderNodeId>], grow: f32) -> Vec<RenderNodeSpec> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            RenderNodeSpec::new(
                *id,
                index.checked_sub(1),
                Rect::new(1.0, 1.0, 400.0 + grow - index as f32 * 0.5, 400.0 + grow),
            )
        })
        .collect()
}

#[test]
fn a_geometry_change_to_every_node_visits_each_subtree_once() {
    // Every node of a chain contains the next, so damaging each changed node
    // by walking its own subtree visits depth² nodes.
    const DEPTH: usize = 200;
    let tree = RenderTree::new();
    let ids = tree
        .sync_structure(&chain_specs(&vec![None; DEPTH], 0.0))
        .unwrap();
    let ids = ids.into_iter().map(Some).collect::<Vec<_>>();
    let _ = tree.take_damage();

    reset_node_lookups();
    tree.sync_structure(&chain_specs(&ids, 8.0)).unwrap();
    let derived = frame_derivations();

    assert!(
        derived <= 12 * DEPTH,
        "{derived} frames were derived to damage {DEPTH} nodes"
    );
    assert!(
        !tree.take_damage().is_empty(),
        "the resize recorded no damage"
    );
}

#[test]
fn damage_covers_every_node_that_moved_even_when_ancestors_and_descendants_change() {
    for seed in [3_u64, 29, 311, 7_001, 90_210] {
        let mut rng = Lcg(seed);
        let count = 60;
        let mut specs = Vec::with_capacity(count);
        let mut clips = Vec::with_capacity(count);
        for index in 0..count {
            // Bias toward recent nodes for depth, as the world tests do.
            let parent = (index > 0).then(|| {
                let recent = index.saturating_sub(5);
                recent + (rng.next() as usize) % (index - recent)
            });
            specs.push(RenderNodeSpec::new(
                None,
                parent,
                Rect::new(
                    rng.range(-10.0, 60.0),
                    rng.range(-10.0, 60.0),
                    rng.range(20.0, 300.0),
                    rng.range(20.0, 300.0),
                ),
            ));
            clips.push(
                rng.chance(30)
                    .then(|| Rect::new(0.0, 0.0, rng.range(50.0, 400.0), 300.0)),
            );
        }
        let tree = RenderTree::new();
        let ids = tree.sync_structure_with_clips(&specs, &clips).unwrap();
        let _ = tree.take_damage();

        let before: Vec<_> = {
            let inner = tree.draw_cmd.borrow();
            ids.iter()
                .map(|id| inner.visible_node_bounds(*id))
                .collect()
        };

        for (index, spec) in specs.iter_mut().enumerate() {
            spec.existing_id = Some(ids[index]);
            if rng.chance(45) {
                spec.bounds = Rect::new(
                    spec.bounds.x + rng.range(-15.0, 15.0),
                    spec.bounds.y + rng.range(-15.0, 15.0),
                    spec.bounds.width + rng.range(-10.0, 40.0),
                    spec.bounds.height + rng.range(-10.0, 40.0),
                );
            }
            if rng.chance(10) {
                clips[index] = Some(Rect::new(0.0, 0.0, rng.range(50.0, 400.0), 250.0));
            }
        }
        tree.sync_structure_with_clips(&specs, &clips).unwrap();
        let damage = tree.take_damage();

        let inner = tree.draw_cmd.borrow();
        for (index, id) in ids.iter().enumerate() {
            let after = inner.visible_node_bounds(*id);
            if after == before[index] {
                continue;
            }
            for rect in before[index].into_iter().chain(after) {
                assert!(
                    damage.iter().any(|damaged| contains(*damaged, rect)),
                    "seed {seed}: node {index} moved from {:?} to {after:?}, \
                     but {rect:?} is outside the damage {damage:?}",
                    before[index],
                );
            }
        }
    }
}
