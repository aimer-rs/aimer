use std::ops::{Deref, DerefMut};

use aimer_rubick::{Rubick, UiAllocator};

use super::Element;
use super::node::ElementNode;

/// An owned, type-erased [`Element`], stored as a thin two-word owner.
///
/// Elements are the retained side of the tree and are large: the smallest
/// documented layout is well over a hundred bytes, so no realistic inline
/// capacity would ever avoid their allocation. `AnyElement` therefore asks
/// [`aimer_rubick::Rubick`] for the minimum capacity of one word, which is
/// exactly the heap pointer. The owner costs two words instead of carrying an
/// inline buffer that could never be used, which matters because the retained
/// tree holds one owner per node for the lifetime of the application.
///
/// `AnyElement` is a thin wrapper around that owner and is exactly its size.
/// Borrowing through `Deref` or `AsRef` provides a `dyn Element` view with
/// normal dynamic dispatch and no adapter call. Construct one with
/// [`Element::boxed`] or [`AnyElement::new`].
#[repr(transparent)]
pub struct AnyElement(Rubick<dyn Element, 4>);

impl AnyElement {
    /// Erases `element`, giving it a fresh [`ElementId`](super::ElementId).
    ///
    /// This is what [`Element::boxed`] calls; it exists so an element can be
    /// erased without importing the trait.
    #[inline]
    pub fn new<E: Element + 'static>(element: E) -> Self {
        element.boxed()
    }

    /// Erases `element` using the supplied application UI allocator.
    ///
    /// The retained element keeps its allocation source alive until it is
    /// dropped, including when it outlives the allocator scope.
    #[inline]
    pub fn new_in<E: Element + 'static>(element: E, allocator: &UiAllocator) -> Self {
        element.boxed_in(allocator)
    }

    /// Wraps an already identified node.
    #[inline]
    pub(super) fn from_node<E: Element + 'static>(node: ElementNode<E>) -> Self {
        Self(Rubick::erase(node))
    }

    /// Wraps an already identified node, with heap storage from `allocator`.
    #[inline]
    pub(super) fn from_node_in<E: Element + 'static>(
        node: ElementNode<E>,
        allocator: &UiAllocator,
    ) -> Self {
        Self(Rubick::erase_in(node, allocator))
    }

    /// Returns whether the element is stored inside this handle.
    ///
    /// Elements are large, so this is `false` for every real element.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_inline()
    }

    /// Returns whether the element lives in a separate heap block.
    #[inline]
    pub fn is_heap(&self) -> bool {
        self.0.is_heap()
    }
}

impl Deref for AnyElement {
    type Target = dyn Element;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for AnyElement {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl AsRef<dyn Element> for AnyElement {
    #[inline]
    fn as_ref(&self) -> &(dyn Element + 'static) {
        &*self.0
    }
}

impl AsMut<dyn Element> for AnyElement {
    #[inline]
    fn as_mut(&mut self) -> &mut (dyn Element + 'static) {
        &mut *self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrapper_adds_nothing_to_the_owner_it_wraps() {
        assert_eq!(size_of::<AnyElement>(), size_of::<Rubick<dyn Element, 4>>());
        assert_eq!(align_of::<AnyElement>(), align_of::<Rubick<dyn Element, 4>>());
        assert_eq!(
            size_of::<Option<AnyElement>>(),
            size_of::<Option<Rubick<dyn Element, 4>>>(),
            "an optional child must keep whatever niche the owner has"
        );
    }
}
