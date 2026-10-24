use std::sync::Arc;

use super::{SvgColor, SvgTransform};

/// Coordinate space used by a gradient.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgGradientUnits {
    /// Coordinates are relative to the painted object's bounding box.
    ObjectBoundingBox,
    /// Coordinates are expressed in the current user space.
    UserSpaceOnUse,
}

/// Repeat behavior after a gradient reaches its final stop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgSpreadMethod {
    /// Clamp the edge colors outside the gradient interval.
    Pad,
    /// Reflect the gradient on each repetition.
    Reflect,
    /// Repeat the gradient from its first stop.
    Repeat,
}

/// A finite gradient stop retained from an SVG document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgGradientStop {
    /// Stop position in the normalized range 0..=1.
    pub offset: f32,
    /// Stop color, including `stop-opacity`.
    pub color: SvgColor,
}

impl SvgGradientStop {
    /// Returns whether the stop can be safely passed to a renderer.
    pub fn is_finite(self) -> bool {
        self.offset.is_finite() && self.color.is_finite()
    }
}

/// A parsed linear or radial SVG gradient.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgGradient {
    /// A linear gradient.
    Linear {
        /// Source id used by `url(#id)` paints.
        id: Arc<str>,
        /// Start x coordinate.
        x1: f32,
        /// Start y coordinate.
        y1: f32,
        /// End x coordinate.
        x2: f32,
        /// End y coordinate.
        y2: f32,
        /// Coordinate interpretation.
        units: SvgGradientUnits,
        /// Gradient-local transform.
        transform: SvgTransform,
        /// Out-of-range spread behavior.
        spread: SvgSpreadMethod,
        /// Ordered normalized stops.
        stops: Arc<[SvgGradientStop]>,
    },
    /// A radial gradient.
    Radial {
        /// Source id used by `url(#id)` paints.
        id: Arc<str>,
        /// Center x coordinate.
        cx: f32,
        /// Center y coordinate.
        cy: f32,
        /// Outer radius.
        radius: f32,
        /// Focal x coordinate.
        fx: f32,
        /// Focal y coordinate.
        fy: f32,
        /// Focal radius.
        focal_radius: f32,
        /// Coordinate interpretation.
        units: SvgGradientUnits,
        /// Gradient-local transform.
        transform: SvgTransform,
        /// Out-of-range spread behavior.
        spread: SvgSpreadMethod,
        /// Ordered normalized stops.
        stops: Arc<[SvgGradientStop]>,
    },
}

impl SvgGradient {
    /// Returns the source id used to reference this gradient.
    pub fn id(&self) -> &str {
        match self {
            Self::Linear { id, .. } | Self::Radial { id, .. } => id,
        }
    }

    /// Returns the retained stops in source order.
    pub fn stops(&self) -> &[SvgGradientStop] {
        match self {
            Self::Linear { stops, .. } | Self::Radial { stops, .. } => stops,
        }
    }

