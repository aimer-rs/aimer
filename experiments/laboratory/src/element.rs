//! The long-lived side of the tree.
//!
//! An element is created once from a widget and then survives many frames:
//! layout, paint, event dispatch, and reconciliation all read from it. Because
//! it outlives the widget that produced it, it can never borrow from that
//! widget — it has to own everything it needs, which is precisely why the
//! widget hands its fields over instead of lending them.

use std::ptr;

use aimer_rubick::{ErasedFrom, Rubick};

/// Machine words an [`AnyElement`] reserves for its payload.
///
/// Four words matches the framework default. Elements are created once and
/// kept, so a larger inline budget buys less here than it does for widgets,
/// which are created and destroyed on every build.
const ELEMENT_WORDS: usize = 4;

/// A node of the retained tree.
///
/// The trait is intentionally tiny: the laboratory studies ownership, not
/// layout. Anything a real element does — measuring, painting, reconciling —
/// would be an additional method on this trait and would not change how the
/// element is created.
pub trait Element: 'static {
    /// Returns a stable name used by assertions and diagnostics.
    fn debug_name(&self) -> &'static str {
        "Unknown"
    }

    /// Rebuilds this element's subtree in place.
    ///
    /// A composing element keeps the widget that produced it and re-runs it
    /// here. Because the element owns the *original* widget rather than a
    /// copy of it, rebuilding needs no `Clone` bound anywhere in the tree.
    /// Leaf elements have nothing to rebuild and use the default.
    fn rebuild(&mut self) {}
}

// SAFETY: The template is `null::<E>()` coerced to the target, so it carries
// exactly `E`'s vtable and a null data address.
unsafe impl<E: Element> ErasedFrom<E> for dyn Element {
    const TEMPLATE: *const Self = ptr::null::<E>() as *const dyn Element;
}

/// An owned, type-erased element.
///
/// The owner is deliberately private: the laboratory uses `aimer_rubick` but
/// does not re-export it, so the storage strategy stays an implementation
/// detail of this crate.
pub struct AnyElement(Rubick<dyn Element, ELEMENT_WORDS>);

impl AnyElement {
    /// Erases a concrete element.
    ///
    /// The element is stored inside this handle when it fits
    /// [`ELEMENT_WORDS`], and in a pooled block otherwise.
    #[inline]
    pub fn new<E: Element>(element: E) -> Self {
        Self(Rubick::erase(element))
    }

    /// Returns the erased element's diagnostic name.
    #[inline]
    pub fn debug_name(&self) -> &'static str {
        self.0.debug_name()
    }

    /// Rebuilds the erased element's subtree in place.
    #[inline]
    pub fn rebuild(&mut self) {
        self.0.rebuild();
    }

    /// Returns `true` when the element needs no separate allocation.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.0.is_inline()
    }

    /// Returns the address of the erased element.
    ///
    /// This is the element's identity: an experiment that has to tell *reuse*
    /// apart from *rebuilding an identical subtree* compares addresses, because
    /// a rebuilt element is a different element even when it looks the same.
    #[inline]
    pub fn address(&self) -> *const () {
        &*self.0 as *const dyn Element as *const ()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Marker;

    impl Element for Marker {
        fn debug_name(&self) -> &'static str {
            "Marker"
        }
    }

    struct Wide(#[allow(dead_code)] [usize; ELEMENT_WORDS + 1]);

    impl Element for Wide {}

    #[test]
    fn erased_elements_keep_their_identity() {
        let element = AnyElement::new(Marker);

        assert_eq!(element.debug_name(), "Marker");
        assert!(element.is_inline());
    }

    #[test]
    fn oversized_elements_fall_back_to_pooled_storage() {
        let element = AnyElement::new(Wide([0; ELEMENT_WORDS + 1]));

        assert_eq!(element.debug_name(), "Unknown");
        assert!(!element.is_inline());
    }
}
