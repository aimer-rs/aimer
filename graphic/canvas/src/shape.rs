//! Typed shape requests at the canvas boundary.
//!
//! `aimer_shape` remains renderer-free. This module is the small bridge that
//! carries a validated path and its paint metadata to a backend. Backends may
//! reject a request; the result is an explicit safe fallback rather than an
//! escape hatch to a platform drawing API.

use std::fmt;
use std::sync::Arc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_cupid::shape::{ShapeRenderRequest, build_scene};
use aimer_shape::{
    FillStyle, PaintError, ShapeBounds, ShapeClip, ShapeFit, ShapeHitTest, ShapePath, ShapePathId,
    ShapeSize, ShapeTransform, StrokeStyle,
};

use crate::Canvas;

/// A bounded, typed request to draw one validated shape.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawShape {
    path: Arc<ShapePath>,
    path_id: ShapePathId,
    transform: ShapeTransform,
    fill: Option<FillStyle>,
    stroke: Option<StrokeStyle>,
    clip: ShapeClip,
    opacity: f32,
    hit_test: ShapeHitTest,
}

impl DrawShape {
    /// Creates a request with identity transform, no paint, and no clip.
    #[inline]
    pub fn new(path: Arc<ShapePath>) -> Self {
        let path_id = path.id();
        Self {
            path,
            path_id,
            transform: ShapeTransform::identity(),
            fill: None,
            stroke: None,
            clip: ShapeClip::None,
            opacity: 1.0,
            hit_test: ShapeHitTest::None,
        }
    }

    /// Creates a request by retaining an owned validated path.
    #[inline]
    pub fn from_path(path: ShapePath) -> Self {
        Self::new(Arc::new(path))
    }

    /// Returns the immutable geometry.
    #[inline]
    pub fn path(&self) -> &ShapePath {
        self.path.as_ref()
    }

    /// Returns the retained geometry handle.
    #[inline]
    pub fn path_arc(&self) -> &Arc<ShapePath> {
        &self.path
    }

    /// Returns the stable geometry identity used by renderer caches.
    #[inline]
    pub const fn path_id(&self) -> ShapePathId {
        self.path_id
    }

    /// Replaces the local affine transform.
    #[inline]
    pub const fn transform(mut self, transform: ShapeTransform) -> Self {
        self.transform = transform;
        self
    }

    /// Replaces the optional fill.
    #[inline]
    pub const fn fill(mut self, fill: FillStyle) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Clears the fill.
    #[inline]
    pub const fn without_fill(mut self) -> Self {
        self.fill = None;
        self
    }

    /// Replaces the optional stroke.
    #[inline]
    pub fn stroke(mut self, stroke: StrokeStyle) -> Self {
        self.stroke = Some(stroke);
        self
    }

    /// Clears the stroke.
    #[inline]
    pub fn without_stroke(mut self) -> Self {
        self.stroke = None;
        self
    }

    /// Replaces the clipping policy.
    #[inline]
    pub fn clip(mut self, clip: ShapeClip) -> Self {
        self.clip = clip;
        self
    }

    /// Replaces the alpha multiplier. Invalid values are rejected by
    /// [`Self::validate`] and become [`ShapeFallback::Skip`] at submission.
    #[inline]
    pub const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Replaces pointer hit-test metadata without changing keyboard focus.
    #[inline]
    pub const fn hit_test(mut self, hit_test: ShapeHitTest) -> Self {
        self.hit_test = hit_test;
        self
    }

    /// Returns the optional fill.
    #[inline]
    pub const fn fill_style(&self) -> Option<FillStyle> {
        self.fill
    }

    /// Returns the optional stroke.
    #[inline]
    pub fn stroke_style(&self) -> Option<&StrokeStyle> {
        self.stroke.as_ref()
    }

    /// Returns the clipping policy.
    #[inline]
    pub fn clip_policy(&self) -> &ShapeClip {
        &self.clip
    }

    /// Returns the transform.
    #[inline]
    pub const fn transform_value(&self) -> ShapeTransform {
        self.transform
    }

    /// Returns the opacity.
    #[inline]
    pub const fn opacity_value(&self) -> f32 {
        self.opacity
    }

    /// Returns the pointer hit-test policy.
    #[inline]
    pub const fn hit_test_policy(&self) -> ShapeHitTest {
        self.hit_test
    }

    /// Validates every request value before it crosses a backend boundary.
    pub fn validate(&self) -> Result<(), ShapeDrawError> {
        if !self.transform.is_valid() {
            return Err(ShapeDrawError::InvalidTransform);
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(ShapeDrawError::InvalidOpacity);
        }
        if let Some(fill) = self.fill {
            fill.validate().map_err(ShapeDrawError::Paint)?;
        }
        if let Some(stroke) = &self.stroke {
            stroke.validate().map_err(ShapeDrawError::Paint)?;
        }
        if self.fill.is_none() && self.stroke.is_none() {
            return Err(ShapeDrawError::NoPaint);
        }
        Ok(())
    }

