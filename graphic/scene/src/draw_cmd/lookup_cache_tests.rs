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
    // Building the tree asked about its nodes; leave `root` and `b` in the
    // memory so that `a` has to be found.
    tree.paint_source(root).unwrap();
    tree.paint_source(b).unwrap();
    reset_node_lookups();

    for _ in 0..3 {
        tree.paint_source(a).unwrap();
    }
    tree.needs_recording(a).unwrap();
    tree.context(a).unwrap();
    assert_eq!(index_hash_lookups(), 1, "six questions about one node");

    // The memory now holds `a` and `b`; `root` is the one pushed out.
    tree.paint_source(root).unwrap();
    assert_eq!(index_hash_lookups(), 2, "a node that was pushed out is looked up again");

    // A wrapper's resolve alternates between itself and the child it checks.
    for _ in 0..3 {
        tree.paint_source(a).unwrap();
        tree.paint_source(root).unwrap();
    }
    assert_eq!(index_hash_lookups(), 2, "the memory holds the last two nodes");

    // `root` and `a` are held; a third node pushes the older one, `a`, out.
    tree.paint_source(b).unwrap();
    assert_eq!(index_hash_lookups(), 3);
    tree.paint_source(a).unwrap();
    assert_eq!(index_hash_lookups(), 4, "the oldest of three is gone");
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
