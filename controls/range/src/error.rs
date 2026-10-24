use std::fmt;

/// Identifies a numeric input rejected by a range-control operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeField {
    /// The lower bound of a range.
    Minimum,
    /// The upper bound of a range.
    Maximum,
    /// The distance between adjacent values.
    Step,
    /// A single slider value.
    Value,
    /// The lower thumb value of a range slider.
    LowerValue,
    /// The upper thumb value of a range slider.
    UpperValue,
    /// The pointer position on a track.
    Position,
    /// The physical length of a track.
    TrackLength,
}

impl fmt::Display for RangeField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Minimum => "minimum",
            Self::Maximum => "maximum",
            Self::Step => "step",
            Self::Value => "value",
            Self::LowerValue => "lower value",
            Self::UpperValue => "upper value",
            Self::Position => "position",
            Self::TrackLength => "track length",
        })
    }
}

/// An invalid configuration or coordinate supplied to a range control.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RangeError {
    /// An input must be finite, but was NaN or an infinity.
    NonFinite {
        /// The input that was rejected.
        field: RangeField,
        /// The non-finite value that was supplied.
        value: f64,
    },
    /// A step must be strictly greater than zero.
    NonPositiveStep {
        /// The invalid step.
        step: f64,
    },
    /// The minimum is greater than the maximum under the reject policy.
    ReversedBounds {
        /// The supplied minimum.
        min: f64,
        /// The supplied maximum.
        max: f64,
    },
    /// A range slider's lower value is greater than its upper value.
    ReversedValues {
        /// The supplied lower value.
        lower: f64,
        /// The supplied upper value.
        upper: f64,
    },
    /// A track length cannot be negative.
    NegativeTrackLength {
        /// The invalid track length.
        length: f64,
    },
}

impl fmt::Display for RangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { field, value } => {
                write!(formatter, "{field} must be finite, got {value}")
            }
            Self::NonPositiveStep { step } => {
                write!(formatter, "step must be greater than zero, got {step}")
            }
            Self::ReversedBounds { min, max } => {
                write!(formatter, "minimum {min} is greater than maximum {max}")
            }
            Self::ReversedValues { lower, upper } => {
                write!(
                    formatter,
                    "lower value {lower} is greater than upper value {upper}"
                )
            }
            Self::NegativeTrackLength { length } => {
                write!(formatter, "track length must not be negative, got {length}")
            }
        }
    }
}

impl std::error::Error for RangeError {}
