//! macOS: the trackpad's Taptic Engine through `NSHapticFeedbackManager`.
//!
//! AppKit offers three canned feedbacks and no way to shape or schedule them,
//! so every [`ImpactStyle`] / [`NotificationStyle`] is mapped onto the nearest
//! of the three, and programmable patterns are not supported (see
//! [`play_pattern`]). Feedback is only felt on a Force Touch trackpad with a
//! finger resting on it; on any other Mac the call is accepted and nothing
//! happens, which is AppKit's own behaviour.

use crate::{HapticPattern, ImpactStyle, NotificationStyle};
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSHapticFeedbackManager, NSHapticFeedbackPattern, NSHapticFeedbackPerformanceTime,
    NSHapticFeedbackPerformer,
};

/// The weakest of the three: a plain tick.
const LIGHT: NSHapticFeedbackPattern = NSHapticFeedbackPattern::Generic;
/// A snap onto a guide or a grid line.
const MEDIUM: NSHapticFeedbackPattern = NSHapticFeedbackPattern::Alignment;
/// A step between discrete levels; the firmest of the three.
const STRONG: NSHapticFeedbackPattern = NSHapticFeedbackPattern::LevelChange;

fn impact_pattern(style: ImpactStyle) -> NSHapticFeedbackPattern {
    match style {
        ImpactStyle::Light | ImpactStyle::Soft => LIGHT,
        ImpactStyle::Medium | ImpactStyle::Rigid => MEDIUM,
        ImpactStyle::Heavy => STRONG,
    }
}

fn notification_pattern(style: NotificationStyle) -> NSHapticFeedbackPattern {
    match style {
        NotificationStyle::Success => MEDIUM,
        // There is nothing firmer than `STRONG` to tell a failure apart with.
        NotificationStyle::Warning | NotificationStyle::Error => STRONG,
    }
}

fn perform(pattern: NSHapticFeedbackPattern) {
    // AppKit objects are main-thread territory. Skip rather than risk it when
    // called from a background task, as the iOS backend does.
    if MainThreadMarker::new().is_none() {
        return;
    }
    // `Now` rather than `Default`: `Default` lets AppKit hold the feedback
    // until the next draw to line it up with what the user sees, but the
    // framework's callers fire it in response to an input event.
    NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(
        pattern,
        NSHapticFeedbackPerformanceTime::Now,
    );
}

/// The trackpad has no warm-up latency to hide.
pub(crate) fn prepare_impact(_style: ImpactStyle) {}

pub(crate) fn impact(style: ImpactStyle) {
    perform(impact_pattern(style));
}

pub(crate) fn selection() {
    perform(MEDIUM);
}

pub(crate) fn notification(style: NotificationStyle) {
    perform(notification_pattern(style));
}

/// Not supported: `NSHapticFeedbackManager` can neither vary intensity nor
/// schedule a feedback in the future, so a timed pattern cannot be played and
/// a lone tick would misrepresent it. Does nothing.
pub(crate) fn play_pattern(_pattern: &HapticPattern) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impact_styles_are_ordered_by_strength() {
        let strength = |style| match impact_pattern(style) {
            LIGHT => 0,
            MEDIUM => 1,
            STRONG => 2,
            other => panic!("unexpected pattern {other:?}"),
        };

        assert!(strength(ImpactStyle::Soft) <= strength(ImpactStyle::Light));
        assert!(strength(ImpactStyle::Light) < strength(ImpactStyle::Medium));
        assert!(strength(ImpactStyle::Medium) <= strength(ImpactStyle::Rigid));
        assert!(strength(ImpactStyle::Rigid) < strength(ImpactStyle::Heavy));
    }

    #[test]
    fn failures_are_felt_at_least_as_hard_as_successes() {
        let level = |style| notification_pattern(style).0;

        assert!(level(NotificationStyle::Error) >= level(NotificationStyle::Success));
        assert!(level(NotificationStyle::Warning) >= level(NotificationStyle::Success));
    }

    #[test]
    fn off_the_main_thread_feedback_is_skipped_not_crashed() {
        // Test threads are never the main thread, so this exercises the guard.
        impact(ImpactStyle::Heavy);
        selection();
        notification(NotificationStyle::Error);
    }
}
