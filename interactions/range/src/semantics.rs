/// Identifies the semantic role published by a range control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeRole {
    /// A control with one adjustable value.
    Slider,
    /// A control with independently adjustable lower and upper values.
    RangeSlider,
}

/// The value or values exposed by a range control's semantic node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SemanticRangeValue {
    /// The current value of a single slider.
    Single(f64),
    /// The lower and upper values of a range slider.
    Pair {
        /// The lower thumb value.
        lower: f64,
        /// The upper thumb value.
        upper: f64,
    },
}

/// Platform-neutral range metadata for an accessibility adapter.
///
/// The model deliberately contains no native accessibility types. A browser,
/// desktop, or test adapter can map [`Self::role`], the bounds, and
/// [`Self::value`] to its own semantic node representation. `invalid_range`
/// is retained even for rejected raw input so validation UIs can announce a
/// bad range without constructing a control from invalid state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeSemantics {
    role: RangeRole,
    min: f64,
    max: f64,
    step: f64,
    value: SemanticRangeValue,
    enabled: bool,
    invalid_range: bool,
}

impl RangeSemantics {
    /// Builds metadata for a single slider without panicking on invalid raw
    /// inputs.
    pub fn from_slider(min: f64, max: f64, step: f64, value: f64, enabled: bool) -> Self {
        let invalid_range =
            !valid_domain(min, max, step) || !value.is_finite() || !contains(min, max, value);
        Self {
            role: RangeRole::Slider,
            min,
            max,
            step,
            value: SemanticRangeValue::Single(value),
            enabled,
            invalid_range,
        }
    }

    /// Builds metadata for a range slider without panicking on invalid raw
    /// inputs.
    pub fn from_range_slider(
        min: f64,
        max: f64,
        step: f64,
        lower: f64,
        upper: f64,
        enabled: bool,
    ) -> Self {
        let invalid_range = !valid_domain(min, max, step)
            || !lower.is_finite()
            || !upper.is_finite()
            || !contains(min, max, lower)
            || !contains(min, max, upper)
            || lower > upper;
        Self {
            role: RangeRole::RangeSlider,
            min,
            max,
            step,
            value: SemanticRangeValue::Pair { lower, upper },
            enabled,
            invalid_range,
        }
    }

    /// Returns the semantic role.
    #[inline]
    pub fn role(&self) -> RangeRole {
        self.role
    }

    /// Returns the inclusive minimum.
    #[inline]
    pub fn min(&self) -> f64 {
        self.min
    }

    /// Returns the inclusive maximum.
    #[inline]
    pub fn max(&self) -> f64 {
        self.max
    }

    /// Returns the positive step.
    #[inline]
    pub fn step(&self) -> f64 {
        self.step
    }

    /// Returns the current single or paired value.
    #[inline]
    pub fn value(&self) -> SemanticRangeValue {
        self.value
    }

    /// Returns whether the control accepts user interaction.
    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Returns whether the raw range metadata is invalid.
    #[inline]
    pub fn invalid_range(&self) -> bool {
        self.invalid_range
    }
}

fn valid_domain(min: f64, max: f64, step: f64) -> bool {
    min.is_finite() && max.is_finite() && step.is_finite() && step > 0.0 && min <= max
}

fn contains(min: f64, max: f64, value: f64) -> bool {
    value.is_finite() && min <= value && value <= max
}
