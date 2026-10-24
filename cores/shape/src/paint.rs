use core::fmt;
use std::sync::Arc;

use crate::{ShapePath, ShapeTransform};

/// An RGBA paint color with channels in the inclusive `0..=1` range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeColor {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel.
    pub a: f32,
}

impl ShapeColor {
    /// Opaque black.
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    /// Transparent black.
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// Creates a color value. Call [`Self::validate`] before crossing a
    /// renderer boundary when values did not come from a trusted constant.
    #[inline]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Creates an opaque RGB color.
    #[inline]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    /// Creates a color from 8-bit channels.
    #[inline]
    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Returns whether every channel is finite and in range.
    #[inline]
    pub const fn is_valid(self) -> bool {
        self.r.is_finite()
            && self.g.is_finite()
            && self.b.is_finite()
            && self.a.is_finite()
            && self.r >= 0.0
            && self.r <= 1.0
            && self.g >= 0.0
            && self.g <= 1.0
            && self.b >= 0.0
            && self.b <= 1.0
            && self.a >= 0.0
            && self.a <= 1.0
    }

    /// Validates the color before it is retained or rendered.
    pub fn validate(self) -> Result<Self, PaintError> {
        if self.is_valid() {
            Ok(self)
        } else {
            Err(PaintError::InvalidColor)
        }
    }
}

/// Errors returned by paint constructors and validators.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaintError {
    /// A channel was non-finite or outside `0..=1`.
    InvalidColor,
    /// A stroke width was non-finite, negative, or zero.
    InvalidStrokeWidth,
    /// A miter limit was non-finite or smaller than one.
    InvalidMiterLimit,
    /// A dash entry was non-finite, non-positive, out of range, or the complete pattern was zero.
    InvalidDash,
    /// The dash array exceeded the explicit bounded limit.
    TooManyDashSegments { limit: usize },
    /// A dash limit exceeded the crate's renderer-safe maximum.
    InvalidDashLimit { limit: usize },
    /// A requested opacity was not finite or in range.
    InvalidOpacity,
    /// A transform was non-finite or singular.
    InvalidTransform,
}

impl fmt::Display for PaintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidColor => f.write_str("shape color must be finite and in 0..=1"),
            Self::InvalidStrokeWidth => f.write_str("stroke width must be finite and positive"),
            Self::InvalidMiterLimit => f.write_str("miter limit must be finite and at least one"),
            Self::InvalidDash => f.write_str(
                "dash entries must be finite, positive, and bounded with a non-zero sum",
            ),
            Self::TooManyDashSegments { limit } => {
                write!(f, "dash array exceeds the {limit}-entry limit")
            }
            Self::InvalidDashLimit { limit } => {
                write!(f, "dash limit cannot exceed {limit} entries")
            }
            Self::InvalidOpacity => f.write_str("opacity must be finite and in 0..=1"),
            Self::InvalidTransform => {
                f.write_str("shape transform must be finite and non-singular")
            }
        }
    }
}

impl std::error::Error for PaintError {}

/// The winding rule used to fill a path.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FillRule {
    /// Non-zero winding fills a region when signed crossings do not cancel.
    #[default]
    NonZero,
    /// Even-odd winding alternates filled and empty regions at crossings.
    EvenOdd,
}

/// Alias retaining the domain-oriented name used by the shape plan.
pub type ShapeFill = FillStyle;

/// A validated solid fill description.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FillStyle {
    /// Fill color.
    pub color: ShapeColor,
    /// Fill winding rule.
    pub rule: FillRule,
}

impl FillStyle {
    /// Creates a fill with the non-zero winding rule.
    #[inline]
    pub const fn solid(color: ShapeColor) -> Self {
        Self {
            color,
            rule: FillRule::NonZero,
        }
    }

    /// Creates a fill with an explicit rule.
    #[inline]
    pub const fn new(color: ShapeColor, rule: FillRule) -> Self {
        Self { color, rule }
    }

    /// Replaces the fill rule.
    #[inline]
    pub const fn with_rule(mut self, rule: FillRule) -> Self {
        self.rule = rule;
        self
    }

