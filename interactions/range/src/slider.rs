use std::{fmt, ops::Range};

use aimer_widget::{ChildBuilder, Widget};

use super::{
    RangeChangeCallback, RangeError, RangeSemantics, RangeSpec, RangeValue, ReversedBoundsPolicy,
    SliderKey,
};

/// A stateful, single-value range control model and widget.
///
/// The builder keeps a scalar `T` all the way through the public API while the
/// range policy uses a checked floating-point representation for geometry and
/// snapping. Pointer and keyboard input updates the widget's retained state
/// immediately; [`Self::on_change`] is an optional notification for an owner
/// that also wants to observe or persist those updates.
#[derive(Clone)]
pub struct Slider<T: RangeValue = f64> {
    range: Range<T>,
    step_value: T,
    value: T,
    spec: Result<RangeSpec, RangeError>,
    bounds_policy: ReversedBoundsPolicy,
    disabled: bool,
    width: f32,
    height: f32,
    track: Option<ChildBuilder>,
    trail: Option<ChildBuilder>,
    thumb: Option<ChildBuilder>,
    pub(crate) on_change: Option<RangeChangeCallback<T>>,
}

impl<T: RangeValue> Slider<T> {
    /// Creates a slider with the default range `0..1`, step `1`, and value `0`.
    ///
    /// Configure the control with [`Self::range`], [`Self::step`], and
    /// [`Self::value`]. The default is intentionally complete, so a slider can
    /// be used as a widget before any optional visual customization is added.
    #[inline]
    pub fn new() -> Self {
        let mut slider = Self {
            range: T::zero()..T::one(),
            step_value: T::one(),
            value: T::zero(),
            spec: Err(RangeError::NonPositiveStep { step: 0.0 }),
            bounds_policy: ReversedBoundsPolicy::Reject,
            disabled: false,
            width: 240.0,
            height: 44.0,
            track: None,
            trail: None,
            thumb: None,
            on_change: None,
        };
        slider.refresh_spec();
        slider
    }

    /// Sets the inclusive numeric range used by this slider.
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

    /// Sets the current value, clamped and snapped when the configuration is
    /// valid.
    #[inline]
    pub fn value(mut self, value: T) -> Self {
        self.value = value;
        self.canonicalize_value();
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
    ///
    /// Builder methods intentionally remain infallible so they compose in a
    /// normal widget chain. Call this method when configuration diagnostics
    /// should be surfaced before mounting the widget.
    #[inline]
    pub fn validate(&self) -> Result<(), RangeError> {
        self.spec.map(|_| ())
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

    /// Returns the current value supplied to the model.
    #[inline]
    pub fn current_value(&self) -> T {
        self.value
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

    /// Returns a slider configured with or without user interaction.
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

    /// Replaces the widget's track with a composable child.
    ///
    /// The child is retained across slider rebuilds. Its own bounds determine
    /// the track's drawn height and it is centered vertically in the slider.
    #[inline]
    pub fn track<W: Widget + 'static>(mut self, track: W) -> Self {
        self.track = Some(ChildBuilder::from_widget(track));
        self
    }

    /// Replaces the active trail with a composable child.
    ///
    /// The slider clips this child to the portion represented by the current
    /// value. When this slot is omitted, the slider uses the public
    /// [`crate::SliderTrail::new`] widget as its default.
    #[inline]
    pub fn trail<W: Widget + 'static>(mut self, trail: W) -> Self {
        self.trail = Some(ChildBuilder::from_widget(trail));
        self
    }

    /// Replaces the widget's thumb with a composable child.
    ///
    /// The child is retained across slider rebuilds and centered on the current
    /// track position. The built-in thumb is omitted when this slot is set.
    #[inline]
    pub fn thumb<W: Widget + 'static>(mut self, thumb: W) -> Self {
        self.thumb = Some(ChildBuilder::from_widget(thumb));
        self
    }

    /// Registers a callback for values proposed by pointer or keyboard input.
    ///
    /// The slider updates its own retained value first, then invokes this
    /// callback. An owner can use the callback for persistence, analytics, or
    /// controlled synchronization without being required for interaction.
    #[inline]
    pub fn on_change<F>(mut self, callback: F) -> Self
    where
        F: Fn(T) + 'static,
    {
        self.on_change = Some(std::rc::Rc::new(callback));
        self
    }

