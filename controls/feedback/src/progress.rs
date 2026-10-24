use std::time::Duration;

use aimer_container::SizedBox;
use aimer_space::Stack;
use aimer_style::TextStyle;
use aimer_text::Text;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{AnyElement, PortableWidget, Widget};

/// Motion preference consumed by feedback indicators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MotionPolicy {
    /// Permit normal indicator animation.
    #[default]
    Full,
    /// Avoid advancing animated feedback while retaining a visible state cue.
    Reduced,
}

/// The non-interactive state of a progress indicator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProgressState {
    /// A normalized finite value in the inclusive range `0.0..=1.0`.
    Determinate(f32),
    /// Work is ongoing but no normalized completion value is available.
    Indeterminate,
}

/// An invalid determinate progress value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressError {
    /// The value was NaN or infinite.
    NonFinite,
    /// The value was outside the inclusive range `0.0..=1.0`.
    OutOfRange,
}

/// Accessibility-facing value range for a [`ProgressIndicator`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressSemantics {
    /// The minimum normalized value, always `0.0`.
    pub min: f32,
    /// The maximum normalized value, always `1.0`.
    pub max: f32,
    /// The current normalized value, or `None` while indeterminate.
    pub current: Option<f32>,
}

/// A non-interactive determinate or indeterminate progress model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressIndicator {
    state: ProgressState,
    motion_policy: MotionPolicy,
    width: f32,
    height: f32,
    track_color: Color,
    progress_color: Color,
}

impl ProgressIndicator {
    /// Creates an indeterminate progress indicator.
    #[inline]
    pub const fn new() -> Self {
        Self {
            state: ProgressState::Indeterminate,
            motion_policy: MotionPolicy::Full,
            width: 240.0,
            height: 8.0,
            track_color: Color::Rgba(210, 216, 226, 255),
            progress_color: Color::Rgba(35, 110, 220, 255),
        }
    }

    /// Creates a determinate progress indicator after validating its fraction.
    pub fn determinate(value: f32) -> Result<Self, ProgressError> {
        validate_fraction(value)?;
        let mut indicator = Self::new();
        indicator.state = ProgressState::Determinate(value);
        Ok(indicator)
    }

    /// Returns an indeterminate progress indicator.
    #[inline]
    pub const fn indeterminate() -> Self {
        Self::new()
    }

    /// Changes the indicator to a validated determinate value.
    pub fn set_determinate(&mut self, value: f32) -> Result<(), ProgressError> {
        validate_fraction(value)?;
        self.state = ProgressState::Determinate(value);
        Ok(())
    }

    /// Changes the indicator to indeterminate state.
    #[inline]
    pub fn set_indeterminate(&mut self) {
        self.state = ProgressState::Indeterminate;
    }

    /// Sets the reduced-motion preference.
    #[inline]
    pub fn set_motion_policy(&mut self, policy: MotionPolicy) {
        self.motion_policy = policy;
    }

    /// Returns the current motion preference.
    #[inline]
    pub const fn motion_policy(self) -> MotionPolicy {
        self.motion_policy
    }

    /// Returns the current determinate or indeterminate state.
    #[inline]
    pub const fn state(self) -> ProgressState {
        self.state
    }

    /// Returns the normalized fraction, or `None` for indeterminate state.
    #[inline]
    pub const fn fraction(self) -> Option<f32> {
        match self.state {
            ProgressState::Determinate(value) => Some(value),
            ProgressState::Indeterminate => None,
        }
    }

    /// Returns a platform-neutral semantic snapshot for the indicator.
    #[inline]
    pub const fn semantics(self) -> ProgressSemantics {
        ProgressSemantics {
            min: 0.0,
            max: 1.0,
            current: self.fraction(),
        }
    }

    /// Sets the indicator's preferred width in logical pixels.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        if width.is_finite() && width >= 0.0 {
            self.width = width;
        }
        self
    }

    /// Sets the indicator's preferred height in logical pixels.
    #[inline]
    pub fn height(mut self, height: f32) -> Self {
        if height.is_finite() && height >= 0.0 {
            self.height = height;
        }
        self
    }

    /// Sets the inactive track color.
    #[inline]
    pub const fn track_color(mut self, color: Color) -> Self {
        self.track_color = color;
        self
    }

    /// Sets the determinate/active segment color.
    #[inline]
    pub const fn progress_color(mut self, color: Color) -> Self {
        self.progress_color = color;
        self
    }

    /// Returns the preferred logical width.
    #[inline]
    pub const fn width_value(self) -> f32 {
        self.width
    }

    /// Returns the preferred logical height.
    #[inline]
    pub const fn height_value(self) -> f32 {
        self.height
    }
}

