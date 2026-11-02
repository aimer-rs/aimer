//! Platform-neutral haptic vocabulary: canned styles and programmable patterns.

/// Mirrors `UIImpactFeedbackGenerator.FeedbackStyle`.
///
/// Platforms with fewer distinct effects map several styles onto one; see the
/// crate documentation for the table.
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
    /// # use aimer_haptics::HapticPattern;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pattern_is_empty() {
        assert!(HapticPattern::new().events.is_empty());
    }

    #[test]
    fn transient_event_has_no_duration() {
        let pattern = HapticPattern::new().transient(0.0, 1.0, 0.5);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.0,
                duration: None,
                intensity: 1.0,
                sharpness: 0.5,
            }]
        );
    }

    #[test]
    fn continuous_event_carries_its_duration() {
        let pattern = HapticPattern::new().continuous(0.1, 0.4, 0.6, 0.2);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.1,
                duration: Some(0.4),
                intensity: 0.6,
                sharpness: 0.2,
            }]
        );
    }

    #[test]
    fn events_keep_insertion_order() {
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .continuous(0.1, 0.4, 0.6, 0.2)
            .transient(0.6, 0.3, 0.9);

        let times: Vec<f64> = pattern.events.iter().map(|event| event.time).collect();
        assert_eq!(times, vec![0.0, 0.1, 0.6]);
    }

    #[test]
    fn duration_spans_until_the_last_event_ends() {
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .continuous(0.1, 0.4, 0.6, 0.2);

        // The buzz starts at 0.1s and lasts 0.4s, so the pattern is 0.5s long.
        assert!((pattern.duration() - 0.5).abs() < 1e-9, "{pattern:?}");
    }

    #[test]
    fn duration_of_an_empty_pattern_is_zero() {
        assert_eq!(HapticPattern::new().duration(), 0.0);
    }

    #[test]
    fn duration_does_not_depend_on_insertion_order() {
        let late_first = HapticPattern::new()
            .transient(2.0, 1.0, 1.0)
            .continuous(0.0, 0.5, 1.0, 1.0);

        assert_eq!(late_first.duration(), 2.0);
    }

    #[test]
    fn out_of_range_parameters_are_clamped() {
        // Core Haptics rejects the *whole* pattern if a single parameter is
        // outside 0..=1, so the builder never lets one through.
        let pattern = HapticPattern::new().transient(-1.0, 4.0, -2.0);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.0,
                duration: None,
                intensity: 1.0,
                sharpness: 0.0,
            }]
        );
    }

    #[test]
    fn a_negative_duration_is_clamped_to_zero() {
        let pattern = HapticPattern::new().continuous(0.0, -1.0, 0.5, 0.5);

        assert_eq!(pattern.events[0].duration, Some(0.0));
    }
}
