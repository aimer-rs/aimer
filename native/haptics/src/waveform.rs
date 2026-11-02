//! The Android haptic model, kept free of JNI so it can be tested on the host.
//!
//! Android's `Vibrator` plays a *timeline of amplitudes*: a list of
//! `(duration, amplitude)` segments, amplitude `0` being silence. Everything the
//! public API offers — canned impacts, notifications and programmable
//! [`HapticPattern`]s — is reduced to that shape here, and the JNI layer only
//! has to ship the result across.

use std::borrow::Cow;

use crate::{HapticPattern, ImpactStyle, NotificationStyle};

/// One stretch of a vibration: `amplitude` held for `millis`.
///
/// `amplitude` is `0` (off) or `1..=255`, the range `VibrationEffect` accepts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub millis: i64,
    pub amplitude: i32,
}

const fn on(millis: i64, amplitude: i32) -> Segment {
    Segment { millis, amplitude }
}

const fn off(millis: i64) -> Segment {
    Segment {
        millis,
        amplitude: 0,
    }
}

/// What to ask the vibrator for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    /// A `VibrationEffect.EFFECT_*` tuned by the device maker (API 29+),
    /// with the segments to play on older releases.
    Predefined {
        id: i32,
        fallback: Cow<'static, [Segment]>,
    },
    /// An explicit timeline.
    Timeline(Cow<'static, [Segment]>),
}

/// `VibrationEffect.EFFECT_CLICK`.
const EFFECT_CLICK: i32 = 0;
/// `VibrationEffect.EFFECT_TICK`.
const EFFECT_TICK: i32 = 2;
/// `VibrationEffect.EFFECT_HEAVY_CLICK`.
const EFFECT_HEAVY_CLICK: i32 = 5;

const LIGHT_TICK: &[Segment] = &[on(12, 80)];
const SELECTION_TICK: &[Segment] = &[on(8, 60)];
const MEDIUM_CLICK: &[Segment] = &[on(20, 150)];
const HEAVY_CLICK: &[Segment] = &[on(30, 255)];
// Two quick pulses, the second firmer: "it went through".
const SUCCESS: &[Segment] = &[on(20, 160), off(50), on(25, 255)];
// Two even, medium pulses: "are you sure?".
const WARNING: &[Segment] = &[on(40, 200), off(60), on(40, 200)];
// Three hard pulses, the last one longer: "no".
const ERROR: &[Segment] = &[on(40, 255), off(40), on(40, 255), off(40), on(60, 255)];

/// How long a transient event of a [`HapticPattern`] buzzes, in milliseconds.
///
/// A vibration motor cannot produce an instant: it has to spin up. This is
/// about the shortest pulse that is still perceptible.
const TRANSIENT_MILLIS: i64 = 10;

impl Effect {
    fn predefined(id: i32, fallback: &'static [Segment]) -> Self {
        Self::Predefined {
            id,
            fallback: Cow::Borrowed(fallback),
        }
    }

    /// The effect for [`Haptics::selection`](crate::Haptics::selection).
    pub(crate) fn selection() -> Self {
        Self::predefined(EFFECT_TICK, SELECTION_TICK)
    }

    /// The effect for [`Haptics::impact`](crate::Haptics::impact).
    ///
    /// Android has three predefined strengths where UIKit has five styles, so
    /// `Soft` shares the tick with `Light` and `Rigid` the click with `Medium`.
    pub(crate) fn impact(style: ImpactStyle) -> Self {
        match style {
            ImpactStyle::Light | ImpactStyle::Soft => Self::predefined(EFFECT_TICK, LIGHT_TICK),
            ImpactStyle::Medium | ImpactStyle::Rigid => {
                Self::predefined(EFFECT_CLICK, MEDIUM_CLICK)
            }
            ImpactStyle::Heavy => Self::predefined(EFFECT_HEAVY_CLICK, HEAVY_CLICK),
        }
    }

