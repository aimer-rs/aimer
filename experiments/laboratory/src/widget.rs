//! The short-lived side of the tree.
//!
//! A widget is a description of configuration built inside `build`, turned
//! into an element on the next line, and dropped immediately afterwards.
//! Nothing else ever reads it. That is the whole justification for the
//! consuming signature below: a value about to be destroyed has no reason to
//! be copied first.

pub(crate) mod erased;
pub(crate) mod retained;

use crate::element::AnyElement;
use crate::widget::erased::AnyWidget;

/// A configuration value that produces exactly one element.
///
/// # Consuming conversion
///
/// [`Widget::to_element`] takes `self` by value. Compared with a borrowing
/// signature this changes three things:
///
/// 1. fields move into the element instead of being cloned, so a widget
///    holding a `Vec`, `String`, or any other owning field costs no
///    allocation at build time;
/// 2. a widget that wants to rebuild itself later can store *itself* in the
///    element's closure rather than a copy, which removes the implicit
///    `Clone` bound a derive would otherwise need;
/// 3. widgets no longer have to be `Clone` at all — the doc test in the crate
///    root builds a deliberately non-`Clone` widget.
///
/// # Example
///
/// ```
/// use aimer_laboratory::{AnyElement, Element, Widget};
///
/// struct Spacer(f32);
///
/// struct SpacerElement(f32);
///
/// impl Element for SpacerElement {}
///
/// impl Widget for Spacer {
///     fn to_element(self) -> AnyElement {
///         AnyElement::new(SpacerElement(self.0))
///     }
/// }
///
/// assert_eq!(Spacer(8.0).boxed().debug_name(), "Unknown");
/// ```
pub trait Widget: 'static {
    /// Consumes this widget and produces its element.
    ///
    /// The widget is gone once this returns, so an implementation should move
    /// its fields into the element rather than clone them.
    fn to_element(self) -> AnyElement
    where
        Self: Sized;

    /// Returns a stable name used by assertions and diagnostics.
    fn debug_name(&self) -> &'static str {
        "Unknown"
    }

    /// Erases this widget into an [`AnyWidget`].
    #[inline]
    fn boxed(self) -> AnyWidget
    where
        Self: Sized,
    {
        AnyWidget::new(self)
    }

    /// Erases this widget into an [`AnyWidget`].
    #[inline]
    fn notype(self) -> AnyWidget
    where
        Self: Sized,
    {
        AnyWidget::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::Element;

    struct Named;

    struct NamedElement;

    impl Element for NamedElement {
        fn debug_name(&self) -> &'static str {
            "NamedElement"
        }
    }

    impl Widget for Named {
        fn to_element(self) -> AnyElement {
            AnyElement::new(NamedElement)
        }

        fn debug_name(&self) -> &'static str {
            "Named"
        }
    }

    #[test]
    fn a_widget_builds_its_element_directly() {
        assert_eq!(Named.to_element().debug_name(), "NamedElement");
    }

    #[test]
    fn erasing_preserves_the_widget_name() {
        assert_eq!(Named.boxed().debug_name(), "Named");
    }
}
