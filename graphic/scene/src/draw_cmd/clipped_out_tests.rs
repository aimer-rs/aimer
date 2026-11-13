//! A node that is clipped out entirely hands its descendants nothing to draw, so
//! a plan does not derive them.
//!
//! A scroll moves one node, the content, and the plan derives everything below
//! it. Most of that is rows outside the viewport, each of which clips itself, so
//! each row's own clip meets the viewport's and comes out empty: nothing under
//! such a row can be seen, and deriving it every frame is work for no pixel. The
//! plan leaves it alone, unless something below the row escapes clipping, in
//! which case that something is still drawn.
//!
//! What these tests hold the plan to is that leaving it out changes nothing that
//! can be seen: the plan must equal the one a full derivation makes, for a scroll,
//! for edits made *under* a node that is clipped out, for a node that comes back
//! into view, and for random edits of every kind.

use super::render_order::set_prune_clipped_out;
use super::*;

/// Every node's position in creation order: two trees built by the same
/// operations name the same node differently, but give it the same position.
fn labels(tree: &RenderTree) -> std::collections::HashMap<RenderNodeId, usize> {
    tree.draw_cmd
        .borrow()
        .nodes
        .iter()
        .enumerate()
        .map(|(position, node)| (node.id, position))
        .collect()
}

fn describe(operation: &RenderOp, labels: &std::collections::HashMap<RenderNodeId, usize>) -> String {
    let label = |id: &RenderNodeId| labels.get(id).copied().map_or("?".to_owned(), |n| n.to_string());
    match operation {
        RenderOp::Draw(item) => format!(
            "draw #{} {:?} {:?} {:?} {:?} {:?} {} {:?}",
            label(&item.element),
            item.bounds,
            item.origin,
            item.transform,
            item.clip,
            item.clip_radius,
            item.opacity,
            item.paint_source,
        ),
        RenderOp::BeginOpacityGroup { element, bounds, opacity, clip, masked, .. } => {
            format!("begin #{} {bounds:?} {opacity} {clip:?} {masked}", label(element))
        }
        RenderOp::EndOpacityGroup { element } => format!("end #{}", label(element)),
    }
}

/// The plan of `tree` as it is now, and how many nodes deriving it touched.
fn plan(tree: &RenderTree, prune: bool) -> (Vec<String>, usize) {
    set_prune_clipped_out(prune);
    reset_node_lookups();
    let operations = tree.render_all();
    let derived = plan_nodes_derived();
    set_prune_clipped_out(true);
    let labels = labels(tree);
    (operations.iter().map(|op| describe(op, &labels)).collect(), derived)
}

const ROW_HEIGHT: f32 = 40.0;
const VIEWPORT_HEIGHT: f32 = 120.0;

/// A viewport 120 high over `rows` rows of 40, each row clipping itself and
/// holding a chain of `depth` nodes, scrolled to the top.
///
/// root -> content -> [row -> chain, ...], with the content placed and clipped the
/// way a scrollable places its content: moved up by the offset, clipped to the
/// viewport window in its own coordinates.
fn scroll_scene(rows: usize, depth: usize) -> (RenderTree, RenderNodeId, Vec<RenderNodeId>) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 300.0, VIEWPORT_HEIGHT)).unwrap();
    let content = tree
        .add_child(root, Rect::new(0.0, 0.0, 300.0, rows as f32 * ROW_HEIGHT))
        .unwrap();
    let mut all = vec![root, content];
    for row in 0..rows {
        let row_node = tree
            .add_child(content, Rect::new(0.0, row as f32 * ROW_HEIGHT, 300.0, ROW_HEIGHT))
            .unwrap();
        tree.set_clip(row_node, Some(Rect::new(0.0, 0.0, 300.0, ROW_HEIGHT))).unwrap();
        all.push(row_node);
        let mut parent = row_node;
        for _ in 0..depth {
            parent = tree.add_child(parent, Rect::new(2.0, 2.0, 200.0, 30.0)).unwrap();
            all.push(parent);
        }
    }
    scroll_to(&tree, content, rows, 0.0);
    (tree, content, all)
}

fn scroll_to(tree: &RenderTree, content: RenderNodeId, rows: usize, offset: f32) {
    tree.set_geometry(
        content,
        Rect::new(0.0, -offset, 300.0, rows as f32 * ROW_HEIGHT),
        Some(Rect::new(0.0, offset, 300.0, VIEWPORT_HEIGHT)),
    )
    .unwrap();
}

