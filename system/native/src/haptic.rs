//! Haptic feedback.
//!
//! Wraps UIKit's `UIFeedbackGenerator` family (`UIImpactFeedbackGenerator`,
//! `UISelectionFeedbackGenerator`, `UINotificationFeedbackGenerator`) on iOS.
//! On every other target the calls compile to no-ops, so widgets can call
//! `Haptics::impact(..)` unconditionally without `cfg` sprinkled everywhere.
//!
//! Also exposes a *programmable* layer on top of Core Haptics
//! (`CHHapticEngine` / `CHHapticPattern`) for custom intensity/sharpness
//! curves, not just the five canned impact styles above.

#[cfg(target_os = "ios")]
mod ios_haptic;
#[cfg(all(target_arch = "wasm32", not(aimer_portable_guest)))]
mod web_haptic;

#[cfg(target_os = "ios")]
use ios_haptic as ios;

/// Mirrors `UIImpactFeedbackGenerator.FeedbackStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactStyle {
    Light,
    Medium,
    Heavy,
    Soft,
    Rigid,
}

/// Mirrors `UINotificationFeedbackGenerator.FeedbackType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationStyle {
    Success,
    Warning,
    Error,
}

/// A single event inside a [`HapticPattern`].
///
/// `intensity` and `sharpness` are both `0.0..=1.0`, matching Core Haptics'
/// `HapticIntensity` / `HapticSharpness` parameters — intensity is loudness,
/// sharpness is how "crisp vs. round" the tap feels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HapticEvent {
    /// Offset from pattern start, in seconds.
    pub time: f64,
    /// `None` = an instantaneous tap (`HapticTransient`).
    /// `Some(seconds)` = a sustained buzz for that duration (`HapticContinuous`).
    pub duration: Option<f64>,
    pub intensity: f32,
    pub sharpness: f32,
}

/// A custom, fully programmable haptic sequence — the Core Haptics
/// equivalent of an AHAP file, built up in code instead of JSON.
///
/// ```ignore
/// let pattern = HapticPattern::new()
///     .transient(0.0, 1.0, 1.0)          // sharp tap at t=0
///     .continuous(0.1, 0.4, 0.6, 0.2);   // soft buzz starting at t=0.1s for 0.4s
/// Haptics::play_pattern(&pattern);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HapticPattern {
    pub events: Vec<HapticEvent>,
}

impl HapticPattern {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an instantaneous tap.
    ///
    /// `time` is the offset from the pattern's start in seconds; `intensity`
    /// and `sharpness` are clamped into `0.0..=1.0`, since Core Haptics
    /// rejects an entire pattern that contains one out-of-range parameter.
    #[inline]
    pub fn transient(mut self, time: f64, intensity: f32, sharpness: f32) -> Self {
        self.events.push(HapticEvent {
            time: time.max(0.0),
            duration: None,
            intensity: intensity.clamp(0.0, 1.0),
            sharpness: sharpness.clamp(0.0, 1.0),
        });
        self
    }

    /// Add a sustained buzz of `duration` seconds.
    ///
    /// Parameters are clamped exactly as in [`HapticPattern::transient`].
    #[inline]
    pub fn continuous(mut self, time: f64, duration: f64, intensity: f32, sharpness: f32) -> Self {
        self.events.push(HapticEvent {
            time: time.max(0.0),
            duration: Some(duration.max(0.0)),
            intensity: intensity.clamp(0.0, 1.0),
            sharpness: sharpness.clamp(0.0, 1.0),
        });
        self
    }

    /// How long the whole pattern takes to play, in seconds.
    ///
    /// This is the end of the event that finishes last — not the sum of the
    /// events, which may overlap, and not the order they were added in. Zero
    /// for a pattern without events.
    ///
    /// # Examples
    ///
    /// ```
    /// # use aimer_native::haptic::HapticPattern;
    /// let pattern = HapticPattern::new()
    ///     .transient(0.0, 1.0, 1.0)
    ///     .continuous(0.1, 0.4, 0.6, 0.2);
    ///
    /// assert_eq!(pattern.duration(), 0.5);
    /// ```
    pub fn duration(&self) -> f64 {
        self.events
            .iter()
            .map(|event| event.time + event.duration.unwrap_or(0.0))
            .fold(0.0, f64::max)
    }
}

/// Entry point widgets call into, e.g. on tap/drag-release/toggle.
pub struct Haptics;

impl Haptics {
    /// A physical "bump" — button presses, drag snapping, drops.
    pub fn impact(style: ImpactStyle) {
        #[cfg(target_os = "ios")]
        ios::impact(style);
        #[cfg(not(target_os = "ios"))]
        let _ = style;
    }

    /// A light tick — picker wheels, tab switches, snapping to a grid line.
    pub fn selection() {
        #[cfg(target_os = "ios")]
        ios::selection();
    }

    /// Task outcome feedback — form validation, async op finishing.
    pub fn notification(style: NotificationStyle) {
        #[cfg(target_os = "ios")]
        ios::notification(style);
        #[cfg(not(target_os = "ios"))]
        let _ = style;
    }

    /// Play a custom [`HapticPattern`] via Core Haptics. Fails silently
    /// (no-op) on hardware without a Taptic Engine, e.g. iPads.
    pub fn play_pattern(pattern: &HapticPattern) {
        #[cfg(target_os = "ios")]
        ios::play_pattern(pattern);
        #[cfg(not(target_os = "ios"))]
        let _ = pattern;
    }
}

/// Permanent-host implementation of Aimer's portable haptics capability.
///
/// iOS maps the portable effects to UIKit feedback generators. Platforms that
/// do not yet provide an Aimer haptics backend return an explicit unsupported
/// result instead of silently accepting an effect they cannot produce.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeHapticsProvider;

impl aimer_haptics::HapticFeedback for NativeHapticsProvider {
    fn trigger(&self, kind: aimer_haptics::HapticKind) -> aimer_anteros::CapabilityResult<()> {
        #[cfg(target_os = "ios")]
        {
            use aimer_haptics::HapticKind;

            match kind {
                HapticKind::Selection => Haptics::selection(),
                HapticKind::LightImpact => Haptics::impact(ImpactStyle::Light),
                HapticKind::MediumImpact => Haptics::impact(ImpactStyle::Medium),
                HapticKind::HeavyImpact => Haptics::impact(ImpactStyle::Heavy),
                HapticKind::Success => Haptics::notification(NotificationStyle::Success),
                HapticKind::Warning => Haptics::notification(NotificationStyle::Warning),
                HapticKind::Error => Haptics::notification(NotificationStyle::Error),
            }
            Ok(())
        }
        #[cfg(not(target_os = "ios"))]
        {
            let _ = kind;
            Err(aimer_anteros::CapabilityError::Unsupported)
        }
    }
}

#[cfg(all(test, not(target_os = "ios")))]
mod capability_tests {
    use aimer_anteros::CapabilityError;
    use aimer_haptics::{HapticFeedback, HapticKind};

    use super::NativeHapticsProvider;

    #[test]
    fn native_provider_reports_unsupported_without_a_platform_backend() {
        let provider = NativeHapticsProvider;

        for kind in [
            HapticKind::Selection,
            HapticKind::LightImpact,
            HapticKind::MediumImpact,
            HapticKind::HeavyImpact,
            HapticKind::Success,
            HapticKind::Warning,
            HapticKind::Error,
        ] {
            assert_eq!(provider.trigger(kind), Err(CapabilityError::Unsupported));
        }
    }
}