    /// Returns whether every numeric value is finite.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Linear {
                x1,
                y1,
                x2,
                y2,
                transform,
                stops,
                ..
            } => {
                [*x1, *y1, *x2, *y2].into_iter().all(f32::is_finite)
                    && transform.is_finite()
                    && stops.iter().all(|stop| stop.is_finite())
            }
            Self::Radial {
                cx,
                cy,
                radius,
                fx,
                fy,
                focal_radius,
                transform,
                stops,
                ..
            } => {
                [*cx, *cy, *radius, *fx, *fy, *focal_radius]
                    .into_iter()
                    .all(f32::is_finite)
                    && transform.is_finite()
                    && stops.iter().all(|stop| stop.is_finite())
            }
        }
    }

    /// Samples the gradient at a point in the painted node's local coordinates.
    ///
    /// `bounds` is the node's local geometry bounds. Radial sampling accounts
    /// for the focal center and focal radius.
    pub fn sample_at(&self, x: f32, y: f32, bounds: [f32; 4]) -> Option<SvgColor> {
        self.color_at_position(self.position_at(x, y, bounds)?)
    }

    /// Returns the normalized spread-adjusted gradient coordinate at a point.
    pub fn position_at(&self, x: f32, y: f32, bounds: [f32; 4]) -> Option<f32> {
        let (transform, units) = match self {
            Self::Linear {
                transform, units, ..
            }
            | Self::Radial {
                transform, units, ..
            } => (*transform, *units),
        };
        let (mut x, mut y) = match units {
            SvgGradientUnits::UserSpaceOnUse => (x, y),
            SvgGradientUnits::ObjectBoundingBox => {
                if bounds[2] <= 0.0 || bounds[3] <= 0.0 {
                    return None;
                }
                ((x - bounds[0]) / bounds[2], (y - bounds[1]) / bounds[3])
            }
        };
        (x, y) = transform.inverse()?.transform_point(x, y);
        let (position, spread) = match self {
            Self::Linear {
                x1,
                y1,
                x2,
                y2,
                spread,
                ..
            } => {
                let dx = x2 - x1;
                let dy = y2 - y1;
                let length_squared = dx * dx + dy * dy;
                if length_squared <= f32::EPSILON {
                    (1.0, *spread)
                } else {
                    (((x - x1) * dx + (y - y1) * dy) / length_squared, *spread)
                }
            }
            Self::Radial {
                cx,
                cy,
                radius,
                fx,
                fy,
                focal_radius,
                spread,
                ..
            } => {
                if *radius <= 0.0 {
                    return None;
                }
                (
                    radial_position(x, y, *cx, *cy, *radius, *fx, *fy, *focal_radius)?,
                    *spread,
                )
            }
        };
        let position = match spread {
            SvgSpreadMethod::Pad => position.clamp(0.0, 1.0),
            SvgSpreadMethod::Repeat => position.rem_euclid(1.0),
            SvgSpreadMethod::Reflect => {
                let value = position.rem_euclid(2.0);
                if value > 1.0 { 2.0 - value } else { value }
            }
        };
        position.is_finite().then_some(position)
    }

    /// Samples the ordered stops at a normalized coordinate.
    pub fn color_at_position(&self, position: f32) -> Option<SvgColor> {
        if !position.is_finite() {
            return None;
        }
        let stops = self.stops();
        if stops.is_empty() {
            return None;
        }
        let first = stops[0];
        if position <= first.offset {
            return Some(first.color);
        }
        for pair in stops.windows(2) {
            let [left, right] = pair else { continue };
            if position <= right.offset {
                let width = right.offset - left.offset;
                let amount = if width <= f32::EPSILON {
                    1.0
                } else {
                    ((position - left.offset) / width).clamp(0.0, 1.0)
                };
                return Some(interpolate_color(left.color, right.color, amount));
            }
        }
        stops.last().map(|stop| stop.color)
    }
}

fn radial_position(
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    radius: f32,
    fx: f32,
    fy: f32,
    focal_radius: f32,
) -> Option<f32> {
    let center_delta = [cx - fx, cy - fy];
    let point_delta = [x - fx, y - fy];
    let radius_delta = radius - focal_radius;
    let a = center_delta[0] * center_delta[0] + center_delta[1] * center_delta[1]
        - radius_delta * radius_delta;
    let b = -2.0
        * (point_delta[0] * center_delta[0]
            + point_delta[1] * center_delta[1]
            + focal_radius * radius_delta);
    let c = point_delta[0] * point_delta[0] + point_delta[1] * point_delta[1]
        - focal_radius * focal_radius;
    if ![a, b, c].into_iter().all(f32::is_finite) {
        return None;
    }

    if a.abs() <= f32::EPSILON {
        if b.abs() <= f32::EPSILON {
            return None;
        }
        let position = -c / b;
        return position.is_finite().then_some(position);
    }
    let discriminant = b * b - 4.0 * a * c;
    if !discriminant.is_finite() || discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    let denominator = 2.0 * a;
    let first = (-b - root) / denominator;
    let second = (-b + root) / denominator;
    let position = first.max(second);
    position.is_finite().then_some(position)
}

fn interpolate_color(from: SvgColor, to: SvgColor, amount: f32) -> SvgColor {
    SvgColor {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: from.a + (to.a - from.a) * amount,
    }
}

/// A paint retained by the parser, including paints the current GPU path has
/// not yet submitted.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgPaint {
    /// A solid color.
    Solid(SvgColor),
    /// A linear gradient.
    Linear(SvgGradient),
    /// A radial gradient.
    Radial(SvgGradient),
    /// A local pattern reference retained for SVG tile rendering.
    Pattern { id: Arc<str> },
}

impl SvgPaint {
    /// Returns whether all retained numeric values are finite.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Solid(color) => color.is_finite(),
            Self::Linear(gradient) | Self::Radial(gradient) => gradient.is_finite(),
            Self::Pattern { .. } => true,
        }
    }
}
