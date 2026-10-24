use std::time::Duration;

/// The smallest accepted text scale factor.
pub const MIN_TEXT_SCALE: f32 = 0.5;

/// The largest accepted text scale factor.
pub const MAX_TEXT_SCALE: f32 = 3.0;

/// Platform-independent accessibility preferences consumed by widgets and
/// adapters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccessibilityPreferences {
    reduced_motion: bool,
    text_scale: f32,
    high_contrast: bool,
    non_color_cues: bool,
}

impl AccessibilityPreferences {
    /// Creates preferences after validating the text scale factor.
    pub fn new(
        reduced_motion: bool,
        text_scale: f32,
        high_contrast: bool,
        non_color_cues: bool,
    ) -> Result<Self, PreferenceError> {
        validate_text_scale(text_scale)?;
        Ok(Self {
            reduced_motion,
            text_scale,
            high_contrast,
            non_color_cues,
        })
    }

    /// Reads platform settings through a narrow adapter supplied by the host.
    pub fn from_adapter<A: PreferenceAdapter>(adapter: &A) -> Result<Self, PreferenceError> {
        Self::new(
            adapter.reduced_motion(),
            adapter.text_scale(),
            adapter.high_contrast(),
            adapter.non_color_cues(),
        )
    }

    /// Returns whether motion should be reduced.
    pub const fn reduced_motion(self) -> bool {
        self.reduced_motion
    }

    /// Returns the validated text scale factor.
    pub const fn text_scale(self) -> f32 {
        self.text_scale
    }

    /// Returns whether high-contrast presentation is requested.
    pub const fn high_contrast(self) -> bool {
        self.high_contrast
    }

    /// Returns whether state must be communicated with more than color alone.
    pub const fn non_color_cues(self) -> bool {
        self.non_color_cues
    }

    /// Returns a copy with a new validated text scale factor.
    pub fn with_text_scale(mut self, text_scale: f32) -> Result<Self, PreferenceError> {
        validate_text_scale(text_scale)?;
        self.text_scale = text_scale;
        Ok(self)
    }

    /// Scales a finite, non-negative text size.
    pub fn scaled_text_size(self, base_size: f32) -> Result<f32, PreferenceError> {
        if !base_size.is_finite() || base_size < 0.0 {
            return Err(PreferenceError::InvalidTextSize(base_size));
        }
        Ok(base_size * self.text_scale)
    }

    /// Applies the reduced-motion policy to a duration.
    pub const fn motion_duration(self, normal: Duration) -> Duration {
        if self.reduced_motion {
            Duration::ZERO
        } else {
            normal
        }
    }
}

impl Default for AccessibilityPreferences {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            text_scale: 1.0,
            high_contrast: false,
            non_color_cues: false,
        }
    }
}

/// The platform-facing input seam for accessibility preferences.
pub trait PreferenceAdapter {
    /// Returns the platform's reduced-motion preference.
    fn reduced_motion(&self) -> bool;

    /// Returns the platform's requested text scale factor.
    fn text_scale(&self) -> f32;

    /// Returns the platform's high-contrast preference.
    fn high_contrast(&self) -> bool;

    /// Returns whether non-color cues are required.
    fn non_color_cues(&self) -> bool;
}

/// A rejected preference value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreferenceError {
    /// The text scale was not finite or fell outside
    /// [`MIN_TEXT_SCALE`], [`MAX_TEXT_SCALE`].
    InvalidTextScale(f32),
    /// A requested base text size was negative or not finite.
    InvalidTextSize(f32),
}

fn validate_text_scale(text_scale: f32) -> Result<(), PreferenceError> {
    if !text_scale.is_finite() || !(MIN_TEXT_SCALE..=MAX_TEXT_SCALE).contains(&text_scale) {
        Err(PreferenceError::InvalidTextScale(text_scale))
    } else {
        Ok(())
    }
}
