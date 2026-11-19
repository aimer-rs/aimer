//! Which of the nodes a geometry synchronization changed are the outermost: the ones with no changed
//! node above them, whose subtrees are the only ones to damage.
//!
//! A scroll under embedded widgets changes the clip of every node below each of them, so the set is
//! hundreds of nodes, and finding them each one's ancestors through lookups cost the number of nodes
//! times the depth.


use super::world_tests::Lcg;
use super::*;

fn bounds() -> Rect {
    Rect::new(0.0, 0.0, 30.0, 30.0)
}

/// A tree of `count` nodes whose parents are picked at random among the nodes before them, or none.
fn random_tree(seed: u64, count: usize, roots: u64) -> (RenderTree, Vec<RenderNodeId>) {
    let mut random = Lcg(seed);
    let specs: Vec<RenderNodeSpec> = (0..count)
        .map(|index| {
            let parent = (index > 0 && random.next() % roots != 0).then(|| (random.next() as usize) % index);
            RenderNodeSpec::new(None, parent, bounds())
        })
        .collect();
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&specs).unwrap();
    (tree, ids)
}

/// What the synchronization did before: the changed nodes that have no changed ancestor, found by
/// walking each one's ancestors.
fn outermost_by_walking(tree: &DrawCommandList, changed: &HashSet<RenderNodeId>) -> HashSet<RenderNodeId> {
    changed
        .iter()
        .copied()
        .filter(|id| {
            let mut ancestor = tree.node(*id).and_then(|node| node.parent);
            while let Some(parent) = ancestor {
                if changed.contains(&parent) {
                    return false;
                }
                ancestor = tree.node(parent).and_then(|node| node.parent);
            }
            true
        })
        .collect()
}

#[test]
fn the_outermost_changed_nodes_are_those_a_walk_up_the_ancestors_finds() {
    for seed in 0..40 {
        let (tree, ids) = random_tree(seed, 120, 1 + seed % 6);
        let mut random = Lcg(seed ^ 0x5eed);
        let inner = tree.draw_cmd.borrow();
        for share in [0, 1, 5, 30, 90, 100] {
            let changed: HashSet<RenderNodeId> = ids.iter().copied().filter(|_| random.next() % 100 < share).collect();
            let found: HashSet<RenderNodeId> = inner.outermost_changed(&changed).into_iter().collect();
            assert_eq!(found, outermost_by_walking(&inner, &changed), "seed {seed}, {share}% changed");
        }
    }
}

#[test]
fn a_node_that_is_not_in_the_tree_is_not_outermost_of_anything() {
    let (tree, ids) = random_tree(3, 10, 3);
    let stranger = RenderNodeId(ids.iter().map(|id| id.0).max().unwrap() + 100);
    let inner = tree.draw_cmd.borrow();
    let changed: HashSet<RenderNodeId> = [ids[2], stranger].into_iter().collect();
    assert_eq!(inner.outermost_changed(&changed), vec![ids[2]]);
    assert!(inner.outermost_changed(&HashSet::new()).is_empty(), "nothing changed");
}

#[test]
fn finding_the_outermost_of_a_deep_chain_that_all_changed_costs_its_length_and_not_its_square() {
    const DEPTH: usize = 300;
    let specs: Vec<RenderNodeSpec> = (0..DEPTH).map(|index| RenderNodeSpec::new(None, index.checked_sub(1), bounds())).collect();
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&specs).unwrap();
    let changed: HashSet<RenderNodeId> = ids.iter().copied().collect();
    let inner = tree.draw_cmd.borrow();
    drop(inner.topology());
    reset_node_lookups();

    let found = inner.outermost_changed(&changed);

    assert_eq!(found, vec![ids[0]], "the root of the chain, and nothing below it");
    let lookups = node_lookups();
    assert!(lookups <= DEPTH, "{lookups} lookups for {DEPTH} nodes in a chain");
}
