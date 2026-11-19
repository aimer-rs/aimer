//! Roots kept through structure synchronizations.
//!
//! A tool that paints above the application (a badge, a pointer) owns nodes the application's list
//! does not name. A synchronization removes what its list leaves out, so such a node was removed on
//! every one and made again on the next frame, which changed the structure each time. A pinned root
//! stays.

use super::*;

fn bounds() -> Rect {
    Rect::new(0.0, 0.0, 40.0, 40.0)
}

fn chain(count: usize, known: &[RenderNodeId]) -> Vec<RenderNodeSpec> {
    (0..count)
        .map(|index| RenderNodeSpec::new(known.get(index).copied(), index.checked_sub(1), bounds()))
        .collect()
}

#[test]
fn a_pinned_root_stays_through_a_synchronization_that_does_not_list_it() {
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&chain(3, &[])).unwrap();
    let badge = tree.add_root(Rect::new(1.0, 2.0, 10.0, 10.0)).unwrap();
    tree.pin_root(badge).unwrap();
    tree.set_paint_source(badge, RenderPaintSource::LocalV2).unwrap();
    tree.context(badge).unwrap().begin_recording().unwrap().commit(Vec::new()).unwrap();
    let painted = tree.draw_list_snapshot(badge).unwrap().revision;

    let again = tree.sync_structure(&chain(3, &ids)).unwrap();

    assert_eq!(again, ids, "the ids returned are those of the list, one for each entry");
    assert_eq!(tree.element_bounds(badge).unwrap(), Rect::new(1.0, 2.0, 10.0, 10.0), "kept as it was");
    assert_eq!(tree.draw_list_snapshot(badge).unwrap().revision, painted, "with what it had painted");
    assert_eq!(tree.paint_source(badge).unwrap(), RenderPaintSource::LocalV2);
}

#[test]
fn a_root_that_is_not_pinned_is_removed_as_it_always_was() {
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&chain(2, &[])).unwrap();
    let stray = tree.add_root(bounds()).unwrap();

    tree.sync_structure(&chain(2, &ids)).unwrap();

    assert_eq!(tree.element_bounds(stray), Err(RenderTreeError::UnknownNode(stray)));
}

#[test]
fn pinned_roots_paint_above_the_listed_ones_in_the_order_they_were_pinned() {
    let tree = RenderTree::new();
    let first = tree.add_root(bounds()).unwrap();
    let second = tree.add_root(bounds()).unwrap();
    tree.pin_root(first).unwrap();
    tree.pin_root(second).unwrap();

    let listed = tree.sync_structure(&[RenderNodeSpec::new(None, None, bounds()), RenderNodeSpec::new(None, None, bounds())]).unwrap();

    let inner = tree.draw_cmd.borrow();
    assert_eq!(inner.roots, vec![listed[0], listed[1], first, second]);
}

#[test]
fn a_pinned_root_the_list_names_is_not_a_duplicate() {
    let tree = RenderTree::new();
    let badge = tree.add_root(bounds()).unwrap();
    tree.pin_root(badge).unwrap();

    let ids = tree.sync_structure(&[RenderNodeSpec::new(Some(badge), None, bounds())]).unwrap();

    assert_eq!(ids, vec![badge]);
    assert_eq!(tree.draw_cmd.borrow().nodes.len(), 1, "one node, not a second for the pin");
}

#[test]
fn only_a_root_without_children_can_be_pinned() {
    let tree = RenderTree::new();
    let root = tree.add_root(bounds()).unwrap();
    let child = tree.add_child(root, bounds()).unwrap();
    let stranger = RenderNodeId(child.0 + 100);

    assert_eq!(tree.pin_root(stranger), Err(RenderTreeError::UnknownNode(stranger)));
    assert_eq!(tree.pin_root(child), Err(RenderTreeError::NotALeafRoot(child)), "it has a parent");
    assert_eq!(tree.pin_root(root), Err(RenderTreeError::NotALeafRoot(root)), "it has a child");
    assert!(tree.draw_cmd.borrow().pinned.is_empty(), "nothing was pinned");
}

#[test]
fn pinning_twice_keeps_one_pin_and_unpinning_lets_the_next_synchronization_remove_it() {
    let tree = RenderTree::new();
    let badge = tree.add_root(bounds()).unwrap();
    tree.pin_root(badge).unwrap();
    tree.pin_root(badge).unwrap();
    assert_eq!(tree.draw_cmd.borrow().pinned.len(), 1);
    tree.sync_structure(&[]).unwrap();
    assert!(tree.element_bounds(badge).is_ok(), "kept by an empty list too");

    tree.unpin_root(badge);
    tree.sync_structure(&[]).unwrap();

    assert!(tree.element_bounds(badge).is_err());
    tree.unpin_root(badge);
}

#[test]
fn a_pin_on_a_node_that_is_gone_is_forgotten() {
    let tree = RenderTree::new();
    let badge = tree.add_root(bounds()).unwrap();
    tree.pin_root(badge).unwrap();
    tree.draw_cmd.borrow_mut().pinned.push(RenderNodeId(badge.0 + 500));

    tree.sync_structure(&[]).unwrap();

    assert_eq!(tree.draw_cmd.borrow().pinned, vec![badge]);
}

/// What it is for: the same list again, with a pinned root beside it, is not a change of structure,
/// so what is worked out from the structure is not worked out again.
#[test]
fn the_same_list_again_does_not_change_the_structure_while_a_root_is_pinned() {
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&chain(50, &[])).unwrap();
    let badge = tree.add_root(bounds()).unwrap();
    tree.pin_root(badge).unwrap();
    tree.sync_structure(&chain(50, &ids)).unwrap();
    let revision = tree.topology_revision();

    for _ in 0..3 {
        tree.sync_structure(&chain(50, &ids)).unwrap();
    }

    assert_eq!(tree.topology_revision(), revision);
    // A real change is still one.
    tree.sync_structure(&chain(51, &ids)).unwrap();
    assert_ne!(tree.topology_revision(), revision);
}

#[test]
fn a_clip_and_a_bound_the_owner_gave_a_pinned_root_survive_a_synchronization_with_clips() {
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&chain(2, &[])).unwrap();
    let pointer = tree.add_root(Rect::new(5.0, 5.0, 20.0, 20.0)).unwrap();
    tree.pin_root(pointer).unwrap();
    let window = Rect::new(0.0, 0.0, 300.0, 200.0);
    tree.set_clip(pointer, Some(window)).unwrap();
    tree.set_bounds(pointer, Rect::new(50.0, 60.0, 20.0, 20.0)).unwrap();

    tree.sync_structure_with_clip_radii(&chain(2, &ids), &[None, None], &[[0.0; 4], [0.0; 4]]).unwrap();

    let state = tree.node_state(pointer).unwrap();
    assert_eq!(state.clip, Some(window));
    assert_eq!(state.bounds, Rect::new(50.0, 60.0, 20.0, 20.0));
}
