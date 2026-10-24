use crate::decoration_box::{ChildMask, decoration_element_with};
use crate::{BoxDecoration, Decoration};
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, PortableWidget, RequiredChild, Widget};

/// Draws its child through the alpha of one decoration.
///
/// This is the mirror of [`MaskedDecorationBox`](crate::MaskedDecorationBox).
/// There the child's shape cuts the decoration out; here the decoration's alpha
/// cuts the child: each pixel of the child is scaled by the decoration's alpha at
/// that pixel. The decoration's own colors are never drawn, only its alpha counts,
/// so an [`OpacityGradient`](crate::OpacityGradient) fades a child out along a
/// ramp, a [`BorderRadius`](crate::BorderRadius) rounds it, and a
/// [`Gradient`](crate::Gradient) with transparent stops works the same way. The
/// decoration covers exactly the child's box, and the box has no background of its
/// own. Layout, events and focus pass through to the child as with
/// [`DecorationBox`](crate::DecorationBox).
///
/// A decoration that [paints nothing](Decoration::is_empty), such as the default
/// one or an [`OpacityGradient`](crate::OpacityGradient) without stops, leaves the
/// child as it is.
///
/// # Cost
///
/// The decoration and the child are rendered into two offscreen targets the size
/// of the box and composited, so the box costs memory and a pass for each. Keep it
/// to the list, image or panel that needs the fade rather than a whole page.
///
/// Custom decoration widgets do not support portable lowering.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{OpacityGradient, OpacityMaskBox};
///
/// // A child that fades to nothing toward the bottom.
/// let widget = OpacityMaskBox::new()
///     .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
///     .child(SizedBox::new().width(160).height(48));
/// ```
pub struct OpacityMaskBox<D = BoxDecoration, W = RequiredChild> {
    decoration: D,
    child: W,
}

impl Default for OpacityMaskBox {
    fn default() -> Self {
        Self::new()
    }
}

impl OpacityMaskBox {
    /// Creates an empty mask builder that requires a child.
    ///
    /// Finish with [`OpacityMaskBox::child`] or [`OpacityMaskBox::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            decoration: BoxDecoration::new(),
            child: RequiredChild,
        }
    }
}

impl<D: Decoration> OpacityMaskBox<D> {
    /// Replaces the decoration, preserving its concrete type without boxing.
    #[inline]
    pub fn decoration<E: Decoration>(self, decoration: E) -> OpacityMaskBox<E> {
        OpacityMaskBox {
            decoration,
            child: self.child,
        }
    }

    /// Attaches the required child and completes the builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> OpacityMaskBox<D, W> {
        OpacityMaskBox {
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

impl<D: Decoration + 'static, W: Widget> PortableWidget for OpacityMaskBox<D, W> {}

impl<D: Decoration + 'static, W: Widget> Widget for OpacityMaskBox<D, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        decoration_element_with(
            self.decoration,
            self.child,
            ctx,
            "OpacityMaskBox",
            ChildMask::ChildByDecoration,
        )
    }

    fn debug_name(&self) -> &'static str {
        "OpacityMaskBox"
    }
}
