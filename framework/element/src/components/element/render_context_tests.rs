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

        assert_eq!(scope.node_for_element(a), Some(a_node));
        assert_eq!(node_for_element_hashes(), 3, "the memory holds only the last element");

        // An element without a node stays without one, however often it is asked.
        assert_eq!(scope.node_for_element(unmapped), None);
        assert_eq!(scope.node_for_element(unmapped), None);
        assert_eq!(scope.node_for_element(a), Some(a_node));
    });
}
