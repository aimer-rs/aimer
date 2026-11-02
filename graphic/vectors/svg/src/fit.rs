use std::str::FromStr;

use super::SvgTransform;

/// Horizontal and vertical alignment used by SVG `preserveAspectRatio`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgAspectAlign {
    /// Do not preserve the aspect ratio; independently scale both axes.
    None,
    /// Align the view box to the minimum x and minimum y edges.
    XMinYMin,
    /// Align the view box to the midpoint x and minimum y edges.
    XMidYMin,
    /// Align the view box to the maximum x and minimum y edges.
    XMaxYMin,
    /// Align the view box to the minimum x and midpoint y edges.
    XMinYMid,
    /// Align the view box to both midpoints.
    XMidYMid,
    /// Align the view box to the maximum x and midpoint y edges.
    XMaxYMid,
    /// Align the view box to the minimum x and maximum y edges.
    XMinYMax,
    /// Align the view box to the midpoint x and maximum y edges.
    XMidYMax,
    /// Align the view box to the maximum x and maximum y edges.
    XMaxYMax,
}

/// The sizing mode used after an SVG view box is aligned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgAspectMode {
    /// Scale until the complete view box is visible.
    Meet,
    /// Scale until the destination is covered, allowing view-box cropping.
    Slice,
}

/// The parsed SVG `preserveAspectRatio` value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SvgPreserveAspectRatio {
    /// The alignment anchor.
    pub align: SvgAspectAlign,
    /// Whether to contain (`meet`) or cover (`slice`) the destination.
    pub mode: SvgAspectMode,
}

impl Default for SvgPreserveAspectRatio {
    fn default() -> Self {
        Self {
            align: SvgAspectAlign::XMidYMid,
            mode: SvgAspectMode::Meet,
        }
    }
}

impl FromStr for SvgPreserveAspectRatio {
    type Err = SvgFitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut tokens = value
            .split(|character: char| character.is_ascii_whitespace() || character == ',')
            .filter(|token| !token.is_empty());
        let mut token = tokens
            .next()
            .ok_or_else(|| SvgFitError::InvalidPreserveAspectRatio("value is empty".to_owned()))?;

        // `defer` is a valid SVG grammar prefix for embedded resources. The
        // root document has no later resource to defer to, so it does not
        // alter the local fit policy.
        if token == "defer" {
            token = tokens.next().ok_or_else(|| {
                SvgFitError::InvalidPreserveAspectRatio("alignment is missing".to_owned())
            })?;
        }

        let align = match token {
            "none" => SvgAspectAlign::None,
            "xMinYMin" => SvgAspectAlign::XMinYMin,
            "xMidYMin" => SvgAspectAlign::XMidYMin,
            "xMaxYMin" => SvgAspectAlign::XMaxYMin,
            "xMinYMid" => SvgAspectAlign::XMinYMid,
            "xMidYMid" => SvgAspectAlign::XMidYMid,
            "xMaxYMid" => SvgAspectAlign::XMaxYMid,
            "xMinYMax" => SvgAspectAlign::XMinYMax,
            "xMidYMax" => SvgAspectAlign::XMidYMax,
            "xMaxYMax" => SvgAspectAlign::XMaxYMax,
            other => {
                return Err(SvgFitError::InvalidPreserveAspectRatio(format!(
                    "unknown alignment `{other}`"
                )));
            }
        };

        let mode = match tokens.next() {
            None => SvgAspectMode::Meet,
            Some("meet") => SvgAspectMode::Meet,
            Some("slice") => SvgAspectMode::Slice,
            Some(other) => {
                return Err(SvgFitError::InvalidPreserveAspectRatio(format!(
                    "unknown sizing mode `{other}`"
                )));
            }
        };
        if tokens.next().is_some() {
            return Err(SvgFitError::InvalidPreserveAspectRatio(
                "too many tokens".to_owned(),
            ));
        }

        Ok(Self { align, mode })
    }
}

/// The fit behavior applied to a document's view box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgFitPolicy {
    /// Scale x and y independently to fill the destination.
    Stretch,
    /// Apply the supplied SVG `preserveAspectRatio` behavior.
    PreserveAspectRatio(SvgPreserveAspectRatio),
}

impl Default for SvgFitPolicy {
    fn default() -> Self {
        Self::PreserveAspectRatio(SvgPreserveAspectRatio::default())
    }
}

/// A finite, positive SVG `viewBox` rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgViewBox {
    /// Minimum x coordinate in user space.
    pub x: f32,
    /// Minimum y coordinate in user space.
    pub y: f32,
    /// Width in user space.
    pub width: f32,
    /// Height in user space.
    pub height: f32,
}

