//! Collects the accessibility tree an element tree describes.
//!
//! Elements opt in through [`EventElement::semantics`](crate::EventElement::semantics).
//! An element that publishes nothing is transparent: whatever its descendants
//! publish is attached to the nearest ancestor that does publish, so a tree of
//! layout wrappers around a button yields the button and not the wrappers.
//!
//! Collection walks the structural children once and allocates only the nodes it
//! publishes. It is meant to run only while an assistive technology is attached;
//! the frame loop never pays for it otherwise.

use aimer_accessibility::{Bounds, NodeId, Role, SemanticAction, SemanticNode, SemanticTree};

use crate::{Element, ElementId};

/// Identity of the synthetic root every collected tree hangs from.
///
/// Element identities start at one, so zero can never collide with an element.
pub const SEMANTIC_ROOT_ID: NodeId = NodeId::new(0);

/// Collects the semantic tree under `root`.
///
/// `bounds_of` resolves an element's rectangle in window logical pixels; an
/// element it answers `None` for is published without bounds. The result always
/// has a synthetic [`Role::Group`] root with id [`SEMANTIC_ROOT_ID`], whose
/// children are the outermost published nodes.
pub fn collect_semantic_tree(
    root: &dyn Element,
    bounds_of: &mut dyn FnMut(ElementId) -> Option<Bounds>,
) -> SemanticTree {
    let mut top = Vec::new();
    collect(root, bounds_of, &mut top);
    let mut group = SemanticNode::new(SEMANTIC_ROOT_ID, Role::Group);
    for node in top {
        group = group.with_child(node);
    }
    SemanticTree::new(group)
}

fn collect(
    element: &dyn Element,
    bounds_of: &mut dyn FnMut(ElementId) -> Option<Bounds>,
    out: &mut Vec<SemanticNode>,
) {
    let published = element
        .element_id()
        .and_then(|id| Some((id, element.semantics(NodeId::new(id.get()))?)));
    let Some((id, mut node)) = published else {
        element.visit_children(&mut |child| collect(child, bounds_of, out));
        return;
    };
    let mut children = Vec::new();
    element.visit_children(&mut |child| collect(child, bounds_of, &mut children));
    // An element that paints nothing of its own (a wrapper that only annotates
    // its child) has no render node, so it spans what it contains.
    if let Some(bounds) = bounds_of(id).or_else(|| union_of(&children)) {
        node = node.with_bounds(bounds);
    }
    for child in children {
        node = node.with_child(child);
    }
    out.push(node);
}

fn union_of(nodes: &[SemanticNode]) -> Option<Bounds> {
    let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut any = false;
    for bounds in nodes.iter().filter_map(SemanticNode::bounds) {
        any = true;
        left = left.min(bounds.x());
        top = top.min(bounds.y());
        right = right.max(bounds.x() + bounds.width());
        bottom = bottom.max(bounds.y() + bounds.height());
    }
    if any { Bounds::new(left, top, right - left, bottom - top).ok() } else { None }
}

