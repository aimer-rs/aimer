//! A retained node can report where it is on screen long after it was
//! synchronized, which is what a tag registry needs when a navigation starts.

use super::*;

#[test]
fn a_context_reports_its_nodes_world_bounds() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(10.0, 20.0, 200.0, 200.0)).unwrap();
    let child = tree.add_child(root, Rect::new(5.0, 6.0, 30.0, 40.0)).unwrap();

    let context = tree.context(child).unwrap();

    assert_eq!(context.world_bounds(), Some(Rect::new(15.0, 26.0, 30.0, 40.0)));
}

#[test]
fn a_kept_context_follows_its_ancestors_as_they_move() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 200.0)).unwrap();
    let child = tree.add_child(root, Rect::new(5.0, 6.0, 30.0, 40.0)).unwrap();
    let context = tree.context(child).unwrap();

    // A scroll moves the content node under its viewport.
    tree.set_bounds(root, Rect::new(0.0, -100.0, 200.0, 200.0)).unwrap();

    assert_eq!(context.world_bounds(), Some(Rect::new(5.0, -94.0, 30.0, 40.0)));
}
