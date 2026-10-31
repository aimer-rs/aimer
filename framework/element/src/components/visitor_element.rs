use crate::{Element, ElementId, Key};
// Only the intra-doc links below name these.
#[cfg(doc)]
use crate::{Drawable, EventElement, LayoutElement, Rebuildable};

/// How the rest of the framework walks an element's children, and how it
/// names the element itself.
///
/// This is the base of the [`Element`] family: [`EventElement`],
/// [`LayoutElement`], [`Rebuildable`] and [`Drawable`] are all defined in terms
/// of it, and their default methods reach the subtree through
/// [`visit_children`](VisitorElement::visit_children). An element that has
/// children therefore implements this trait first, and the defaults of the
/// other traits (invalidating layout, marking for rebuild, rebuilding dirty
/// descendants) then work without further code.
///
/// Only [`debug_name`](VisitorElement::debug_name) is required. The remaining
/// methods are identity hooks used by reconciliation; most elements keep their
/// defaults.
pub trait VisitorElement {
    /// Visits the children that are painted, laid out and rebuilt with this
    /// element, in structural order.
    ///
    /// This is the *visual* view of the subtree. The defaults of
    /// [`LayoutElement::invalidate_layout`], [`Rebuildable::mark_needs_rebuild`]
    /// and [`Rebuildable::rebuild_if_dirty`] walk it, so an element that holds
    /// children and does not report them here is silently skipped by all three.
    /// Event delivery has its own views, [`EventElement::event_children`] and
    /// [`EventElement::hit_test_children`], which default to this one.
    ///
    /// A leaf keeps the default, which visits nothing. The visitor is called
    /// synchronously and must not be retained.
    #[allow(unused_variables)]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {}

    /// Visits children in retained paint order and reports each child's stable
    /// source index for layout-aware v2 geometry.
    ///
    /// Most elements paint children in structural order. Layered containers
    /// can override this visitor while keeping `visit_children` unchanged for
    /// reconciliation and event structure.
    #[doc(hidden)]
    fn visit_retained_v2_children<'a>(
        &'a self,
        visitor: &mut dyn FnMut(usize, &'a dyn Element),
    ) {
        let mut index = 0;
        self.visit_children(&mut |child| {
            visitor(index, child);
            index += 1;
        });
    }

    /// Names the element's type for diagnostics and reconciliation.
    ///
    /// Reconciliation treats two elements as the same widget only when their
    /// concrete type, this name and their [`reconciliation_key`] agree, so the
    /// name must be stable for a given kind of element and distinct between
    /// kinds that can occupy the same position. It is also what appears in
    /// frame statistics and unmapped-draw reports; a short type name such as
    /// `"Container"` is conventional.
    ///
    /// [`reconciliation_key`]: VisitorElement::reconciliation_key
    fn debug_name(&self) -> &'static str;

    /// Returns the `TypeId` of the concrete element type, enabling runtime
    /// type checks without the `Reconcilable` trait. Used by the state-carrying
    /// logic to identify `StatefulElement`s in the element tree.
    fn element_type_id(&self) -> std::any::TypeId {
        // Default returns a dummy that never matches any real element type.
        std::any::TypeId::of::<()>()
    }

    /// Returns the key used to preserve logical identity during reconciliation.
    fn reconciliation_key(&self) -> Option<&Key> {
        None
    }

    /// Names the retained element this one is a placement of, if it is one.
    ///
    /// Two placements that name the same element stand for one subtree seen
    /// twice, so there is nothing to carry, match or compare between them.
    #[doc(hidden)]
    fn retained_identity(&self) -> Option<usize> {
        None
    }

    /// Whether this retained element is a slot its owner declared with
    /// `BuildContext::retain`.
    ///
    /// A slot is a part the owner named and keeps, so it is never the stand-in
    /// for a different retained part that happened to occupy its position.
    #[doc(hidden)]
    fn is_retained_slot(&self) -> bool {
        false
    }

    /// Returns the identity this element carries, if it carries one.
    ///
    /// Elements are given an [`ElementId`] when they are erased with
    /// [`Element::boxed`]; the erasing wrapper answers this, so a concrete
    /// element keeps the default. [`Element::id`] is the infallible form for
    /// elements that are known to be erased.
    #[doc(hidden)]
    fn element_id(&self) -> Option<ElementId> {
        None
    }

    /// Replaces this element's identity.
    ///
    /// Reconciliation calls this to hand the id of the element being replaced
    /// to its compatible successor, which is what keeps an element's identity
    /// stable across rebuilds. Only the erasing wrapper stores the id; the
    /// default ignores it.
    #[doc(hidden)]
    fn set_element_id(&self, _id: ElementId) {}
}