    /// Applies a new value from an owner, clamping and snapping it to policy.
    ///
    /// This setter remains available while disabled because a disabled control
    /// may still receive a value from its owner. It returns whether the
    /// canonical value changed.
    pub fn set_value(&mut self, value: T) -> Result<bool, RangeError> {
        let value = self.canonical_value(value)?;
        Ok(replace_if_changed(&mut self.value, value))
    }

    /// Moves by a number of configured steps when enabled.
    pub fn adjust_steps(&mut self, steps: i32) -> Result<bool, RangeError> {
        if self.disabled || steps == 0 {
            return Ok(false);
        }
        let target = self.value.to_f64() + self.step_value.to_f64() * f64::from(steps);
        let target = if target.is_finite() {
            target
        } else if steps.is_negative() {
            self.min().to_f64()
        } else {
            self.max().to_f64()
        };
        self.set_value(T::from_f64(target))
    }

    /// Handles the standard keyboard actions for a slider.
    ///
    /// Horizontal and vertical arrow keys both adjust by one step. Home and
    /// End move to the inclusive bounds; PageUp and PageDown adjust ten steps.
    pub fn handle_key(&mut self, key: SliderKey) -> Result<bool, RangeError> {
        if self.disabled {
            return Ok(false);
        }
        match key {
            SliderKey::ArrowLeft | SliderKey::ArrowDown => self.adjust_steps(-1),
            SliderKey::ArrowRight | SliderKey::ArrowUp => self.adjust_steps(1),
            SliderKey::Home => self.set_value(self.min()),
            SliderKey::End => self.set_value(self.max()),
            SliderKey::PageDown => self.adjust_steps(-10),
            SliderKey::PageUp => self.adjust_steps(10),
        }
    }

    /// Converts a pointer coordinate on a track to a stepped value.
    #[inline]
    pub fn value_at_position(&self, position: f64, track_length: f64) -> Result<T, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        Ok(T::from_f64(spec.value_at_position(position, track_length)?))
    }

    /// Converts a value to its coordinate on a track.
    #[inline]
    pub fn position_for_value(&self, value: T, track_length: f64) -> Result<f64, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        spec.position_for_value(value.to_f64(), track_length)
    }

    /// Applies a pointer coordinate when the slider is enabled.
    pub fn set_from_position(
        &mut self,
        position: f64,
        track_length: f64,
    ) -> Result<bool, RangeError> {
        let value = self.value_at_position(position, track_length)?;
        if self.disabled {
            return Ok(false);
        }
        self.set_value(value)
    }

    /// Returns the platform-neutral semantic metadata for this slider.
    #[inline]
    pub fn semantics(&self) -> RangeSemantics {
        let min = self.min().to_f64();
        let max = self.max().to_f64();
        let step = self.step_value.to_f64();
        RangeSemantics::from_slider(min, max, step, self.value.to_f64(), !self.disabled)
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

    pub(crate) fn thumb_child(&self) -> Option<ChildBuilder> {
        self.thumb.clone()
    }

    pub(crate) fn canonical_value(&self, value: T) -> Result<T, RangeError> {
        let spec = self.spec.as_ref().map_err(|error| *error)?;
        Ok(T::from_f64(spec.clamp_and_snap_field(
            super::RangeField::Value,
            value.to_f64(),
        )?))
    }

    fn refresh_spec(&mut self) {
        self.spec = RangeSpec::with_reversed_bounds_policy(
            self.range.start.to_f64(),
            self.range.end.to_f64(),
            self.step_value.to_f64(),
            self.bounds_policy,
        );
        self.canonicalize_value();
    }

    fn canonicalize_value(&mut self) {
        if let Ok(value) = self.canonical_value(self.value) {
            self.value = value;
        }
    }
}

impl<T: RangeValue> Default for Slider<T> {
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

impl<T: RangeValue + fmt::Debug> fmt::Debug for Slider<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Slider")
            .field("range", &self.range)
            .field("step", &self.step_value)
            .field("value", &self.value)
            .field("bounds_policy", &self.bounds_policy)
            .field("disabled", &self.disabled)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl<T: RangeValue + PartialEq> PartialEq for Slider<T> {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range
            && self.step_value == other.step_value
            && self.value == other.value
            && self.bounds_policy == other.bounds_policy
            && self.disabled == other.disabled
            && self.width == other.width
            && self.height == other.height
    }
}
