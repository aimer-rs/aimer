//! [`MaskedDecorationBox`], which shows a decoration only where its child has coverage.

use crate::decoration_box::{ChildMask, decoration_element_with};
use crate::{BoxDecoration, Decoration};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, PortableWidget, RequiredChild, Widget};

/// Shows one decoration only where its child has coverage.
///
/// The decoration is painted through the child's shape instead of behind it:
/// where the child has pixels the decoration shows, and everywhere else, between
/// the glyphs of a text or outside a logo, nothing does. The child's own colors
/// are not drawn, only its coverage counts. This is the effect CSS calls
/// `background-clip: text`, and it is how a [`Gradient`](crate::Gradient) becomes
/// rainbow text. The decoration covers exactly the child's box, and the box has
/// no background of its own. Layout, events and focus pass through to the child
/// as with [`DecorationBox`](crate::DecorationBox).
///
/// # Cost
///
/// A masked box renders its decoration and its child into two offscreen targets
/// the size of the box and composites them, so it costs memory and a pass for
/// each. Keep masked boxes to the text or shape that needs them rather than a
/// whole page.
///
/// # Glass text
///
/// Any decoration works, including one that samples what is behind the box.
/// A [`BackdropFilter`](crate::BackdropFilter) masked by text blurs and tints
/// the backdrop only inside the glyphs:
///
/// ```
/// use aimer_decoration::{BackdropFilter, MaskedDecorationBox};
/// use aimer_container::SizedBox;
///
/// let glass = MaskedDecorationBox::new()
///     .decoration(BackdropFilter::new().blur_radius(10.0).brightness(1.5))
///     .child(SizedBox::new().width(160).height(48));
/// ```
///
/// Custom decoration widgets do not support portable lowering.
///
/// # Examples
///
/// ```
/// use aimer_decoration::{Gradient, MaskedDecorationBox};
/// use aimer_container::SizedBox;
///
/// // A rainbow that shows only where the child has coverage.
/// let widget = MaskedDecorationBox::new()
///     .decoration(Gradient::rainbow())
///     .child(SizedBox::new().width(160).height(48));
/// ```
pub struct MaskedDecorationBox<D = BoxDecoration, W = RequiredChild> {
    decoration: D,
    child: W,
}

impl Default for MaskedDecorationBox {
    fn default() -> Self {
        Self::new()
    }
}

impl MaskedDecorationBox {
    /// Creates an empty masked builder that requires a child.
    ///
    /// Finish with [`MaskedDecorationBox::child`] or
    /// [`MaskedDecorationBox::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            decoration: BoxDecoration::new(),
            child: RequiredChild,
        }
    }
}

impl<D: Decoration> MaskedDecorationBox<D> {
    /// Replaces the decoration, preserving its concrete type without boxing.
    #[inline]
    pub fn decoration<E: Decoration>(self, decoration: E) -> MaskedDecorationBox<E> {
        MaskedDecorationBox {
            decoration,
            child: self.child,
        }
    }

    /// Attaches the required child and completes the builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> MaskedDecorationBox<D, W> {
        MaskedDecorationBox {
            decoration: self.decoration,
            child,
        }
    }

    /// Attaches a child and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget
    where
        D: 'static,
    {
        self.child(child).boxed()
    }
}

impl<D: Decoration + 'static, W: Widget> PortableWidget for MaskedDecorationBox<D, W> {}

impl<D: Decoration + 'static, W: Widget> Widget for MaskedDecorationBox<D, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        decoration_element_with(
            self.decoration,
            self.child,
            ctx,
            "MaskedDecorationBox",
            ChildMask::DecorationByChild,
        )
    }

    fn debug_name(&self) -> &'static str {
        "MaskedDecorationBox"
    }
}
