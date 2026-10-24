/// Layout bounds in logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Bounds {
    /// Creates finite bounds with non-negative width and height.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, BoundsError> {
        if !x.is_finite() || !y.is_finite() || !width.is_finite() || !height.is_finite() {
            return Err(BoundsError::NonFinite);
        }
        if width < 0.0 || height < 0.0 {
            return Err(BoundsError::NegativeSize { width, height });
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the horizontal origin.
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the vertical origin.
    pub const fn y(self) -> f32 {
        self.y
    }

    /// Returns the width.
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Returns the height.
    pub const fn height(self) -> f32 {
        self.height
    }
}

/// An invalid semantic bounds value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundsError {
    /// At least one coordinate or size was NaN or infinite.
    NonFinite,
    /// A width or height was negative.
    NegativeSize {
        /// The rejected width.
        width: f32,
        /// The rejected height.
        height: f32,
    },
}

/// A normalized opaque RGB color used only by contrast validation helpers.
///
/// The semantic tree never chooses colors. This type lets style and platform
/// adapters validate a proposed foreground/background pair without importing
/// a renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    red: f32,
    green: f32,
    blue: f32,
}

impl Color {
    /// Creates a color from components in the inclusive `0.0..=1.0` range.
    pub fn new(red: f32, green: f32, blue: f32) -> Result<Self, ColorError> {
        if !red.is_finite() || !green.is_finite() || !blue.is_finite() {
            return Err(ColorError::NonFinite);
        }
        if !(0.0..=1.0).contains(&red)
            || !(0.0..=1.0).contains(&green)
            || !(0.0..=1.0).contains(&blue)
        {
            return Err(ColorError::OutOfRange);
        }
        Ok(Self { red, green, blue })
    }

    /// Returns the red component.
    pub const fn red(self) -> f32 {
        self.red
    }

    /// Returns the green component.
    pub const fn green(self) -> f32 {
        self.green
    }

    /// Returns the blue component.
    pub const fn blue(self) -> f32 {
        self.blue
    }
}

/// An invalid color component.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColorError {
    /// At least one component was NaN or infinite.
    NonFinite,
    /// At least one component was outside `0.0..=1.0`.
    OutOfRange,
}

/// A documented minimum interactive target in logical units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchTargetPolicy {
    min_width: f32,
    min_height: f32,
}

impl TouchTargetPolicy {
    /// Creates a policy with finite, strictly positive dimensions.
    pub fn new(min_width: f32, min_height: f32) -> Result<Self, TouchTargetPolicyError> {
        if !min_width.is_finite() || !min_height.is_finite() {
            return Err(TouchTargetPolicyError::NonFinite);
        }
        if min_width <= 0.0 || min_height <= 0.0 {
            return Err(TouchTargetPolicyError::NotPositive {
                min_width,
                min_height,
            });
        }
        Ok(Self {
            min_width,
            min_height,
        })
    }

    /// Returns the minimum width.
    pub const fn min_width(self) -> f32 {
        self.min_width
    }

    /// Returns the minimum height.
    pub const fn min_height(self) -> f32 {
        self.min_height
    }
}

impl Default for TouchTargetPolicy {
    /// Uses the documented 44 by 44 logical-unit minimum target.
    fn default() -> Self {
        Self {
            min_width: 44.0,
            min_height: 44.0,
        }
    }
}

/// An invalid minimum touch-target policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TouchTargetPolicyError {
    /// At least one policy dimension was NaN or infinite.
    NonFinite,
    /// At least one policy dimension was zero or negative.
    NotPositive {
        /// The rejected minimum width.
        min_width: f32,
        /// The rejected minimum height.
        min_height: f32,
    },
}

/// The axis that failed a minimum target check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TouchTargetAxis {
    /// The target was too narrow.
    Width,
    /// The target was too short.
    Height,
}

/// A target-size validation failure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TouchTargetError {
    /// The axis that failed.
    pub axis: TouchTargetAxis,
    /// The actual dimension.
    pub actual: f32,
    /// The required dimension.
    pub minimum: f32,
}

/// Checks both dimensions against a minimum touch-target policy.
pub fn validate_touch_target(
    bounds: Bounds,
    policy: TouchTargetPolicy,
) -> Result<(), TouchTargetError> {
    if bounds.width() < policy.min_width() {
        return Err(TouchTargetError {
            axis: TouchTargetAxis::Width,
            actual: bounds.width(),
            minimum: policy.min_width(),
        });
    }
    if bounds.height() < policy.min_height() {
        return Err(TouchTargetError {
            axis: TouchTargetAxis::Height,
            actual: bounds.height(),
            minimum: policy.min_height(),
        });
    }
    Ok(())
}

/// A contrast-ratio validation failure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ContrastError {
    /// The requested minimum was outside the physically possible `1.0..=21.0`
    /// range.
    InvalidMinimum(f32),
    /// The calculated ratio was below the requested minimum.
    Insufficient {
        /// The calculated contrast ratio.
        ratio: f32,
        /// The requested minimum ratio.
        minimum: f32,
    },
}

/// Calculates the WCAG relative contrast ratio for two opaque colors.
pub fn contrast_ratio(first: Color, second: Color) -> Result<f32, ColorError> {
    let first_luminance = relative_luminance(first);
    let second_luminance = relative_luminance(second);
    let (lighter, darker) = if first_luminance >= second_luminance {
        (first_luminance, second_luminance)
    } else {
        (second_luminance, first_luminance)
    };
    Ok((lighter + 0.05) / (darker + 0.05))
}

/// Validates a contrast ratio against a finite WCAG-possible minimum.
pub fn validate_contrast(
    foreground: Color,
    background: Color,
    minimum: f32,
) -> Result<(), ContrastError> {
    if !minimum.is_finite() || !(1.0..=21.0).contains(&minimum) {
        return Err(ContrastError::InvalidMinimum(minimum));
    }
    let ratio = contrast_ratio(foreground, background).expect("validated colors are finite");
    if ratio + f32::EPSILON < minimum {
        return Err(ContrastError::Insufficient { ratio, minimum });
    }
    Ok(())
}

fn relative_luminance(color: Color) -> f32 {
    fn linearize(component: f32) -> f32 {
        if component <= 0.04045 {
            component / 12.92
        } else {
            ((component + 0.055) / 1.055).powf(2.4)
        }
    }

    0.2126 * linearize(color.red())
        + 0.7152 * linearize(color.green())
        + 0.0722 * linearize(color.blue())
}