    /// Returns a safe non-drawing plan for invalid or unsupported requests.
    #[inline]
    pub fn safe_fallback(&self) -> ShapeDrawResult {
        ShapeDrawResult::Fallback(ShapeFallback::Skip)
    }

    /// Returns the fit transform used by a caller that supplies a target box.
    pub fn fit_transform(
        &self,
        fit: ShapeFit,
        target: ShapeSize,
    ) -> Result<ShapeTransform, aimer_shape::FitError> {
        fit.transform(self.path.bounds(), target)
    }
}

/// Errors found before a typed draw request reaches a backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeDrawError {
    /// The affine transform was non-finite or singular.
    InvalidTransform,
    /// The alpha multiplier was non-finite or outside `0..=1`.
    InvalidOpacity,
    /// The request contained no fill and no stroke.
    NoPaint,
    /// A paint value was invalid.
    Paint(PaintError),
    /// A backend does not support a requested clipping form.
    UnsupportedClip,
    /// A backend cannot submit the requested stroke representation.
    UnsupportedStroke,
}

impl fmt::Display for ShapeDrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTransform => f.write_str("shape transform is invalid"),
            Self::InvalidOpacity => f.write_str("shape opacity is invalid"),
            Self::NoPaint => f.write_str("shape draw request has no paint"),
            Self::Paint(error) => error.fmt(f),
            Self::UnsupportedClip => f.write_str("backend does not support this shape clip"),
            Self::UnsupportedStroke => f.write_str("backend does not support this shape stroke"),
        }
    }
}

impl std::error::Error for ShapeDrawError {}

/// Bounded behavior when a shape cannot be submitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeFallback {
    /// Skip shape paint; the retained child and tree semantics remain intact.
    Skip,
    /// Paint only a backend-owned bounds marker when a backend explicitly opts in.
    Bounds,
}

/// The result of a typed shape submission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShapeDrawResult {
    /// The request was converted into an existing canvas draw command.
    Submitted,
    /// The request was rejected and a safe fallback was selected.
    Fallback(ShapeFallback),
}

/// An inherent canvas bridge for typed shapes recorded in a retained v2 list.
impl Canvas {
    /// Records a typed shape as an SVG-backed local draw command.
    ///
    /// The request is validated and converted with Cupid's existing shape
    /// tessellation path. Unsupported clipping or strokes return a safe skip
    /// result without adding commands to the local list.
    pub fn draw_shape(&self, request: &DrawShape, viewport: ShapeSize) -> ShapeDrawResult {
        if request.validate().is_err() || !viewport.is_valid() {
            return request.safe_fallback();
        }
        let render_request = ShapeRenderRequest {
            path: request.path(),
            transform: request.transform_value(),
            fill: request.fill_style(),
            stroke: request.stroke_style(),
            clip: request.clip_policy(),
            opacity: request.opacity_value(),
            hit_test: request.hit_test_policy(),
        };
        let scene = match build_scene(&render_request, viewport) {
            Ok(scene) => scene,
            Err(_) => return ShapeDrawResult::Fallback(ShapeFallback::Skip),
        };
        let destination =
            aimer_cupid::utilities::Rect::new(0.0, 0.0, viewport.width, viewport.height);
        if matches!(request.clip_policy(), ShapeClip::Bounds) {
            let Some((clip_pos, clip_size)) =
                transformed_bounds(request.path().bounds(), request.transform_value())
            else {
                return ShapeDrawResult::Fallback(ShapeFallback::Skip);
            };
            self.push_clip(
                aimer_cupid::utilities::Rect::new(
                    clip_pos.x,
                    clip_pos.y,
                    clip_size.width,
                    clip_size.height,
                ),
                [0.0; 4],
            );
            self.draw_svg(scene, destination, Arc::from([]));
            self.pop_clip();
        } else {
            self.draw_svg(scene, destination, Arc::from([]));
        }
        ShapeDrawResult::Submitted
    }
}

/// Returns the axis-aligned bounds of a path after applying its local shape
/// transform. The canvas clip is expressed in the same local coordinates as
/// the SVG destination, so all four corners are considered for rotations and
/// reflected scales.
fn transformed_bounds(
    bounds: ShapeBounds,
    transform: ShapeTransform,
) -> Option<(Vec2d, ResolvedSize)> {
    let corners = [
        (bounds.min.x, bounds.min.y),
        (bounds.min.x, bounds.max.y),
        (bounds.max.x, bounds.min.y),
        (bounds.max.x, bounds.max.y),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for (x, y) in corners {
        let point = transform.transform_point(aimer_shape::Point::new(x, y));
        if !point.is_finite() {
            return None;
        }
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let width = max_x - min_x;
    let height = max_y - min_y;
    (width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0)
        .then_some((Vec2d { x: min_x, y: min_y }, ResolvedSize { width, height }))
}
