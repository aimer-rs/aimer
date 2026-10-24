//! Decorations and layout-transparent widgets for painting behind, over or through a child.

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
