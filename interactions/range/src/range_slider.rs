use std::{fmt, ops::Range};

use aimer_widget::{ChildBuilder, Widget};

use super::{
    RangeError, RangePairChangeCallback, RangeSemantics, RangeSpec, RangeThumb, RangeValue,
    ReversedBoundsPolicy, SliderKey,
};

/// A stateful, two-thumb range control model and widget.
///
/// Both thumbs share one scalar `T` domain and cannot cross. Pointer and
/// keyboard input updates the widget's retained state immediately; the
/// optional [`Self::on_change`] callback observes the resulting ordered pair.
/// 
#[derive(Clone)]
pub struct RangeSlider<T: RangeValue = f64> {
    range: Range<T>,
    step_value: T,
    lower: T,
    upper: T,
    spec: Result<RangeSpec, RangeError>,
    bounds_policy: ReversedBoundsPolicy,
    disabled: bool,
    width: f32,
    height: f32,
    track: Option<ChildBuilder>,
    trail: Option<ChildBuilder>,
    lower_thumb: Option<ChildBuilder>,
    upper_thumb: Option<ChildBuilder>,
    pub(crate) on_change: Option<RangePairChangeCallback<T>>,
}

impl<T: RangeValue> RangeSlider<T> {
    /// Creates a range slider with the default range `0..1`, step `1`, and
    /// values `0..1`.
    #[inline]
    pub fn new() -> Self {
        let mut slider = Self {
            range: T::zero()..T::one(),
            step_value: T::one(),
            lower: T::zero(),
            upper: T::one(),
            spec: Err(RangeError::NonPositiveStep { step: 0.0 }),
            bounds_policy: ReversedBoundsPolicy::Reject,
            disabled: false,
            width: 240.0,
            height: 44.0,
            track: None,
            trail: None,
            lower_thumb: None,
            upper_thumb: None,
            on_change: None,
        };
        slider.refresh_spec();
        slider
    }

    /// Sets the inclusive numeric range shared by both thumbs.
    #[inline]
    pub fn range(mut self, range: Range<T>) -> Self {
        self.range = range;
        self.refresh_spec();
        self
    }

    /// Sets the positive distance between adjacent values.
    #[inline]
    pub fn step(mut self, step: T) -> Self {
        self.step_value = step;
        self.refresh_spec();
        self
    }

    /// Sets both thumb values from an inclusive range.
    #[inline]
    pub fn values(mut self, values: Range<T>) -> Self {
        self.lower = values.start;
        self.upper = values.end;
        self.canonicalize_values();
        self
    }

    /// Alias for [`Self::values`] that mirrors the single-slider builder.
    #[inline]
    pub fn value(self, values: Range<T>) -> Self {
        self.values(values)
    }

    /// Sets both thumb values as a `(lower, upper)` pair.
    #[inline]
    pub fn thumbs_value(mut self, lower: T, upper: T) -> Self {
        self.lower = lower;
        self.upper = upper;
        self.canonicalize_values();
        self
    }

    /// Sets how a reversed range is handled.
    #[inline]
    pub fn reversed_bounds_policy(mut self, policy: ReversedBoundsPolicy) -> Self {
        self.bounds_policy = policy;
        self.refresh_spec();
        self
    }

