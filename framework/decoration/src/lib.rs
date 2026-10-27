//! Decorations and layout-transparent widgets for painting behind, over or through a child.
//!
//! # Decorations
//!
//! A [`Decoration`] records paint commands for a box and can describe the
//! geometry it implies. [`BoxDecoration`] is the all-in-one style (fill,
//! [`BoxBorder`], [`BoxOutline`], [`BorderRadius`] and [`BoxShadow`]); each of
//! those pieces, a [`Color`](aimer_color::prelude::Color), and the effects
//! [`Gradient`], [`OpacityGradient`], [`BackdropFilter`], [`LightEdge`] and
//! [`Magnification`] also work as decorations on their own. Slices and arrays of
//! decorations paint as an ordered stack, and [`AnyDecor`] erases the type.
//!
//! # Widgets
//!
//! | Widget | Paints the decoration |
//! |---|---|
//! | [`DecorationBox`] | behind the child |
//! | [`ForegroundDecorationBox`] | over the child |
//! | [`MultiDecorationBox`] | in stacks behind and over the child |
//! | [`MaskedDecorationBox`] | only where the child has coverage |
//! | [`OpacityMaskBox`] | as an alpha mask applied to the child |
//!
//! None of them changes layout, events or focus of the child.

mod border;
mod box_decoration;
mod decoration;
mod decoration_box;
mod foreground_decoration_box;
mod masked_decoration_box;
mod multi_decoration_box;
mod opacity_mask_box;
mod portable;

pub use border::{BorderSlice, BorderStyle, BoxBorder, BoxOutline, Stroke};
pub use box_decoration::BoxDecoration;
pub use box_decoration::border_radius::BorderRadius;
pub use box_decoration::box_shadow::{BoxShadow, ShadowSide};

pub use decoration_box::{
    BackdropFilter, DecorationBox, Gradient, LightEdge, Magnification, MagnificationStyle,
    OpacityGradient,
};

pub use decoration::{AnyDecor, Decoration};
pub use foreground_decoration_box::ForegroundDecorationBox;
pub use masked_decoration_box::MaskedDecorationBox;
pub use multi_decoration_box::MultiDecorationBox;
pub use opacity_mask_box::OpacityMaskBox;
