//! Which elements may have left a compositor animation on their render node.
//!
//! An element that stops animating has to reset its node, but nearly every
//! element never animates at all. Recording the few that did, instead of
//! asking the render tree about all of them every frame, makes the common
//! answer a check of an empty set.

use std::cell::RefCell;

use aimer_cupid::utilities::IdBuildHasher;

use super::*;

thread_local! {
    static APPLIED: RefCell<HashSet<ElementId, IdBuildHasher>> = RefCell::new(HashSet::default());
}

/// Records that `element` may now have a compositor animation on its node.
#[cfg_attr(
    any(target_arch = "wasm32", feature = "portable-guest"),
    allow(dead_code)
)]
pub(crate) fn note_compositor_animation_applied(element: ElementId) {
    APPLIED.with(|applied| {
        applied.borrow_mut().insert(element);
    });
}

/// Whether `element` may have a compositor animation to clear.
#[cfg_attr(
    any(target_arch = "wasm32", feature = "portable-guest"),
    allow(dead_code)
)]
#[inline]
pub(crate) fn compositor_animation_may_be_applied(element: ElementId) -> bool {
    APPLIED.with(|applied| {
        let applied = applied.borrow();
        !applied.is_empty() && applied.contains(&element)
    })
}

/// Records that `element`'s node was reset, so there is nothing left to clear.
#[cfg_attr(
    any(target_arch = "wasm32", feature = "portable-guest"),
    allow(dead_code)
)]
pub(crate) fn note_compositor_animation_cleared(element: ElementId) {
    APPLIED.with(|applied| {
        applied.borrow_mut().remove(&element);
    });
}

/// Forgets elements that no longer have a render node.
///
/// An element removed while it was animating can never be asked to clear, and
/// a node it gets later starts at rest. Dropping its entry keeps the set to
/// the elements that can still be affected. Called as each traversal begins.
pub(crate) fn forget_unmapped_compositor_animations(element_nodes: &ElementNodeMap) {
    APPLIED.with(|applied| {
        let mut applied = applied.borrow_mut();
        if !applied.is_empty() {
            applied.retain(|element| element_nodes.contains_key(element));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes_for(elements: &[ElementId]) -> ElementNodeMap {
        let tree = RenderTree::new();
        let mut nodes = ElementNodeMap::default();
        for element in elements {
            nodes.insert(*element, tree.add_root(Rect::new(0.0, 0.0, 1.0, 1.0)).unwrap());
        }
        nodes
    }

    #[test]
    fn nothing_is_applied_until_it_is_noted() {
        let element = ElementId::next();
        assert!(!compositor_animation_may_be_applied(element));

        note_compositor_animation_applied(element);
        assert!(compositor_animation_may_be_applied(element));
        assert!(!compositor_animation_may_be_applied(ElementId::next()));

        note_compositor_animation_cleared(element);
        assert!(!compositor_animation_may_be_applied(element));
    }

    #[test]
    fn noting_twice_still_clears_once() {
        let element = ElementId::next();
        note_compositor_animation_applied(element);
        note_compositor_animation_applied(element);
        note_compositor_animation_cleared(element);
        assert!(!compositor_animation_may_be_applied(element));
        // Clearing what was never applied is harmless.
        note_compositor_animation_cleared(element);
        note_compositor_animation_cleared(ElementId::next());
    }

    #[test]
    fn elements_are_tracked_independently() {
        let (a, b, c) = (ElementId::next(), ElementId::next(), ElementId::next());
        note_compositor_animation_applied(a);
        note_compositor_animation_applied(b);

        note_compositor_animation_cleared(a);

        assert!(!compositor_animation_may_be_applied(a));
        assert!(compositor_animation_may_be_applied(b));
        assert!(!compositor_animation_may_be_applied(c));
    }

    #[test]
    fn pruning_keeps_mapped_elements_and_drops_the_rest() {
        let (kept, gone) = (ElementId::next(), ElementId::next());
        note_compositor_animation_applied(kept);
        note_compositor_animation_applied(gone);

        forget_unmapped_compositor_animations(&nodes_for(&[kept]));

        assert!(compositor_animation_may_be_applied(kept));
        assert!(!compositor_animation_may_be_applied(gone));
    }

    #[test]
    fn pruning_an_empty_set_does_nothing() {
        forget_unmapped_compositor_animations(&nodes_for(&[]));
        assert!(!compositor_animation_may_be_applied(ElementId::next()));
    }

    #[test]
    fn a_traversal_prunes_as_it_begins() {
        let (kept, gone) = (ElementId::next(), ElementId::next());
        note_compositor_animation_applied(kept);
        note_compositor_animation_applied(gone);
        let nodes = Rc::new(nodes_for(&[kept]));

        with_v2_render_tree_context(RenderTree::new(), nodes, None, || {
            assert!(compositor_animation_may_be_applied(kept));
            assert!(!compositor_animation_may_be_applied(gone));
        });
    }
}
