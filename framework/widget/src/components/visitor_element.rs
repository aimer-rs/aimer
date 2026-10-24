use crate::{Element, ElementId, Key};

pub trait VisitorElement {
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

    #[doc(hidden)]
    fn element_id(&self) -> Option<ElementId> {
        None
    }

    #[doc(hidden)]
    fn set_element_id(&self, _id: ElementId) {}
}