    /// The effect for [`Haptics::notification`](crate::Haptics::notification).
    ///
    /// There is no predefined effect for an outcome, so each is a short
    /// waveform with a rhythm of its own.
    pub(crate) fn notification(style: NotificationStyle) -> Self {
        Self::Timeline(Cow::Borrowed(match style {
            NotificationStyle::Success => SUCCESS,
            NotificationStyle::Warning => WARNING,
            NotificationStyle::Error => ERROR,
        }))
    }

    /// The effect for [`Haptics::play_pattern`](crate::Haptics::play_pattern),
    /// or `None` when the pattern would make no vibration at all.
    ///
    /// Android has no notion of sharpness, so only `intensity` survives: it
    /// becomes the amplitude.
    pub(crate) fn pattern(pattern: &HapticPattern) -> Option<Self> {
        let segments = pattern_segments(pattern);
        (!segments.is_empty()).then(|| Self::Timeline(Cow::Owned(segments)))
    }
}

/// Maps an intensity in `0.0..=1.0` onto `VibrationEffect`'s amplitude scale.
///
/// Zero stays silent; anything audible-in-principle is at least `1`, because
/// `0` means "off" to the vibrator.
fn amplitude(intensity: f32) -> i32 {
    let intensity = intensity.clamp(0.0, 1.0);
    if intensity.is_nan() || intensity <= 0.0 {
        0
    } else {
        ((intensity * 255.0).round() as i32).clamp(1, 255)
    }
}

fn millis(seconds: f64) -> i64 {
    // `as` saturates, and maps NaN to 0.
    (seconds * 1000.0).round() as i64
}

/// Flattens a pattern's possibly overlapping events into one timeline.
///
/// Where events overlap the loudest wins, which is also what a listener would
/// feel. A leading gap is kept (the pattern may start late); trailing silence
/// is dropped. Empty when nothing in the pattern would vibrate.
fn pattern_segments(pattern: &HapticPattern) -> Vec<Segment> {
    struct Span {
        start: i64,
        end: i64,
        amplitude: i32,
    }

    let spans: Vec<Span> = pattern
        .events
        .iter()
        .map(|event| {
            let start = millis(event.time.max(0.0));
            let length = match event.duration {
                None => TRANSIENT_MILLIS,
                // A zero-length buzz would be an empty segment, which the
                // vibrator rejects; give it the least length that exists.
                Some(duration) => millis(duration).max(1),
            };
            Span {
                start,
                end: start.saturating_add(length),
                amplitude: amplitude(event.intensity),
            }
        })
        .collect();

    // Every edge of every span: between two neighbouring edges the set of
    // active spans does not change, so one amplitude describes the whole gap.
    let mut edges: Vec<i64> = spans
        .iter()
        .flat_map(|span| [span.start, span.end])
        .collect();
    edges.sort_unstable();
    edges.dedup();

    let mut timeline: Vec<Segment> = Vec::new();
    // The first edge is the pattern's start of vibration, not of time: the
    // gap before it is silence only if the pattern starts after zero.
    if let Some(&first) = edges.first() {
        if first > 0 {
            timeline.push(off(first));
        }
    }
    for window in edges.windows(2) {
        let (from, to) = (window[0], window[1]);
        let loudest = spans
            .iter()
            .filter(|span| span.start <= from && to <= span.end)
            .map(|span| span.amplitude)
            .max()
            .unwrap_or(0);
        match timeline.last_mut() {
            Some(last) if last.amplitude == loudest => last.millis += to - from,
            _ => timeline.push(Segment {
                millis: to - from,
                amplitude: loudest,
            }),
        }
    }

    while timeline.last().is_some_and(|last| last.amplitude == 0) {
        timeline.pop();
    }
    // Only silence: nothing to play, however long it lasts.
    if timeline.iter().all(|segment| segment.amplitude == 0) {
        timeline.clear();
    }
    timeline
}

