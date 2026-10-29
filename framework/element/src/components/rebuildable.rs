use crate::base::BuildContext;
use crate::components::element::{Element, ElementId, VisitorElement, element_tree_generation};

/// The part of a self-rebuilding element (one that owns a dirty flag and a
/// `build` closure) that the dirty-path index and the rebuild walk need.
///
/// Implemented by the stateless and stateful elements; reached through
/// [`Rebuildable::self_rebuilding`] so the walk never has to name those types.
#[doc(hidden)]
pub trait SelfRebuilding {
    /// Whether this element's own `build` is waiting to run.
    fn is_dirty(&self) -> bool;

    /// Records the root-relative path the dirty-path index reaches this element by.
    fn set_rebuild_path(&self, path: &[ElementId]);

    /// Like [`set_rebuild_path`](SelfRebuilding::set_rebuild_path), but leaves a
    /// path that is already known in place.
    fn set_rebuild_path_if_missing(&self, path: &[ElementId]);

    /// Whether the last build may have read inherited context. A build that did
    /// not may be skipped by a placement; the conservative answer is `true`.
    fn reads_inherited_context(&self) -> bool;
}

// Rebuild capabilities
pub trait Rebuildable: VisitorElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.visit_children(&mut |child| {
            child.rebuild_if_dirty(ctx);
        });
    }

    /// Rebuilds only the children the dirty-path index leads to, without
    /// entering the rest.
    ///
    /// Called instead of [`rebuild_if_dirty`](Rebuildable::rebuild_if_dirty) when
    /// the pass has already established that this element has no work of its
    /// own and that the index describes the tree. A pruned child costs little
    /// to enter, but entering every child of a long list to find that out is
    /// work proportional to the list. A container that can reach a child by
    /// position implements this with `rebuild_dirty_children`.
    ///
    /// Returns `false` when the children could not be resolved, in which case
    /// the caller visits all of them as usual. The default does exactly that.
    #[doc(hidden)]
    #[inline]
    #[allow(unused_variables)]
    fn rebuild_indexed_children(&self, ctx: &BuildContext) -> bool {
        false
    }

    /// Whether marking this element for rebuild does nothing but mark its
    /// children, and rebuilding it does nothing but rebuild them.
    ///
    /// The dirty-path index lets a rebuild pass skip everything that cannot lead
    /// to work, which it knows for the elements that keep a rebuild source of
    /// their own. A mark on any other element that has children could reach a
    /// producer of rebuilds the index does not know of, so such a mark discards
    /// the index and the next pass walks the whole tree. An element that returns
    /// `true` promises it is not such a producer itself: the mark passes through
    /// it, and what it reaches below is judged on its own. A plain wrapper (a
    /// container, a retained child) should; anything that overrides
    /// [`mark_needs_rebuild`](Rebuildable::mark_needs_rebuild) or
    /// [`rebuild_if_dirty`](Rebuildable::rebuild_if_dirty) to do work of its own
    /// must not.
    ///
    /// The default is `false`, which is always safe.
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        false
    }

    fn option_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    fn is_stateful_element(&self) -> bool {
        false
    }

    /// Returns the state this element carries across a rebuild, if it owns any
    /// of the kind the state carry hands over by key.
    #[doc(hidden)]
    #[inline]
    fn state_carrier(&self) -> Option<&dyn crate::components::element::StateCarrier> {
        None
    }

    /// Returns this element's rebuild source if it owns one.
    ///
    /// Wrappers that forward [`option_any`](Rebuildable::option_any) to an inner
    /// element forward this too.
    #[doc(hidden)]
    #[inline]
    fn self_rebuilding(&self) -> Option<&dyn SelfRebuilding> {
        None
    }

    fn is_carry_state(&self) -> bool {
        false
    }

    /// Adopts whatever runtime state the element this one replaces was holding.
    ///
    /// Reconciliation pairs children by sibling position, which reaches
    /// everything an ordinary element owns. Two kinds of state escape it:
    ///
    /// - children a container materializes *on demand*, because a freshly built
    ///   container has none of them yet, so the walk finds no pair;
    /// - state a container keeps beside its children, such as a measurement of a
    ///   list too long to measure again.
    ///
    /// Such an element overrides this and takes what it needs — including the
    /// children themselves — out of `old`, which is always the same concrete type
    /// and is dropped immediately afterwards. Reach the concrete type through
    /// [`Rebuildable::option_any`].
    #[inline]
    #[allow(unused_variables)]
    fn adopt_runtime_state_from(&self, old: &dyn Element) {}

    /// Run reconciliation work with any inherited state published by this
    /// element available in `ctx`.
    ///
    /// Scope elements override this so eager descendant rebuilds performed
    /// during state carry observe the same context as normal draw and layout.
    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        callback(ctx);
    }

    /// Mark this element (and its subtree) as needing a rebuild on the next
    /// frame.
    ///
    /// The default just recurses through `visit_children`; elements that
    /// actually hold a build closure (`StatefulElement`,
    /// `StatelessElement`) override this to flip their own dirty flag so
    /// `rebuild_if_dirty` re-runs `build()`. Called on window resize so
    /// `MediaQuery`-dependent widgets rebuild.
    fn mark_needs_rebuild(&self) {
        self.visit_children(&mut |child| {
            child.mark_needs_rebuild();
        });
    }

    /// Returns the last installed-tree generation observed by this subtree.
    ///
    /// The erased element owner records this value around rebuild and draw
    /// calls. Layout containers can use it to distinguish an unchanged child
    /// from one whose generated descendants were replaced, even when another
    /// branch advanced the global tree generation.
    #[inline]
    fn subtree_generation(&self) -> u64 {
        element_tree_generation()
    }

    /// Records the installed-tree generation that a reconciliation committed
    /// for this subtree.
    #[inline]
    #[allow(unused_variables)]
    fn set_subtree_generation(&self, generation: u64) {}
}