    /// Validates the color before renderer submission.
    #[inline]
    pub fn validate(self) -> Result<Self, PaintError> {
        self.color.validate()?;
        Ok(self)
    }
}

/// Stroke cap behavior at open path endpoints.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LineCap {
    /// Ends exactly at the endpoint.
    #[default]
    Butt,
    /// Adds a semicircular endpoint.
    Round,
    /// Extends the endpoint by half the stroke width.
    Square,
}

/// Stroke join behavior at path corners.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum LineJoin {
    /// Extends edges until they meet, bounded by the miter limit.
    #[default]
    Miter,
    /// Rounds the corner.
    Round,
    /// Cuts the corner off.
    Bevel,
    /// Clips a miter when it reaches its limit.
    MiterClip,
}

/// Aliases for callers that prefer the fully qualified shape names.
pub type ShapeLineCap = LineCap;
/// Alias for callers that prefer the fully qualified shape names.
pub type ShapeLineJoin = LineJoin;

/// A bounded dash pattern in logical path units.
#[derive(Clone, Debug, PartialEq)]
pub struct DashSettings {
    segments: Arc<[f32]>,
    offset: f32,
}

impl DashSettings {
    /// Creates a solid stroke pattern with no dash entries.
    #[inline]
    pub fn solid() -> Self {
        Self {
            segments: Arc::from([]),
            offset: 0.0,
        }
    }

    /// Validates and creates a dash pattern using the default entry limit.
    pub fn new(segments: impl IntoIterator<Item = f32>, offset: f32) -> Result<Self, PaintError> {
        Self::with_limit(segments, offset, crate::DEFAULT_MAX_DASH_SEGMENTS)
    }

    /// Validates and creates a dash pattern with an explicit entry limit.
    pub fn with_limit(
        segments: impl IntoIterator<Item = f32>,
        offset: f32,
        limit: usize,
    ) -> Result<Self, PaintError> {
        if limit > crate::DEFAULT_MAX_DASH_SEGMENTS {
            return Err(PaintError::InvalidDashLimit {
                limit: crate::DEFAULT_MAX_DASH_SEGMENTS,
            });
        }
        if !offset.is_finite() || offset.abs() > crate::DEFAULT_MAX_ABS_COORDINATE {
            return Err(PaintError::InvalidDash);
        }
        let mut values = Vec::new();
        for value in segments {
            if values.len() >= limit {
                return Err(PaintError::TooManyDashSegments { limit });
            }
            values.push(value);
        }
        if values.iter().any(|value| {
            !value.is_finite() || *value <= 0.0 || *value > crate::DEFAULT_MAX_ABS_COORDINATE
        }) || (!values.is_empty() && {
            let total = values.iter().copied().sum::<f32>();
            !total.is_finite() || total <= 0.0
        }) {
            return Err(PaintError::InvalidDash);
        }
        Ok(Self {
            segments: values.into(),
            offset,
        })
    }

    /// Returns the validated dash entries.
    #[inline]
    pub fn segments(&self) -> &[f32] {
        &self.segments
    }

    /// Returns the dash phase offset.
    #[inline]
    pub const fn offset(&self) -> f32 {
        self.offset
    }

    /// Returns whether the pattern is solid.
    #[inline]
    pub fn is_solid(&self) -> bool {
        self.segments.is_empty()
    }
}

impl Default for DashSettings {
    fn default() -> Self {
        Self::solid()
    }
}

/// A validated path stroke description.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    /// Stroke color.
    pub color: ShapeColor,
    /// Stroke width in local units.
    pub width: f32,
    /// Endpoint cap.
    pub line_cap: LineCap,
    /// Corner join.
    pub line_join: LineJoin,
    /// Miter length limit.
    pub miter_limit: f32,
    dash: DashSettings,
}

impl StrokeStyle {
    /// Creates a solid stroke after validating its width and color.
    pub fn new(width: f32, color: ShapeColor) -> Result<Self, PaintError> {
        let style = Self {
            color,
            width,
            line_cap: LineCap::default(),
            line_join: LineJoin::default(),
            miter_limit: 4.0,
            dash: DashSettings::solid(),
        };
        style.validate().map(|()| style)
    }