impl SvgViewBox {
    /// Creates a validated view box.
    pub fn try_new(x: f32, y: f32, width: f32, height: f32) -> Result<Self, SvgFitError> {
        if ![x, y, width, height].into_iter().all(f32::is_finite) {
            return Err(SvgFitError::NonFinite("viewBox"));
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(SvgFitError::NonPositive("viewBox width and height"));
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Maps this view box into a finite positive destination rectangle.
    pub fn fit_transform(
        self,
        destination_width: f32,
        destination_height: f32,
        policy: SvgFitPolicy,
    ) -> Result<SvgTransform, SvgFitError> {
        if !destination_width.is_finite() || !destination_height.is_finite() {
            return Err(SvgFitError::NonFinite("fit destination"));
        }
        if destination_width <= 0.0 || destination_height <= 0.0 {
            return Err(SvgFitError::NonPositive("fit destination"));
        }

        let sx = destination_width / self.width;
        let sy = destination_height / self.height;
        let (sx, sy, extra_x, extra_y) = match policy {
            SvgFitPolicy::Stretch => (sx, sy, 0.0, 0.0),
            SvgFitPolicy::PreserveAspectRatio(preserve)
                if preserve.align == SvgAspectAlign::None =>
            {
                (sx, sy, 0.0, 0.0)
            }
            SvgFitPolicy::PreserveAspectRatio(preserve) => {
                let scale = match preserve.mode {
                    SvgAspectMode::Meet => sx.min(sy),
                    SvgAspectMode::Slice => sx.max(sy),
                };
                (
                    scale,
                    scale,
                    destination_width - self.width * scale,
                    destination_height - self.height * scale,
                )
            }
        };

        let (align_x, align_y) = match policy {
            SvgFitPolicy::Stretch
            | SvgFitPolicy::PreserveAspectRatio(SvgPreserveAspectRatio {
                align: SvgAspectAlign::None,
                ..
            }) => (0.0, 0.0),
            SvgFitPolicy::PreserveAspectRatio(preserve) => alignment(preserve.align),
        };
        let transform = SvgTransform {
            sx,
            sy,
            tx: -self.x * sx + extra_x * align_x,
            ty: -self.y * sy + extra_y * align_y,
            ..SvgTransform::default()
        };
        if transform.is_finite() {
            Ok(transform)
        } else {
            Err(SvgFitError::NonFinite("fit transform"))
        }
    }
}

impl FromStr for SvgViewBox {
    type Err = SvgFitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let values = value
            .split(|character: char| character.is_ascii_whitespace() || character == ',')
            .filter(|token| !token.is_empty())
            .map(|token| {
                token
                    .parse::<f32>()
                    .map_err(|_| SvgFitError::InvalidViewBox(format!("invalid number `{token}`")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != 4 {
            return Err(SvgFitError::InvalidViewBox(
                "expected four numbers".to_owned(),
            ));
        }
        Self::try_new(values[0], values[1], values[2], values[3])
    }
}

/// Errors returned when an SVG fit contract is not safe to evaluate.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SvgFitError {
    /// A coordinate or size was not finite.
    #[error("SVG {0} contains a non-finite value")]
    NonFinite(&'static str),
    /// A required positive size was zero or negative.
    #[error("SVG {0} must be positive")]
    NonPositive(&'static str),
    /// The view box grammar was malformed.
    #[error("invalid SVG viewBox: {0}")]
    InvalidViewBox(String),
    /// The preserve-aspect-ratio grammar was malformed.
    #[error("invalid SVG preserveAspectRatio: {0}")]
    InvalidPreserveAspectRatio(String),
}

fn alignment(align: SvgAspectAlign) -> (f32, f32) {
    match align {
        SvgAspectAlign::None => (0.0, 0.0),
        SvgAspectAlign::XMinYMin => (0.0, 0.0),
        SvgAspectAlign::XMidYMin => (0.5, 0.0),
        SvgAspectAlign::XMaxYMin => (1.0, 0.0),
        SvgAspectAlign::XMinYMid => (0.0, 0.5),
        SvgAspectAlign::XMidYMid => (0.5, 0.5),
        SvgAspectAlign::XMaxYMid => (1.0, 0.5),
        SvgAspectAlign::XMinYMax => (0.0, 1.0),
        SvgAspectAlign::XMidYMax => (0.5, 1.0),
        SvgAspectAlign::XMaxYMax => (1.0, 1.0),
    }
}