    /// Returns the current configuration error, if any.
    #[inline]
    pub fn validate(&self) -> Result<(), RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        if self.lower.to_f64() > self.upper.to_f64() {
            return Err(RangeError::ReversedValues {
                lower: self.lower.to_f64(),
                upper: self.upper.to_f64(),
            });
        }
        if spec.min() > spec.max() {
            return Err(RangeError::ReversedBounds {
                min: spec.min(),
                max: spec.max(),
            });
        }
        Ok(())
    }

    /// Returns the range supplied to [`Self::range`].
    #[inline]
    pub fn range_bounds(&self) -> Range<T> {
        self.range.clone()
    }

    /// Returns the effective lower bound.
    #[inline]
    pub fn min(&self) -> T {
        self.spec
            .as_ref()
            .map(|spec| T::from_f64(spec.min()))
            .unwrap_or(self.range.start)
    }

    /// Returns the effective upper bound.
    #[inline]
    pub fn max(&self) -> T {
        self.spec
            .as_ref()
            .map(|spec| T::from_f64(spec.max()))
            .unwrap_or(self.range.end)
    }

    /// Returns the configured step.
    #[inline]
    pub fn step_value(&self) -> T {
        self.step_value
    }

    /// Returns the lower thumb value.
    #[inline]
    pub fn lower(&self) -> T {
        self.lower
    }

    /// Returns the upper thumb value.
    #[inline]
    pub fn upper(&self) -> T {
        self.upper
    }

    /// Returns both current thumb values as an inclusive range.
    #[inline]
    pub fn current_values(&self) -> Range<T> {
        self.lower..self.upper
    }

    /// Returns the policy used for reversed bounds.
    #[inline]
    pub fn reversed_bounds_policy_value(&self) -> ReversedBoundsPolicy {
        self.bounds_policy
    }

    /// Returns whether pointer and keyboard input are ignored.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Enables or disables user interaction.
    #[inline]
    pub fn set_disabled(&mut self, disabled: bool) {
        self.disabled = disabled;
    }

    /// Returns a range slider configured with or without user interaction.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the logical width reserved by the widget implementation.
    ///
    /// Non-finite and negative values are ignored. A zero width is valid for a
    /// collapsed layout.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the logical height reserved by the widget implementation.
    ///
    /// Non-finite and negative values are ignored. A zero height is valid for
    /// a collapsed layout.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Replaces the widget's shared track with a composable child.
    #[inline]
    pub fn track<W: Widget + 'static>(mut self, track: W) -> Self {
        self.track = Some(ChildBuilder::from_widget(track));
        self
    }

    /// Replaces the active trail with a composable child.
    ///
    /// The range slider clips this child to the span between its two thumbs.
    /// When omitted, the slider uses the public [`crate::SliderTrail::new`] widget as
    /// its default.
    #[inline]
    pub fn trail<W: Widget + 'static>(mut self, trail: W) -> Self {
        self.trail = Some(ChildBuilder::from_widget(trail));
        self
    }

    /// Replaces the lower thumb with a composable child.
    #[inline]
    pub fn lower_thumb<W: Widget + 'static>(mut self, thumb: W) -> Self {
        self.lower_thumb = Some(ChildBuilder::from_widget(thumb));
        self
    }

    /// Replaces the upper thumb with a composable child.
    #[inline]
    pub fn upper_thumb<W: Widget + 'static>(mut self, thumb: W) -> Self {
        self.upper_thumb = Some(ChildBuilder::from_widget(thumb));
        self
    }

    /// Replaces both thumbs with independently retained composable children.
    #[inline]
    pub fn thumbs<L: Widget + 'static, U: Widget + 'static>(mut self, lower: L, upper: U) -> Self {
        self.lower_thumb = Some(ChildBuilder::from_widget(lower));
        self.upper_thumb = Some(ChildBuilder::from_widget(upper));
        self
    }

    /// Replaces both thumbs with copies of one composable child.
    ///
    /// Two independent retained elements are required because the lower and
    /// upper thumbs occupy different positions in the tree.
    #[inline]
    pub fn thumb<W: Widget + Clone + 'static>(self, thumb: W) -> Self {
        self.thumbs(thumb.clone(), thumb)
    }

    /// Registers a callback for ordered lower and upper values proposed by
    /// pointer or keyboard input.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn((T, T)) + 'static,
    {
        self.on_change = Some(std::rc::Rc::new(callback));
        self
    }

    /// Replaces both controlled values without allowing the pair to cross.
    pub fn set_values(&mut self, lower: T, upper: T) -> Result<bool, RangeError> {
        let lower = self.canonical_value(super::RangeField::LowerValue, lower)?;
        let upper = self.canonical_value(super::RangeField::UpperValue, upper)?;
        if lower.to_f64() > upper.to_f64() {
            return Err(RangeError::ReversedValues {
                lower: lower.to_f64(),
                upper: upper.to_f64(),
            });
        }
        let changed = self.lower != lower || self.upper != upper;
        self.lower = lower;
        self.upper = upper;
        Ok(changed)
    }

    /// Replaces the lower controlled value, clamping it to the upper thumb.
    pub fn set_lower(&mut self, lower: T) -> Result<bool, RangeError> {
        let lower = self.canonical_value(super::RangeField::LowerValue, lower)?;
        let lower = if lower <= self.upper {
            lower
        } else {
            self.upper
        };
        Ok(replace_if_changed(&mut self.lower, lower))
    }

    /// Replaces the upper controlled value, clamping it to the lower thumb.
    pub fn set_upper(&mut self, upper: T) -> Result<bool, RangeError> {
        let upper = self.canonical_value(super::RangeField::UpperValue, upper)?;
        let upper = if upper >= self.lower {
            upper
        } else {
            self.lower
        };
        Ok(replace_if_changed(&mut self.upper, upper))
    }

    /// Moves one thumb by a number of configured steps when enabled.
    pub fn adjust_thumb_by_steps(
        &mut self,
        thumb: RangeThumb,
        steps: i32,
    ) -> Result<bool, RangeError> {
        if self.disabled || steps == 0 {
            return Ok(false);
        }
        let target = match thumb {
            RangeThumb::Lower => self.lower.to_f64(),
            RangeThumb::Upper => self.upper.to_f64(),
        } + self.step_value.to_f64() * f64::from(steps);
        let target = if target.is_finite() {
            target
        } else if steps.is_negative() {
            self.min().to_f64()
        } else {
            self.max().to_f64()
        };
        match thumb {
            RangeThumb::Lower => self.set_lower(T::from_f64(target)),
            RangeThumb::Upper => self.set_upper(T::from_f64(target)),
        }
    }

    /// Handles standard keyboard actions for one thumb.
    pub fn handle_key(&mut self, thumb: RangeThumb, key: SliderKey) -> Result<bool, RangeError> {
        if self.disabled {
            return Ok(false);
        }
        match key {
            SliderKey::ArrowLeft | SliderKey::ArrowDown => self.adjust_thumb_by_steps(thumb, -1),
            SliderKey::ArrowRight | SliderKey::ArrowUp => self.adjust_thumb_by_steps(thumb, 1),
            SliderKey::PageDown => self.adjust_thumb_by_steps(thumb, -10),
            SliderKey::PageUp => self.adjust_thumb_by_steps(thumb, 10),
            SliderKey::Home => match thumb {
                RangeThumb::Lower => self.set_lower(self.min()),
                RangeThumb::Upper => self.set_upper(self.min()),
            },
            SliderKey::End => match thumb {
                RangeThumb::Lower => self.set_lower(self.max()),
                RangeThumb::Upper => self.set_upper(self.max()),
            },
        }
    }

    /// Converts a pointer coordinate to a stepped value in this slider's
    /// shared numeric domain.
    #[inline]
    pub fn value_at_position(&self, position: f64, track_length: f64) -> Result<T, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        Ok(T::from_f64(spec.value_at_position(position, track_length)?))
    }

    /// Converts a value to its coordinate on the shared track.
    #[inline]
    pub fn position_for_value(&self, value: T, track_length: f64) -> Result<f64, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        spec.position_for_value(value.to_f64(), track_length)
    }

    /// Applies a pointer coordinate to one thumb when enabled.
    pub fn set_thumb_from_position(
        &mut self,
        thumb: RangeThumb,
        position: f64,
        track_length: f64,
    ) -> Result<bool, RangeError> {
        let value = self.value_at_position(position, track_length)?;
        if self.disabled {
            return Ok(false);
        }
        match thumb {
            RangeThumb::Lower => self.set_lower(value),
            RangeThumb::Upper => self.set_upper(value),
        }
    }

    /// Returns the platform-neutral semantic metadata for this range slider.
    #[inline]
    pub fn semantics(&self) -> RangeSemantics {
        RangeSemantics::from_range_slider(
            self.min().to_f64(),
            self.max().to_f64(),
            self.step_value.to_f64(),
            self.lower.to_f64(),
            self.upper.to_f64(),
            !self.disabled,
        )
    }

    pub(crate) fn widget_width(&self) -> f32 {
        self.width
    }

    pub(crate) fn widget_height(&self) -> f32 {
        self.height
    }

    pub(crate) fn track_child(&self) -> Option<ChildBuilder> {
        self.track.clone()
    }

    pub(crate) fn trail_child(&self) -> Option<ChildBuilder> {
        self.trail.clone()
    }

    pub(crate) fn lower_thumb_child(&self) -> Option<ChildBuilder> {
        self.lower_thumb.clone()
    }

    pub(crate) fn upper_thumb_child(&self) -> Option<ChildBuilder> {
        self.upper_thumb.clone()
    }

    pub(crate) fn canonical_value(
        &self,
        field: super::RangeField,
        value: T,
    ) -> Result<T, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        Ok(T::from_f64(
            spec.clamp_and_snap_field(field, value.to_f64())?,
        ))
    }

    fn refresh_spec(&mut self) {
        self.spec = RangeSpec::with_reversed_bounds_policy(
            self.range.start.to_f64(),
            self.range.end.to_f64(),
            self.step_value.to_f64(),
            self.bounds_policy,
        );
        self.canonicalize_values();
    }

    fn canonicalize_values(&mut self) {
        let Ok(lower) = self.canonical_value(super::RangeField::LowerValue, self.lower) else {
            return;
        };
        let Ok(upper) = self.canonical_value(super::RangeField::UpperValue, self.upper) else {
            return;
        };
        self.lower = lower;
        self.upper = upper;
    }
}

impl<T: RangeValue> Default for RangeSlider<T> {
    fn default() -> Self {
        Self::new()
    }
}

fn replace_if_changed<T: PartialEq>(current: &mut T, next: T) -> bool {
    if *current == next {
        false
    } else {
        *current = next;
        true
    }
}

impl<T: RangeValue + fmt::Debug> fmt::Debug for RangeSlider<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RangeSlider")
            .field("range", &self.range)
            .field("step", &self.step_value)
            .field("lower", &self.lower)
            .field("upper", &self.upper)
            .field("bounds_policy", &self.bounds_policy)
            .field("disabled", &self.disabled)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl<T: RangeValue + PartialEq> PartialEq for RangeSlider<T> {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range
            && self.step_value == other.step_value
            && self.lower == other.lower
            && self.upper == other.upper
            && self.bounds_policy == other.bounds_policy
            && self.disabled == other.disabled
            && self.width == other.width
            && self.height == other.height
    }
}
