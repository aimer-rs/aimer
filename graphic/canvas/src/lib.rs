mod canvas;
mod paint_background;
#[doc(hidden)]
pub use paint_background::composite_background;
pub mod material;
pub mod shape;

pub use canvas::TextInteractionLayout;
pub use canvas::{
    Canvas, CanvasContext, CanvasRendering, FontFamily, FontStyle, FrameCanvas, InnerCanvas, Mat3,
    MorphParams, MorphStyle, RETAINED_LAYER_MAX_BYTES, RETAINED_LAYER_MAX_DIMENSION,
    RETAINED_LAYER_MAX_TILES_PER_FRAME, RETAINED_LAYER_TILE_SIZE, Rect, RenderColor,
    RetainedDrawList, RetainedLayerContent, RetainedLayerPadding, TextHorizontalAlign,
    TextOverflowMode, next_retained_layer_id,
};
pub use material::{
    GradientShape, MAX_GRADIENT_STOPS, MagnificationStyle, MaterialClip, MaterialDrawRequest,
    MaterialGradient, MaterialKind, MaterialMotionPolicy, record_material,
};
pub use shape::{DrawShape, ShapeDrawError, ShapeDrawResult, ShapeFallback};
