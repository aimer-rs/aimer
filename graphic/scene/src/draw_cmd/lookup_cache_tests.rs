//! A node asked about several times in a row is found in the id index once.
//!
//! Resolving one element's paint asks the tree about the same node over and
//! over: its paint source, its context, whether it needs recording. Each used
//! to hash the id again. The list now remembers the last position it found.

use super::*;

fn tree() -> (RenderTree, RenderNodeId, RenderNodeId, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let a = tree.add_child(root, Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let b = tree.add_child(root, Rect::new(10.0, 0.0, 10.0, 10.0)).unwrap();
    (tree, root, a, b)
}

#[test]
fn repeated_queries_about_one_node_hash_its_id_once() {
    let (tree, root, a, b) = tree();
    let c = tree.add_child(root, Rect::new(20.0, 0.0, 10.0, 10.0)).unwrap();
    // Building the tree asked about its nodes; start from a table that knows none.
    tree.draw_cmd.borrow().forget_lookup_hints();
    reset_node_lookups();

    for _ in 0..3 {
        tree.paint_source(a).unwrap();
    }
    tree.needs_recording(a).unwrap();
    tree.context(a).unwrap();
    assert_eq!(index_hash_lookups(), 1, "six questions about one node");

    // A wrapper's resolve alternates between itself and the child it checks.
    for _ in 0..3 {
        tree.paint_source(a).unwrap();
        tree.paint_source(b).unwrap();
    }
    assert_eq!(index_hash_lookups(), 2, "each of the two nodes is hashed once");

    // A third node does not push either of the others out.
    tree.paint_source(c).unwrap();
    tree.paint_source(a).unwrap();
    tree.paint_source(b).unwrap();
    assert_eq!(index_hash_lookups(), 3, "only the new node is hashed");
}

/// A frame asks about every node of a window, one after another and then again
/// for the next question about each. Remembering only the last few positions
/// hashes every one of them every time.
#[test]
fn many_nodes_asked_about_in_rotation_are_hashed_once_each() {
    const COUNT: usize = 300;
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let mut ids = vec![root];
    for _ in 1..COUNT {
        ids.push(tree.add_child(root, Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap());
    }
    tree.draw_cmd.borrow().forget_lookup_hints();
    reset_node_lookups();

    for _ in 0..3 {
        for id in &ids {
            tree.paint_source(*id).unwrap();
        }
    }

    // The root sits at position 0, which an empty slot already names, so it
    // is found without hashing; every other node is hashed on the first pass
    // only.
    assert_eq!(
        index_hash_lookups(),
        COUNT - 1,
        "three passes over {COUNT} nodes hash each node on the first pass only"
    );
}

/// Two nodes whose ids share a table slot still each get their own answer.
#[test]
fn nodes_that_share_a_table_slot_are_still_told_apart() {
    let mut list = DrawCommandList::default();
    let root = list.insert(None, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let mut children = Vec::new();
    for offset in 0..(HINT_SLOTS + 8) {
        children.push(
            list.insert(Some(root), Rect::new(offset as f32, 0.0, 1.0, 1.0))
                .unwrap(),
        );
    }
    // `children[0]` and `children[HINT_SLOTS]` have ids that differ by exactly
    // the table size, so they map to one slot.
    let (first, second) = (children[0], children[HINT_SLOTS]);
    assert_eq!(first.get() as usize % HINT_SLOTS, second.get() as usize % HINT_SLOTS);
    list.forget_lookup_hints();

    for _ in 0..4 {
        assert_eq!(list.node(first).unwrap().bounds.x, 0.0);
        assert_eq!(list.node(second).unwrap().bounds.x, HINT_SLOTS as f32);
    }
}

#[test]
fn a_remembered_position_is_never_trusted_after_the_nodes_move() {
    let mut list = DrawCommandList::default();
    let root = list.insert(None, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let a = list.insert(Some(root), Rect::new(1.0, 0.0, 10.0, 10.0)).unwrap();
    let b = list.insert(Some(root), Rect::new(2.0, 0.0, 10.0, 10.0)).unwrap();

    // Remember `a` at position 1, then rearrange the way a structure sync can.
    assert_eq!(list.node(a).unwrap().bounds.x, 1.0);
    list.nodes.swap(1, 2);
    list.indices.insert(a, 2);
    list.indices.insert(b, 1);

    assert_eq!(list.node(a).unwrap().bounds.x, 1.0, "the swapped node is still found");
    assert_eq!(list.node(b).unwrap().bounds.x, 2.0);

    // A node that is gone is gone, whatever the memory holds.
    list.nodes.truncate(2);
    list.indices.remove(&a);
    assert!(list.node(a).is_none());
    assert_eq!(list.node(b).unwrap().bounds.x, 2.0);
}