impl Default for ProgressIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ProgressIndicator {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let active_width = match self.state {
            ProgressState::Determinate(fraction) => self.width * fraction,
            // A short active segment gives an indeterminate indicator a
            // stable, visible fallback even when no animation driver is
            // installed. Hosts may animate/reposition it using the same model.
            ProgressState::Indeterminate => self.width * 0.32,
        };
        Stack::new()
            .add_child(
                SizedBox::new()
                    .width(self.width)
                    .height(self.height)
                    .color(self.track_color),
            )
            .add_child(
                SizedBox::new()
                    .width(active_width.clamp(0.0, self.width))
                    .height(self.height)
                    .color(self.progress_color),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "ProgressIndicator"
    }
}

impl PortableWidget for ProgressIndicator {}

/// The error returned when configuring a spinner with an invalid period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinnerError {
    /// A spinner period must be greater than zero.
    ZeroPeriod,
}

/// A non-interactive indeterminate progress indicator with an explicit phase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spinner {
    phase: f32,
    period: Duration,
    motion_policy: MotionPolicy,
    size: f32,
    color: Color,
}

impl Spinner {
    /// The default time for one complete spinner revolution.
    pub const DEFAULT_PERIOD: Duration = Duration::from_millis(1_000);

    /// Creates a spinner with [`Self::DEFAULT_PERIOD`].
    #[inline]
    pub const fn new() -> Self {
        Self {
            phase: 0.0,
            period: Self::DEFAULT_PERIOD,
            motion_policy: MotionPolicy::Full,
            size: 24.0,
            color: Color::Rgba(35, 110, 220, 255),
        }
    }

    /// Creates a spinner with a positive period.
    pub fn with_period(period: Duration) -> Result<Self, SpinnerError> {
        if period.is_zero() {
            return Err(SpinnerError::ZeroPeriod);
        }
        Ok(Self {
            period,
            ..Self::new()
        })
    }

    /// Returns the configured period.
    #[inline]
    pub const fn period(self) -> Duration {
        self.period
    }

    /// Sets the reduced-motion preference.
    #[inline]
    pub fn set_motion_policy(&mut self, policy: MotionPolicy) {
        self.motion_policy = policy;
    }

    /// Returns the current motion preference.
    #[inline]
    pub const fn motion_policy(self) -> MotionPolicy {
        self.motion_policy
    }

    /// Returns the normalized phase in the half-open range `0.0..1.0`.
    #[inline]
    pub const fn phase(self) -> f32 {
        self.phase
    }

    /// Advances the spinner by a deterministic duration when motion is enabled.
    pub fn advance(&mut self, elapsed: Duration) {
        if self.motion_policy == MotionPolicy::Reduced || elapsed.is_zero() {
            return;
        }
        let turns = elapsed.as_secs_f64() / self.period.as_secs_f64();
        self.phase = ((self.phase as f64 + turns) % 1.0) as f32;
    }

    /// Sets the spinner's preferred square size in logical pixels.
    #[inline]
    pub fn size(mut self, size: f32) -> Self {
        if size.is_finite() && size >= 0.0 {
            self.size = size;
        }
        self
    }

    /// Sets the spinner color.
    #[inline]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Returns the preferred square size.
    #[inline]
    pub const fn size_value(self) -> f32 {
        self.size
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Spinner {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // Text glyphs keep this fallback renderer portable and cheap. A host
        // that has a vector indicator can use `phase()` to replace the visual
        // while retaining the same deterministic motion model.
        let glyph = ["◴", "◷", "◶", "◵"][(self.phase * 4.0).floor().clamp(0.0, 3.0) as usize];
        SizedBox::new()
            .width(self.size)
            .height(self.size)
            .child(Text::new(glyph).text_style(TextStyle::new().color(self.color)))
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "Spinner"
    }
}

impl PortableWidget for Spinner {}

fn validate_fraction(value: f32) -> Result<(), ProgressError> {
    if !value.is_finite() {
        Err(ProgressError::NonFinite)
    } else if !(0.0..=1.0).contains(&value) {
        Err(ProgressError::OutOfRange)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_rejects_zero_period() {
        assert_eq!(
            Spinner::with_period(Duration::ZERO),
            Err(SpinnerError::ZeroPeriod)
        );
    }
}
