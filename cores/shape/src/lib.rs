//! Small, validated, platform-neutral geometry values for custom paths.
//!
//! The crate deliberately knows nothing about widgets, canvases, GPU buffers,
//! or a particular tessellator. A [`ShapePath`] owns only finite commands and
//! its deterministic local bounds. Renderer adapters can turn those commands
//! into their own mesh or draw representation without making the geometry API
//! carry renderer types.

mod fit;
mod geometry;
mod hit_test;
mod paint;
mod path;

pub use fit::{FitError, ShapeFit};
pub use geometry::{Point, ShapeBounds, ShapeSize, ShapeTransform};
pub use hit_test::ShapeHitTest;
pub use paint::{
    DashSettings, FillRule, FillStyle, LineCap, LineJoin, PaintError, ShapeClip, ShapeColor,
    ShapeFill, ShapeLineCap, ShapeLineJoin, ShapeStyle, StrokeStyle,
};
pub use path::{
    ArcError, ShapeCommand, ShapeError, ShapeLimits, ShapePath, ShapePathBuilder, ShapePathId,
    ShapePolyline,
};

/// Maximum command count used by the default path builder.
pub const DEFAULT_MAX_COMMANDS: usize = 4_096;
/// Maximum number of contours used by the default path builder.
pub const DEFAULT_MAX_CONTOURS: usize = 512;
/// Maximum number of dash entries accepted by a stroke.
pub const DEFAULT_MAX_DASH_SEGMENTS: usize = 64;
/// Maximum absolute coordinate, radius, or extent accepted by a path.
pub const DEFAULT_MAX_ABS_COORDINATE: f32 = 1_000_000.0;