    /// Alias for [`Self::new`].
    #[inline]
    pub fn try_new(width: f32, color: ShapeColor) -> Result<Self, PaintError> {
        Self::new(width, color)
    }

    /// Replaces the stroke color, retaining validation for later submission.
    #[inline]
    pub const fn with_color(mut self, color: ShapeColor) -> Self {
        self.color = color;
        self
    }

    /// Replaces the line cap.
    #[inline]
    pub const fn with_line_cap(mut self, line_cap: LineCap) -> Self {
        self.line_cap = line_cap;
        self
    }

    /// Replaces the line join.
    #[inline]
    pub const fn with_line_join(mut self, line_join: LineJoin) -> Self {
        self.line_join = line_join;
        self
    }

    /// Replaces the miter limit after validating it.
    pub fn with_miter_limit(mut self, miter_limit: f32) -> Result<Self, PaintError> {
        if !miter_limit.is_finite() || miter_limit < 1.0 {
            return Err(PaintError::InvalidMiterLimit);
        }
        self.miter_limit = miter_limit;
        Ok(self)
    }

    /// Replaces the dash pattern after validating it.
    pub fn with_dash(
        mut self,
        segments: impl IntoIterator<Item = f32>,
        offset: f32,
    ) -> Result<Self, PaintError> {
        self.dash = DashSettings::new(segments, offset)?;
        Ok(self)
    }

    /// Returns the dash settings.
    #[inline]
    pub fn dash(&self) -> &DashSettings {
        &self.dash
    }

    /// Validates all stroke values.
    pub fn validate(&self) -> Result<(), PaintError> {
        if !self.color.is_valid() {
            return Err(PaintError::InvalidColor);
        }
        if !self.width.is_finite()
            || self.width <= 0.0
            || self.width > crate::DEFAULT_MAX_ABS_COORDINATE
        {
            return Err(PaintError::InvalidStrokeWidth);
        }
        if !self.miter_limit.is_finite()
            || self.miter_limit < 1.0
            || self.miter_limit > crate::DEFAULT_MAX_ABS_COORDINATE
        {
            return Err(PaintError::InvalidMiterLimit);
        }
        DashSettings::with_limit(
            self.dash.segments.iter().copied(),
            self.dash.offset,
            crate::DEFAULT_MAX_DASH_SEGMENTS,
        )?;
        Ok(())
    }
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self::new(1.0, ShapeColor::BLACK).expect("default stroke is valid")
    }
}

/// Clipping policy carried by a typed draw request.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ShapeClip {
    /// Do not add a shape-owned clip.
    #[default]
    None,
    /// Clip to the path's local bounds.
    Bounds,
    /// Clip to another validated path.
    Path(Arc<ShapePath>),
}

/// A complete renderer-independent paint model for a path.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeStyle {
    /// Local path transform.
    pub transform: ShapeTransform,
    /// Optional fill.
    pub fill: Option<FillStyle>,
    /// Optional stroke.
    pub stroke: Option<StrokeStyle>,
    /// Shape-owned clipping policy.
    pub clip: ShapeClip,
    /// Alpha multiplier.
    pub opacity: f32,
}

impl Default for ShapeStyle {
    fn default() -> Self {
        Self {
            transform: ShapeTransform::identity(),
            fill: None,
            stroke: None,
            clip: ShapeClip::None,
            opacity: 1.0,
        }
    }
}

impl ShapeStyle {
    /// Validates the transform, optional paints, and opacity.
    pub fn validate(&self) -> Result<(), PaintError> {
        if !self.transform.is_valid() {
            return Err(PaintError::InvalidTransform);
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(PaintError::InvalidOpacity);
        }
        if let Some(fill) = self.fill {
            fill.validate()?;
        }
        if let Some(stroke) = &self.stroke {
            stroke.validate()?;
        }
        Ok(())
    }
}
