use super::{RangeError, RangeField, ReversedBoundsPolicy};

/// The validated numeric domain shared by a [`Slider`](super::Slider) and a
/// [`RangeSlider`](super::RangeSlider).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeSpec {
    min: f64,
    max: f64,
    step: f64,
    bounds_policy: ReversedBoundsPolicy,
}

impl RangeSpec {
    /// Creates a range using [`ReversedBoundsPolicy::Reject`].
    pub fn new(min: f64, max: f64, step: f64) -> Result<Self, RangeError> {
        Self::with_reversed_bounds_policy(min, max, step, ReversedBoundsPolicy::Reject)
    }

    /// Creates a range with an explicit policy for reversed bounds.
    ///
    /// [`ReversedBoundsPolicy::Reject`] preserves the caller's mistake as an
    /// error. [`ReversedBoundsPolicy::Normalize`] swaps the two bounds before
    /// validating values, so the effective range always increases from
    /// `min()` to `max()`.
    pub fn with_reversed_bounds_policy(
        min: f64,
        max: f64,
        step: f64,
        bounds_policy: ReversedBoundsPolicy,
    ) -> Result<Self, RangeError> {
        require_finite(RangeField::Minimum, min)?;
        require_finite(RangeField::Maximum, max)?;
        require_finite(RangeField::Step, step)?;
        if step <= 0.0 {
            return Err(RangeError::NonPositiveStep { step });
        }

        let (min, max) = if min > max {
            match bounds_policy {
                ReversedBoundsPolicy::Reject => {
                    return Err(RangeError::ReversedBounds { min, max });
                }
                ReversedBoundsPolicy::Normalize => (max, min),
            }
        } else {
            (min, max)
        };

        Ok(Self {
            min,
            max,
            step,
            bounds_policy,
        })
    }

    /// Returns the effective lower bound.
    #[inline]
    pub fn min(&self) -> f64 {
        self.min
    }

    /// Returns the effective upper bound.
    #[inline]
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Returns the positive distance between adjacent values.
    #[inline]
    pub fn step(&self) -> f64 {
        self.step
    }

    /// Returns the policy used when this range was built.
    #[inline]
    pub fn reversed_bounds_policy(&self) -> ReversedBoundsPolicy {
        self.bounds_policy
    }

    /// Clamps a finite input to the range and rounds it to the nearest step.
    ///
    /// Rounding is measured from `min()`. Because the offset is non-negative,
    /// halfway values round toward the next larger step. The maximum remains
    /// reachable even when it is not itself a step boundary.
    pub fn clamp_and_snap(&self, value: f64) -> Result<f64, RangeError> {
        self.clamp_and_snap_field(RangeField::Value, value)
    }

    pub(crate) fn clamp_and_snap_field(
        &self,
        field: RangeField,
        value: f64,
    ) -> Result<f64, RangeError> {
        require_finite(field, value)?;

        if self.min == self.max {
            return Ok(self.min);
        }
        if value <= self.min {
            return Ok(self.min);
        }
        if value >= self.max {
            return Ok(self.max);
        }

        let offset = quotient_difference(value, self.min, self.step);
        let rounded_steps = offset.round();
        let snapped_offset = rounded_steps * self.step;
        let snapped = self.min + snapped_offset;
        if snapped.is_finite() {
            Ok(snapped.clamp(self.min, self.max))
        } else {
            // A finite endpoint difference can still overflow while computing
            // an intermediate offset. In that extreme case the only safe
            // representable result is the endpoint in the direction of the
            // rounded value.
            Ok(if rounded_steps.is_sign_negative() {
                self.min
            } else {
                self.max
            })
        }
    }

    /// Converts a pointer coordinate into a clamped, stepped value.
    ///
    /// `position` is measured from the start of a track and `track_length`
    /// must be non-negative. A zero-length track is valid and maps every
    /// position to the minimum.
    pub fn value_at_position(&self, position: f64, track_length: f64) -> Result<f64, RangeError> {
        require_finite(RangeField::Position, position)?;
        validate_track_length(track_length)?;

        if self.min == self.max || track_length == 0.0 {
            return Ok(self.min);
        }

        let clamped_position = position.clamp(0.0, track_length);
        let fraction = clamped_position / track_length;
        let value = interpolate(self.min, self.max, fraction);
        self.clamp_and_snap_field(RangeField::Value, value)
    }

    /// Converts a value to its pointer coordinate on a track.
    ///
    /// The value is first clamped and stepped using the same rules as
    /// [`Self::clamp_and_snap`], making this operation the inverse of
    /// [`Self::value_at_position`] up to the configured step.
    pub fn position_for_value(&self, value: f64, track_length: f64) -> Result<f64, RangeError> {
        validate_track_length(track_length)?;
        let value = self.clamp_and_snap_field(RangeField::Value, value)?;
        if self.min == self.max || track_length == 0.0 {
            return Ok(0.0);
        }

        let fraction = difference_ratio(value, self.min, self.max);
        Ok(track_length * fraction)
    }
}

fn require_finite(field: RangeField, value: f64) -> Result<(), RangeError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(RangeError::NonFinite { field, value })
    }
}

fn validate_track_length(length: f64) -> Result<(), RangeError> {
    require_finite(RangeField::TrackLength, length)?;
    if length < 0.0 {
        return Err(RangeError::NegativeTrackLength { length });
    }
    Ok(())
}

fn quotient_difference(value: f64, origin: f64, divisor: f64) -> f64 {
    let difference = value - origin;
    if difference.is_finite() {
        return difference / divisor;
    }

    let scale = value.abs().max(origin.abs()).max(divisor);
    (value / scale - origin / scale) / (divisor / scale)
}

fn difference_ratio(value: f64, min: f64, max: f64) -> f64 {
    let difference = max - min;
    let offset = value - min;
    if difference.is_finite() && offset.is_finite() {
        return (offset / difference).clamp(0.0, 1.0);
    }

    let scale = max.abs().max(min.abs()).max(value.abs());
    ((value / scale - min / scale) / (max / scale - min / scale)).clamp(0.0, 1.0)
}

fn interpolate(min: f64, max: f64, fraction: f64) -> f64 {
    let interpolated = min * (1.0 - fraction) + max * fraction;
    if interpolated.is_finite() {
        interpolated
    } else if fraction <= 0.0 {
        min
    } else {
        max
    }
}
