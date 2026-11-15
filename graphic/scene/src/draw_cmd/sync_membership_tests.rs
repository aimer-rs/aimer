//! Which nodes a structure synchronization accepts, and what it does to the ones
//! it no longer lists.
//!
//! The synchronization tracks the ids it has seen while it validates the list.
//! These pin down the answers that tracking gives, so how it is kept can change.

use super::*;

fn bounds() -> Rect {
    Rect::new(0.0, 0.0, 40.0, 40.0)
}

fn chain(ids: &[Option<RenderNodeId>]) -> Vec<RenderNodeSpec> {
    ids.iter()
        .enumerate()
        .map(|(index, id)| RenderNodeSpec::new(*id, index.checked_sub(1), bounds()))
        .collect()
}

fn existing_chain(len: usize) -> (RenderTree, Vec<RenderNodeId>) {
    let tree = RenderTree::new();
    let ids = tree.sync_structure(&chain(&vec![None; len])).unwrap();
    (tree, ids)
}

#[test]
fn a_list_naming_the_same_existing_node_twice_is_refused() {
    let (tree, ids) = existing_chain(3);

    let specs = chain(&[Some(ids[0]), Some(ids[1]), Some(ids[1])]);

    assert_eq!(
        tree.sync_structure(&specs),
        Err(RenderTreeError::DuplicateNode(ids[1]))
    );
}

#[test]
fn a_list_naming_a_node_the_tree_does_not_have_is_refused() {
    let (tree, ids) = existing_chain(2);
    let stranger = RenderNodeId(ids.iter().map(|id| id.0).max().unwrap() + 100);

    let specs = chain(&[Some(ids[0]), Some(stranger)]);

    assert_eq!(
        tree.sync_structure(&specs),
        Err(RenderTreeError::UnknownNode(stranger))
    );
}

#[test]
fn a_refused_list_leaves_the_tree_as_it_was() {
    let (tree, ids) = existing_chain(3);
    let _ = tree.take_damage();
    let before = tree.render_all().len();

    let _ = tree.sync_structure(&chain(&[Some(ids[0]), Some(ids[1]), Some(ids[1])]));

    assert_eq!(tree.render_all().len(), before);
    assert!(tree.take_damage().is_empty(), "a refused list damaged nothing");
}

#[test]
fn nodes_the_list_no_longer_names_are_dropped_and_their_parent_is_damaged() {
    let (tree, ids) = existing_chain(4);
    let _ = tree.take_damage();

    // Keep the first two and add a fresh one in the place of the last two.
    let kept = tree
        .sync_structure(&chain(&[Some(ids[0]), Some(ids[1]), None]))
        .unwrap();

    assert_eq!(&kept[..2], &ids[..2], "kept nodes keep their identity");
    assert!(!kept.contains(&ids[2]) && !kept.contains(&ids[3]));
    assert!(
        !tree.take_damage().is_empty(),
        "dropping nodes must repaint where they were"
    );
}

#[test]
fn a_list_naming_every_node_once_changes_nothing_structural() {
    let (tree, ids) = existing_chain(5);
    let _ = tree.take_damage();

    let again = tree
        .sync_structure(&chain(&ids.iter().copied().map(Some).collect::<Vec<_>>()))
        .unwrap();

    assert_eq!(again, ids);
    assert!(tree.take_damage().is_empty());
}