#[test]
fn scrolling_does_not_derive_what_is_below_rows_that_are_out_of_view() {
    let (tree, content, all) = scroll_scene(100, 8);
    assert_eq!(all.len(), 2 + 100 * 9);
    tree.render_all();

    scroll_to(&tree, content, 100, 40.0);
    let (_, derived) = plan(&tree, true);

    // The content, every row (each is judged by its own clip), and the chains of
    // the few rows the viewport shows; not the chains of the ninety-odd others.
    assert!(
        derived <= 1 + 100 + 4 * 8,
        "derived {derived} of {} nodes",
        all.len() - 1
    );
    // With pruning off the same scroll derives the content's whole subtree.
    scroll_to(&tree, content, 100, 80.0);
    let (_, derived_in_full) = plan(&tree, false);
    assert_eq!(derived_in_full, all.len() - 1);
}

#[test]
fn the_plan_of_a_scroll_is_the_plan_a_full_derivation_makes() {
    let (pruned, pruned_content, _) = scroll_scene(60, 6);
    let (full, full_content, _) = scroll_scene(60, 6);
    for offset in [0.0, 37.0, 400.0, 1234.5, 2279.0, 2280.0, 400.0, 0.0, 39.0, 1.0] {
        scroll_to(&pruned, pruned_content, 60, offset);
        scroll_to(&full, full_content, 60, offset);
        let (with_pruning, _) = plan(&pruned, true);
        let (without, _) = plan(&full, false);
        assert!(!with_pruning.is_empty(), "offset {offset} draws something");
        assert_eq!(with_pruning, without, "offset {offset}");
    }
}

#[test]
fn a_row_that_comes_back_into_view_is_derived_again_from_what_it_is_now() {
    let (tree, content, all) = scroll_scene(30, 5);
    let (reference, reference_content, reference_all) = scroll_scene(30, 5);
    let deep = |all: &[RenderNodeId]| all[2 + 5 * 6 + 5]; // the end of row 5's chain, 240 down

    scroll_to(&tree, content, 30, 800.0);
    scroll_to(&reference, reference_content, 30, 800.0);
    plan(&tree, true);
    plan(&reference, false);

    // While row 5 is clipped out its chain is edited: the edit is not seen, and
    // nothing about it must leak into what is drawn.
    for tree_and_ids in [(&tree, &all), (&reference, &reference_all)] {
        tree_and_ids
            .0
            .set_bounds(deep(tree_and_ids.1), Rect::new(7.0, 9.0, 60.0, 20.0))
            .unwrap();
    }
    assert_eq!(plan(&tree, true).0, plan(&reference, false).0, "edited while clipped out");

    // Scrolling row 5 back into view derives it, and the edit shows.
    scroll_to(&tree, content, 30, 200.0);
    scroll_to(&reference, reference_content, 30, 200.0);
    let (back, _) = plan(&tree, true);
    assert_eq!(back, plan(&reference, false).0, "back in view");
    assert!(
        // World coordinates: the chain's five offsets of 2, then the edit's (7, 9).
        back.iter().any(|operation| operation.contains("Rect { x: 15.0, y: 17.0, width: 60.0, height: 20.0 }")),
        "the edit made while the row was out of view is drawn once it is back"
    );
}

#[test]
fn an_edit_under_a_node_that_is_clipped_out_does_not_make_it_visible() {
    let (tree, content, all) = scroll_scene(30, 5);
    scroll_to(&tree, content, 30, 800.0);
    let before = plan(&tree, true).0;

    // Every node of a row far out of view, edited one after another.
    for &node in &all[2 + 5 * 6..2 + 5 * 6 + 6] {
        tree.set_bounds(node, Rect::new(1.0, 1.0, 90.0, 25.0)).unwrap();
        tree.set_opacity(node, 0.5).unwrap();
        let (after, derived) = plan(&tree, true);
        assert_eq!(after, before, "an edit nobody can see changes nothing drawn");
        assert!(derived <= 1, "the edit is under a clipped-out row, so it derives nothing: {derived}");
    }
}

#[test]
fn something_below_a_clipped_out_row_that_escapes_clipping_is_still_drawn() {
    let (tree, content, all) = scroll_scene(30, 5);
    let (reference, reference_content, reference_all) = scroll_scene(30, 5);
    scroll_to(&tree, content, 30, 800.0);
    scroll_to(&reference, reference_content, 30, 800.0);
    // The end of row 5's chain, 240 down and far out of view, draws regardless.
    tree.set_clip_escape(all[2 + 5 * 6 + 5], true).unwrap();
    reference.set_clip_escape(reference_all[2 + 5 * 6 + 5], true).unwrap();

    let (with_pruning, derived) = plan(&tree, true);
    let (without, derived_in_full) = plan(&reference, false);

    assert_eq!(with_pruning, without);
    assert!(
        with_pruning.iter().any(|operation| operation.starts_with(&format!("draw #{}", 2 + 5 * 6 + 5))),
        "the escaping node is drawn although its row is out of view"
    );
    // Row 5 is not pruned, the other rows are.
    assert!(derived > 1 + 30 && derived < derived_in_full, "{derived} of {derived_in_full}");
}

