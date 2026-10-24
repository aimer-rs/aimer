use core::fmt;

use crate::{ShapeBounds, ShapeSize, ShapeTransform};

/// Errors returned when a fit transform cannot be computed safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FitError {
    /// Bounds or target size contained a non-finite value or negative extent.
    InvalidGeometry,
    /// A fit would require a non-finite transform.
    NonFiniteTransform,
}

impl fmt::Display for FitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGeometry => {
                f.write_str("shape bounds and target size must be finite and non-negative")
            }
            Self::NonFiniteTransform => f.write_str("shape fit produced a non-finite transform"),
        }
    }
}

impl std::error::Error for FitError {}

/// How a local path is fitted into a target rectangle.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ShapeFit {
    /// Keep local coordinates unchanged.
    #[default]
    None,
    /// Scale independently on both axes to fill the target.
    Fill,
    /// Preserve aspect ratio and fit completely inside the target.
    Contain,
    /// Preserve aspect ratio and cover the target.
    Cover,
    /// Preserve aspect ratio, but never enlarge the local bounds.
    ScaleDown,
}

impl ShapeFit {
    /// Computes a finite affine transform for `bounds` inside `target`.
    pub fn transform(
        self,
        bounds: ShapeBounds,
        target: ShapeSize,
    ) -> Result<ShapeTransform, FitError> {
        if !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || bounds.min.x > bounds.max.x
            || bounds.min.y > bounds.max.y
            || !target.is_valid()
        {
            return Err(FitError::InvalidGeometry);
        }

        if self == Self::None {
            return Ok(ShapeTransform::identity());
        }

        let local = bounds.size();
        let mut sx = 1.0;
        let mut sy = 1.0;
        match self {
            Self::None => {}
            Self::Fill => {
                sx = if local.width > 0.0 {
                    target.width / local.width
                } else {
                    1.0
                };
                sy = if local.height > 0.0 {
                    target.height / local.height
                } else {
                    1.0
                };
            }
            Self::Contain | Self::Cover | Self::ScaleDown => {
                let width_scale = if local.width > 0.0 {
                    target.width / local.width
                } else {
                    f32::INFINITY
                };
                let height_scale = if local.height > 0.0 {
                    target.height / local.height
                } else {
                    f32::INFINITY
                };
                let mut scale = match self {
                    Self::Cover => width_scale.max(height_scale),
                    _ => width_scale.min(height_scale),
                };
                if self == Self::ScaleDown {
                    scale = scale.min(1.0);
                }
                if !scale.is_finite() {
                    scale = 1.0;
                }
                sx = scale;
                sy = scale;
            }
        }

        let fitted_width = local.width * sx;
        let fitted_height = local.height * sy;
        let tx = (target.width - fitted_width) * 0.5 - bounds.min.x * sx;
        let ty = (target.height - fitted_height) * 0.5 - bounds.min.y * sy;
        let transform = ShapeTransform::scale_translate(sx, sy, tx, ty);
        if transform.is_valid() {
            Ok(transform)
        } else {
            Err(FitError::NonFiniteTransform)
        }
    }
}