/// The on/off timing array for `Vibrator.vibrate(long[], int)`, used before
/// API 26 where amplitude does not exist.
///
/// That array alternates *wait, vibrate, wait, vibrate …* starting with a wait,
/// so a timeline that opens with a vibration gets a zero wait in front, and
/// neighbouring segments in the same state are merged (a quiet and a loud
/// stretch are both just "on" there).
pub(crate) fn legacy_timings(segments: &[Segment]) -> Vec<i64> {
    let mut timings: Vec<i64> = Vec::with_capacity(segments.len() + 1);
    for segment in segments {
        let vibrating = segment.amplitude > 0;
        // Entries alternate wait, vibrate, ..., so the open (last) entry is a
        // vibration exactly when the array has an even, non-zero length.
        let open_is_vibration = !timings.is_empty() && timings.len() % 2 == 0;
        if timings.is_empty() {
            if vibrating {
                timings.push(0);
            }
            timings.push(segment.millis);
        } else if vibrating == open_is_vibration {
            if let Some(open) = timings.last_mut() {
                *open += segment.millis;
            }
        } else {
            timings.push(segment.millis);
        }
    }
    timings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern_of(segments: &[Segment]) -> Vec<(i64, i32)> {
        segments.iter().map(|s| (s.millis, s.amplitude)).collect()
    }

    fn segments(pattern: &HapticPattern) -> Vec<(i64, i32)> {
        pattern_of(&pattern_segments(pattern))
    }

    #[test]
    fn intensity_maps_onto_the_amplitude_scale() {
        assert_eq!(amplitude(0.0), 0);
        assert_eq!(amplitude(1.0), 255);
        assert_eq!(amplitude(0.5), 128);
        // Faint but not silent: 0 would mean "off".
        assert_eq!(amplitude(0.001), 1);
    }

    #[test]
    fn out_of_range_intensity_is_clamped() {
        assert_eq!(amplitude(7.0), 255);
        assert_eq!(amplitude(-3.0), 0);
        assert_eq!(amplitude(f32::NAN), 0);
    }

    #[test]
    fn an_empty_pattern_makes_no_effect() {
        assert_eq!(Effect::pattern(&HapticPattern::new()), None);
    }

    #[test]
    fn a_transient_is_a_short_pulse() {
        let pattern = HapticPattern::new().transient(0.0, 1.0, 1.0);

        assert_eq!(segments(&pattern), vec![(TRANSIENT_MILLIS, 255)]);
    }

    #[test]
    fn a_continuous_event_keeps_its_length() {
        let pattern = HapticPattern::new().continuous(0.0, 0.4, 0.5, 0.0);

        assert_eq!(segments(&pattern), vec![(400, 128)]);
    }

    #[test]
    fn a_gap_between_events_is_silence() {
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .transient(0.1, 1.0, 1.0);

        assert_eq!(
            segments(&pattern),
            vec![(10, 255), (90, 0), (10, 255)],
        );
    }

    #[test]
    fn a_late_start_keeps_the_leading_delay() {
        let pattern = HapticPattern::new().transient(0.2, 1.0, 1.0);

        assert_eq!(segments(&pattern), vec![(200, 0), (10, 255)]);
    }

    #[test]
    fn overlapping_events_play_at_the_louder_amplitude() {
        // A soft buzz 0..400 ms with a hard tap at 100 ms riding on it.
        let pattern = HapticPattern::new()
            .continuous(0.0, 0.4, 0.2, 0.0)
            .transient(0.1, 1.0, 1.0);

        assert_eq!(
            segments(&pattern),
            vec![(100, 51), (10, 255), (290, 51)],
        );
    }

    #[test]
    fn events_added_out_of_order_give_the_same_timeline() {
        let forward = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .transient(0.1, 0.5, 1.0);
        let reversed = HapticPattern::new()
            .transient(0.1, 0.5, 1.0)
            .transient(0.0, 1.0, 1.0);

        assert_eq!(segments(&forward), segments(&reversed));
    }

    #[test]
    fn adjacent_equal_amplitudes_merge_into_one_segment() {
        let pattern = HapticPattern::new()
            .continuous(0.0, 0.1, 1.0, 0.0)
            .continuous(0.1, 0.1, 1.0, 0.0);

        assert_eq!(segments(&pattern), vec![(200, 255)]);
    }

    #[test]
    fn a_zero_intensity_pattern_makes_no_effect() {
        let pattern = HapticPattern::new().continuous(0.0, 0.5, 0.0, 0.0);

        assert_eq!(Effect::pattern(&pattern), None);
    }

    #[test]
    fn a_zero_length_buzz_still_has_a_length() {
        let pattern = HapticPattern::new().continuous(0.0, 0.0, 1.0, 0.0);

        assert_eq!(segments(&pattern), vec![(1, 255)]);
    }

    #[test]
    fn trailing_silence_is_dropped() {
        // A silent event after the last audible one must not stretch the effect.
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .transient(0.5, 0.0, 1.0);

        assert_eq!(segments(&pattern), vec![(10, 255)]);
    }

    #[test]
    fn every_segment_is_valid_for_the_vibrator() {
        // `VibrationEffect.createWaveform` rejects empty segments and
        // amplitudes outside 0..=255.
        let canned = [
            Effect::selection(),
            Effect::impact(ImpactStyle::Light),
            Effect::impact(ImpactStyle::Soft),
            Effect::impact(ImpactStyle::Medium),
            Effect::impact(ImpactStyle::Rigid),
            Effect::impact(ImpactStyle::Heavy),
            Effect::notification(NotificationStyle::Success),
            Effect::notification(NotificationStyle::Warning),
            Effect::notification(NotificationStyle::Error),
        ];
        for effect in canned {
            let (Effect::Predefined { fallback: s, .. } | Effect::Timeline(s)) = &effect;
            assert!(!s.is_empty(), "{effect:?}");
            assert!(s.iter().any(|seg| seg.amplitude > 0), "{effect:?}");
            for seg in s.iter() {
                assert!(seg.millis > 0, "{effect:?}");
                assert!((0..=255).contains(&seg.amplitude), "{effect:?}");
            }
        }
    }

    #[test]
    fn impact_styles_map_onto_the_three_predefined_strengths() {
        let id = |style| match Effect::impact(style) {
            Effect::Predefined { id, .. } => id,
            other => panic!("not predefined: {other:?}"),
        };

        assert_eq!(id(ImpactStyle::Light), EFFECT_TICK);
        assert_eq!(id(ImpactStyle::Soft), EFFECT_TICK);
        assert_eq!(id(ImpactStyle::Medium), EFFECT_CLICK);
        assert_eq!(id(ImpactStyle::Rigid), EFFECT_CLICK);
        assert_eq!(id(ImpactStyle::Heavy), EFFECT_HEAVY_CLICK);
    }

    #[test]
    fn legacy_timings_start_with_a_wait() {
        // vibrate-first timelines get a zero wait so the parity lines up.
        assert_eq!(legacy_timings(&[on(20, 160)]), vec![0, 20]);
    }

    #[test]
    fn legacy_timings_alternate_wait_and_vibrate() {
        assert_eq!(
            legacy_timings(&[on(20, 160), off(50), on(25, 255)]),
            vec![0, 20, 50, 25],
        );
    }

    #[test]
    fn legacy_timings_keep_a_leading_delay() {
        assert_eq!(legacy_timings(&[off(200), on(10, 255)]), vec![200, 10]);
    }

    #[test]
    fn legacy_timings_merge_segments_of_the_same_state() {
        // Quiet and loud are both just "on" without amplitude control.
        assert_eq!(
            legacy_timings(&[on(100, 51), on(10, 255), on(290, 51)]),
            vec![0, 400],
        );
    }

    #[test]
    fn legacy_timings_of_nothing_is_empty() {
        assert!(legacy_timings(&[]).is_empty());
    }
}