/// Asks the element published as `node` to perform `action`.
///
/// Returns whether an element with that identity exists and handled the action.
/// This walks the tree, so it belongs on the assistive-technology path, not on a
/// per-frame one.
pub fn perform_semantic_action(
    root: &dyn Element,
    node: NodeId,
    action: &SemanticAction,
) -> bool {
    if root.element_id().is_some_and(|id| id.get() == node.get()) {
        return root.perform_semantic_action(action);
    }
    let mut handled = false;
    root.visit_children(&mut |child| {
        if !handled {
            handled = perform_semantic_action(child, node, action);
        }
    });
    handled
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::base::BuildContext;
    use crate::{AnyElement, Drawable, EventElement, LayoutElement, Rebuildable, VisitorElement};

    /// Publishes `role` when it has one; otherwise it is a transparent wrapper.
    struct Probe {
        role: Option<Role>,
        label: &'static str,
        children: Vec<AnyElement>,
        activated: RefCell<u32>,
    }

    impl Probe {
        fn wrapper(children: Vec<AnyElement>) -> AnyElement {
            Self { role: None, label: "", children, activated: RefCell::new(0) }.boxed()
        }

        fn node(role: Role, label: &'static str, children: Vec<AnyElement>) -> AnyElement {
            Self { role: Some(role), label, children, activated: RefCell::new(0) }.boxed()
        }
    }

    impl VisitorElement for Probe {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.children {
                visitor(&**child);
            }
        }

        fn debug_name(&self) -> &'static str {
            "SemanticsProbe"
        }
    }

    impl EventElement for Probe {
        fn semantics(&self, id: NodeId) -> Option<SemanticNode> {
            Some(
                SemanticNode::new(id, self.role?)
                    .with_name(self.label)
                    .with_action(SemanticAction::Activate),
            )
        }

        fn perform_semantic_action(&self, action: &SemanticAction) -> bool {
            *action == SemanticAction::Activate && {
                *self.activated.borrow_mut() += 1;
                true
            }
        }
    }

    impl LayoutElement for Probe {}
    impl Rebuildable for Probe {}

    impl Drawable for Probe {
        fn update(&self, _ctx: &BuildContext) {}
    }

    fn no_bounds(_: ElementId) -> Option<Bounds> {
        None
    }

    #[test]
    fn a_tree_that_publishes_nothing_collects_to_an_empty_root() {
        let root = Probe::wrapper(vec![Probe::wrapper(vec![])]);

        let tree = collect_semantic_tree(&*root, &mut no_bounds);

        assert_eq!(tree.root().id(), SEMANTIC_ROOT_ID);
        assert!(tree.root().children().is_empty());
    }

    #[test]
    fn wrappers_are_transparent_and_published_nodes_nest() {
        let button = Probe::node(Role::Button, "Save", vec![]);
        let button_id = button.id();
        let root = Probe::wrapper(vec![Probe::node(
            Role::Group,
            "Toolbar",
            vec![Probe::wrapper(vec![button])],
        )]);

        let tree = collect_semantic_tree(&*root, &mut no_bounds);

        let toolbar = &tree.root().children()[0];
        assert_eq!(toolbar.name(), Some("Toolbar"));
        assert_eq!(toolbar.children().len(), 1);
        let save = &toolbar.children()[0];
        assert_eq!(save.name(), Some("Save"));
        assert_eq!(save.id(), NodeId::new(button_id.get()));
    }

    #[test]
    fn bounds_come_from_the_resolver_and_are_optional() {
        let sized = Probe::node(Role::Button, "Sized", vec![]);
        let sized_id = sized.id();
        let root = Probe::wrapper(vec![sized, Probe::node(Role::Button, "Unsized", vec![])]);
        let rect = Bounds::new(4.0, 8.0, 100.0, 40.0).unwrap();

        let tree = collect_semantic_tree(&*root, &mut |id| (id == sized_id).then_some(rect));

        let children = tree.root().children();
        assert_eq!(children[0].bounds(), Some(rect));
        assert_eq!(children[1].bounds(), None);
    }

    #[test]
    fn a_node_without_its_own_bounds_spans_its_children() {
        let left = Probe::node(Role::Button, "Left", vec![]);
        let left_id = left.id();
        let right = Probe::node(Role::Button, "Right", vec![]);
        let right_id = right.id();
        let root = Probe::wrapper(vec![Probe::node(Role::Group, "Row", vec![left, right])]);
        let left_rect = Bounds::new(10.0, 20.0, 30.0, 40.0).unwrap();
        let right_rect = Bounds::new(50.0, 25.0, 30.0, 50.0).unwrap();

        let tree = collect_semantic_tree(&*root, &mut |id| {
            if id == left_id {
                Some(left_rect)
            } else if id == right_id {
                Some(right_rect)
            } else {
                None
            }
        });

        let row = &tree.root().children()[0];
        assert_eq!(row.bounds(), Some(Bounds::new(10.0, 20.0, 70.0, 55.0).unwrap()));
    }

    #[test]
    fn an_action_reaches_the_element_that_published_the_node() {
        let target = Probe::node(Role::Button, "Target", vec![]);
        let target_id = NodeId::new(target.id().get());
        let other = Probe::node(Role::Button, "Other", vec![]);
        let root = Probe::wrapper(vec![other, Probe::wrapper(vec![target])]);

        assert!(perform_semantic_action(&*root, target_id, &SemanticAction::Activate));
        assert!(!perform_semantic_action(&*root, target_id, &SemanticAction::Dismiss));
        assert!(!perform_semantic_action(
            &*root,
            NodeId::new(u64::MAX),
            &SemanticAction::Activate
        ));
    }
}
