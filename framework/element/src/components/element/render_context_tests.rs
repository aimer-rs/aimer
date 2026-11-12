//! The retained render context is a stack scoped to one traversal.

use super::*;

fn scope(tree: RenderTree) -> (RenderTree, Rc<ElementNodeMap>) {
    (tree, Rc::new(ElementNodeMap::default()))
}

#[test]
fn a_render_tree_is_active_only_inside_its_traversal() {
    assert!(!has_active_v2_render_tree());
    assert!(!has_active_v2_render_presentation());

    let (tree, nodes) = scope(RenderTree::new());
    with_v2_render_tree_context(tree, nodes, None, || {
        assert!(has_active_v2_render_tree());
        assert!(has_active_v2_render_presentation());
    });

    assert!(!has_active_v2_render_tree());
}

#[test]
fn nested_traversals_restore_the_enclosing_one() {
    let outer = RenderTree::new();
    let outer_node = outer.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let inner = RenderTree::new();
    let element = ElementId::next();
    let mut mapped = ElementNodeMap::default();
    mapped.insert(element, outer_node);
    let mapped = Rc::new(mapped);

    with_v2_render_tree_context(outer, mapped, None, || {
        assert!(current_v2_render_context().unwrap().node_for_element(element).is_some());

        let (inner, empty) = scope(inner);
        with_v2_render_tree_context(inner, empty, None, || {
            assert!(has_active_v2_render_tree());
            assert!(current_v2_render_context().unwrap().node_for_element(element).is_none());
        });

        assert!(has_active_v2_render_tree());
        assert!(current_v2_render_context().unwrap().node_for_element(element).is_some());
    });
    assert!(!has_active_v2_render_tree());
}

#[test]
fn a_traversal_that_panics_does_not_leave_its_tree_active() {
    let (tree, nodes) = scope(RenderTree::new());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_v2_render_tree_context(tree, nodes, None, || panic!("paint failed"));
    }));

    assert!(outcome.is_err());
    assert!(!has_active_v2_render_tree());
}

#[test]
fn the_prepared_root_is_handed_out_once() {
    let root = ElementId::next();
    let other = ElementId::next();
    let (tree, nodes) = scope(RenderTree::new());

    with_v2_render_tree_context(tree, nodes, Some(root), || {
        let context = current_v2_render_context().unwrap();
        assert!(!context.take_prepared_root(other));
        assert!(context.take_prepared_root(root));
        assert!(!context.take_prepared_root(root), "it is consumed");
        // Every clone of the context shares the one slot.
        assert!(!current_v2_render_context().unwrap().take_prepared_root(root));
    });
}

#[test]
fn clones_of_a_context_share_the_same_tree() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let element = ElementId::next();
    let mut mapped = ElementNodeMap::default();
    mapped.insert(element, node);

    with_v2_render_tree_context(tree.clone(), Rc::new(mapped), None, || {
        let first = current_v2_render_context().unwrap();
        let second = first.clone();
        second.tree.set_bounds(node, Rect::new(1.0, 2.0, 10.0, 10.0)).unwrap();

        assert_eq!(first.tree.element_bounds(node).unwrap().x, 1.0);
        assert_eq!(tree.element_bounds(node).unwrap().x, 1.0);
        assert_eq!(first.node_for_element(element), Some(node));
    });
}

#[test]
fn asking_about_one_element_repeatedly_hashes_it_once() {
    let tree = RenderTree::new();
    let a_node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let b_node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let (a, b, unmapped) = (ElementId::next(), ElementId::next(), ElementId::next());
    let mut mapped = ElementNodeMap::default();
    mapped.insert(a, a_node);
    mapped.insert(b, b_node);

    with_v2_render_tree_context(tree, Rc::new(mapped), None, || {
        let scope = current_v2_render_context().unwrap();
        reset_node_for_element_hashes();

        for _ in 0..3 {
            assert_eq!(scope.node_for_element(a), Some(a_node));
        }
        assert_eq!(node_for_element_hashes(), 1, "three questions about one element");

        assert_eq!(scope.node_for_element(b), Some(b_node));
        assert_eq!(node_for_element_hashes(), 2, "another element is looked up again");

        // Neither element pushes the other out of the map's table of recent answers.
        assert_eq!(scope.node_for_element(a), Some(a_node));
        assert_eq!(node_for_element_hashes(), 2, "the first element is still remembered");

        // An element without a node stays without one, however often it is asked.
        assert_eq!(scope.node_for_element(unmapped), None);
        assert_eq!(scope.node_for_element(unmapped), None);
        assert_eq!(scope.node_for_element(a), Some(a_node));
    });
}

/// A frame asks about every element of a window, and each one several times.
/// Remembering only the last element hashes all of them on every question.
#[test]
fn many_elements_asked_about_in_rotation_are_hashed_once_each() {
    const COUNT: usize = 300;
    let tree = RenderTree::new();
    let mut mapped = ElementNodeMap::default();
    let mut elements = Vec::new();
    for _ in 0..COUNT {
        let element = ElementId::next();
        let node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        mapped.insert(element, node);
        elements.push((element, node));
    }

    with_v2_render_tree_context(tree, Rc::new(mapped), None, || {
        let scope = current_v2_render_context().unwrap();
        reset_node_for_element_hashes();

        for _ in 0..3 {
            for (element, node) in &elements {
                assert_eq!(scope.node_for_element(*element), Some(*node));
            }
        }

        assert_eq!(
            node_for_element_hashes(),
            COUNT as u64,
            "three passes over {COUNT} elements hash each element on the first pass only"
        );
    });
}

/// Elements whose ids share a slot of the table of recent answers each still
/// get their own node.
#[test]
fn elements_that_share_a_slot_are_still_told_apart() {
    let tree = RenderTree::new();
    let mut mapped = ElementNodeMap::default();
    let first = ElementId::next();
    let first_node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    mapped.insert(first, first_node);
    // Ids are consecutive, so a run of one table size longer than the
    // table reaches an id that lands in `first`'s slot.
    let mut colliding = None;
    for _ in 0..(ElementNodeMap::SLOTS + 1) {
        let element = ElementId::next();
        if element.get() as usize % ElementNodeMap::SLOTS == first.get() as usize % ElementNodeMap::SLOTS {
            let node = tree.add_root(Rect::new(1.0, 0.0, 10.0, 10.0)).unwrap();
            mapped.insert(element, node);
            colliding = Some((element, node));
            break;
        }
    }
    let (second, second_node) = colliding.expect("an id lands in the slot within one table length");

    with_v2_render_tree_context(tree, Rc::new(mapped), None, || {
        let scope = current_v2_render_context().unwrap();
        for _ in 0..4 {
            assert_eq!(scope.node_for_element(first), Some(first_node));
            assert_eq!(scope.node_for_element(second), Some(second_node));
        }
    });
}

/// The table of recent answers is filled from the map it belongs to, so changing
/// the map while it is unshared must change what it answers.
#[test]
fn inserting_into_a_map_replaces_what_was_remembered() {
    let tree = RenderTree::new();
    let old_node = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let new_node = tree.add_root(Rect::new(1.0, 0.0, 10.0, 10.0)).unwrap();
    let element = ElementId::next();
    let mut mapped = ElementNodeMap::default();
    mapped.insert(element, old_node);

    assert_eq!(mapped.get(&element), Some(old_node));
    mapped.insert(element, new_node);
    assert_eq!(mapped.get(&element), Some(new_node), "the answer follows the insert");
    assert_eq!(mapped.get(&ElementId::next()), None);
}