/// One random edit, the same on every tree it is applied to.
fn apply_edit(tree: &RenderTree, ids: &[RenderNodeId], mut rng: super::world_tests::Lcg) -> String {
    use super::world_tests::{random_clip, random_transform};
    let which = (rng.next() as usize) % ids.len();
    let id = ids[which];
    let kind = rng.next() % 11;
    let name = ["set_bounds", "set_transform", "set_clip", "set_opacity", "invalidate_paint", "set_compositor_animation", "add_child", "set_clip_escape", "set_clip_escape", "set_geometry", "set_geometry"][kind as usize];
    match kind {
        0 => {
            let _ = tree.set_bounds(
                id,
                Rect::new(rng.range(-30.0, 60.0), rng.range(-30.0, 60.0), rng.range(1.0, 150.0), rng.range(1.0, 150.0)),
            );
        }
        1 => {
            let _ = tree.set_transform(id, random_transform(&mut rng));
        }
        2 => {
            let clip = rng.chance(60).then(|| random_clip(&mut rng));
            let _ = tree.set_clip(id, clip);
        }
        3 => {
            let _ = tree.set_opacity(id, rng.range(0.0, 1.0));
        }
        4 => {
            let _ = tree.invalidate_paint(id);
        }
        5 => {
            let _ = tree.set_compositor_animation(
                id,
                random_transform(&mut rng),
                rng.range(0.0, 1.0),
                rng.chance(40).then(|| random_clip(&mut rng)),
            );
        }
        6 => {
            let _ = tree.add_child(id, Rect::new(1.0, 2.0, 30.0, 30.0));
        }
        7 | 8 => {
            // Escapes come and go, on few nodes so most clipped-out ones have none below.
            let _ = tree.set_clip_escape(id, rng.chance(35));
        }
        _ => {
            let _ = tree.set_geometry(
                id,
                Rect::new(rng.range(0.0, 50.0), rng.range(0.0, 50.0), 40.0, 40.0),
                rng.chance(70).then(|| random_clip(&mut rng)),
            );
        }
    }
    format!("{name} on node {which}")
}

/// After any sequence of edits the pruned plan must be the plan a full derivation
/// makes, and for the trees these make it must actually leave something out.
#[test]
fn a_pruned_plan_always_equals_a_plan_derived_in_full() {
    let (mut pruned_total, mut full_total, mut steps_that_pruned) = (0usize, 0usize, 0usize);
    // `CLIPPED_OUT_SEEDS=<n>` widens the sweep to the first `n` seeds, with trees of
    // several sizes: worth a run after touching how a plan derives or prunes.
    let seeds: Vec<u64> = match std::env::var("CLIPPED_OUT_SEEDS").ok().and_then(|n| n.parse().ok()) {
        Some(count) => (0..count).collect(),
        None => vec![3, 19, 144, 2_718, 31_415, 9_001],
    };
    for seed in seeds {
        let size = 20 + (seed as usize % 7) * 25;
        let (pruned, pruned_ids) = super::world_tests::untransformed_tree(seed, size);
        let (full, full_ids) = super::world_tests::untransformed_tree(seed, size);
        let mut rng = super::world_tests::Lcg(seed ^ 0xc11b);
        assert_eq!(plan(&pruned, true).0, plan(&full, false).0, "seed {seed}: the first plan");

        for step in 0..120 {
            let step_seed = rng.next();
            let edit = apply_edit(&pruned, &pruned_ids, super::world_tests::Lcg(step_seed));
            apply_edit(&full, &full_ids, super::world_tests::Lcg(step_seed));
            let (with_pruning, derived) = plan(&pruned, true);
            let (without, derived_in_full) = plan(&full, false);
            if with_pruning != without {
                let at = with_pruning.iter().zip(&without).position(|(a, b)| a != b).unwrap_or(with_pruning.len().min(without.len()));
                panic!(
                    "seed {seed} step {step} after {edit}: plans differ at operation {at} of {} / {}\n  pruned: {:?}\n  full:   {:?}",
                    with_pruning.len(), without.len(), with_pruning.get(at), without.get(at)
                );
            }
            assert!(derived <= derived_in_full, "seed {seed} step {step}: pruning derived more");
            pruned_total += derived;
            full_total += derived_in_full;
            steps_that_pruned += usize::from(derived < derived_in_full);
        }
    }
    assert!(
        steps_that_pruned >= 10 && pruned_total < full_total,
        "the test has power only if pruning happens: {steps_that_pruned} steps, {pruned_total} < {full_total}"
    );
}
