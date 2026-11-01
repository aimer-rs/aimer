//! The cross-platform entry point and the Anteros capability provider.

use crate::backend;
use crate::{HapticFeedback, HapticKind, HapticPattern, ImpactStyle, NotificationStyle};

/// Entry point widgets call into, e.g. on tap/drag-release/toggle.
///
/// Every call is safe on every target: where the platform has no haptic
/// hardware or no backend, it does nothing. See the crate documentation for how
/// each platform realises the effects.
pub struct Haptics;

impl Haptics {
    /// Wakes the haptic hardware ahead of an [`impact`](Self::impact) of
    /// `style` that is expected within the next moment or two.
    ///
    /// The first impact after the device has been idle is late by the time the
    /// Taptic Engine takes to start. Call this when the gesture that will cause
    /// the impact begins (a press that may become a hold, say), so the engine
    /// is ready when the impact fires. A no-op on targets with no such delay.
    #[inline]
    pub fn prepare_impact(style: ImpactStyle) {
        backend::prepare_impact(style);
    }

    /// A physical "bump" — button presses, drag snapping, drops.
    #[inline]
    pub fn impact(style: ImpactStyle) {
        backend::impact(style);
    }

    /// A light tick — picker wheels, tab switches, snapping to a grid line.
    #[inline]
    pub fn selection() {
        backend::selection();
    }

    /// Task outcome feedback — form validation, async op finishing.
    #[inline]
    pub fn notification(style: NotificationStyle) {
        backend::notification(style);
    }

    /// Plays a custom [`HapticPattern`].
    ///
    /// Core Haptics on iOS and a `VibrationEffect` waveform on Android (where
    /// sharpness has no equivalent and only intensity is honoured). A no-op on
    /// hardware that cannot play it — iPads, Macs, devices without a vibrator —
    /// and wherever a platform cannot schedule feedback in time.
    #[inline]
    pub fn play_pattern(pattern: &HapticPattern) {
        backend::play_pattern(pattern);
    }
}

/// Permanent-host implementation of Aimer's portable haptics capability.
///
/// iOS, macOS and Android map the portable effects onto their native feedback.
/// Platforms that do not provide an Aimer haptics backend return an explicit
/// unsupported result instead of silently accepting an effect they cannot
/// produce.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeHapticsProvider;

impl HapticFeedback for NativeHapticsProvider {
    fn trigger(&self, kind: HapticKind) -> aimer_anteros::CapabilityResult<()> {
        if !backend::SUPPORTED {
            return Err(aimer_anteros::CapabilityError::Unsupported);
        }
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
}

#[cfg(test)]
mod tests {
    use aimer_anteros::CapabilityError;

    use super::*;

    const ALL_KINDS: [HapticKind; 7] = [
        HapticKind::Selection,
        HapticKind::LightImpact,
        HapticKind::MediumImpact,
        HapticKind::HeavyImpact,
        HapticKind::Success,
        HapticKind::Warning,
        HapticKind::Error,
    ];

    #[test]
    fn calls_never_panic() {
        // The point of the `cfg`-free API: widgets can call these on any
        // target without a panic or a `cfg` guard at the call site.
        Haptics::prepare_impact(ImpactStyle::Light);
        Haptics::impact(ImpactStyle::Light);
        Haptics::selection();
        Haptics::notification(NotificationStyle::Success);
        Haptics::play_pattern(&HapticPattern::new().transient(0.0, 1.0, 1.0));
        Haptics::play_pattern(&HapticPattern::new());
    }

    #[test]
    fn provider_answers_every_kind_consistently_with_the_backend() {
        for kind in ALL_KINDS {
            let result = NativeHapticsProvider.trigger(kind);
            if backend::SUPPORTED {
                assert_eq!(result, Ok(()), "{kind:?}");
            } else {
                assert_eq!(result, Err(CapabilityError::Unsupported), "{kind:?}");
            }
        }
    }

    #[test]
    #[cfg(not(any(target_os = "ios", target_os = "macos", target_os = "android")))]
    fn provider_reports_unsupported_without_a_platform_backend() {
        assert!(!backend::SUPPORTED);
    }

    #[test]
    #[cfg(any(target_os = "ios", target_os = "macos", target_os = "android"))]
    fn provider_is_supported_on_platforms_with_a_backend() {
        assert!(backend::SUPPORTED);
    }
}
