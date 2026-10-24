use std::time::Duration;

use crate::control::controller::AnimationController;
use crate::control::playback::controller_playback;
use crate::control::timed_keyframe::KeyframeTrack;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::AnyCurve;
use crate::primitives::time::AnimInstant;

/// One clock that several [`KeyframeTrack`]s are sampled from.
///
/// Each property of an effect (opacity, scale, a color) is its own track with
/// its own keyframes and value type. The timeline owns only the clock, so the
/// tracks stay in step however they are repeated, reversed or interrupted, and
/// tracks of different lengths simply hold their last value once they finish.
///
/// The timeline lasts as long as the longest track passed to
/// [`including`](Self::including), or the explicit duration it was created
/// with. Clones share one playback.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimInstant, KeyframeTimeline, KeyframeTrack};
///
/// let ms = Duration::from_millis;
/// let opacity = KeyframeTrack::builder()
///     .at(ms(0), 0.0f32)
///     .at(ms(200), 1.0)
///     .build()
///     .unwrap();
/// let scale = KeyframeTrack::builder()
///     .at(ms(0), 0.8f32)
///     .at(ms(400), 1.0)
///     .build()
///     .unwrap();
///
/// let timeline = KeyframeTimeline::new(Duration::ZERO)
///     .including(&opacity)
///     .including(&scale);
/// assert_eq!(timeline.duration(), ms(400));
///
/// timeline.forward_from_first_tick();
/// let start = AnimInstant::now();
/// timeline.tick(start);
/// let frame = timeline.tick(start + ms(200));
/// assert_eq!(frame.sample(&opacity), 1.0);
/// assert!((frame.sample(&scale) - 0.9).abs() < 1e-4);
/// ```
#[derive(Debug, Clone)]
pub struct KeyframeTimeline {
    controller: AnimationController,
}

impl KeyframeTimeline {
    /// Creates a stopped timeline that lasts at least `duration`.
    pub fn new(duration: Duration) -> Self {
        Self {
            controller: AnimationController::new(duration, AnyCurve::default()),
        }
    }

    /// Extends the timeline to cover `track`, if it is longer than the
    /// timeline already is.
    pub fn including<T: Animatable + Clone>(self, track: &KeyframeTrack<T>) -> Self {
        let duration = self.controller.duration().max(track.duration());
        self.controller.set_duration(duration);
        self
    }

    /// Returns how long the timeline lasts.
    pub fn duration(&self) -> Duration {
        self.controller.duration()
    }

    /// Returns the current position on the timeline without advancing time.
    pub fn frame(&self) -> TimelineFrame {
        self.frame_at_progress(self.controller.value())
    }

    /// Advances to `now` and returns the resulting position.
    ///
    /// Call it once per frame while [`is_animating`](Self::is_animating).
    pub fn tick(&self, now: AnimInstant) -> TimelineFrame {
        self.frame_at_progress(self.controller.tick(now))
    }

    /// Returns the position `progress` of the way through the timeline, where
    /// `0.0` is the start and `1.0` the end. Out-of-range values clamp.
    pub fn frame_at_progress(&self, progress: f32) -> TimelineFrame {
        let elapsed = if progress.is_finite() {
            self.duration().mul_f32(progress.clamp(0.0, 1.0))
        } else {
            Duration::ZERO
        };
        TimelineFrame { elapsed }
    }

    controller_playback!();
}

/// A position on a [`KeyframeTimeline`], from which tracks are sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineFrame {
    elapsed: Duration,
}

impl TimelineFrame {
    /// Returns how far into the timeline this position is.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Returns the value of `track` at this position.
    pub fn sample<T: Animatable + Clone>(&self, track: &KeyframeTrack<T>) -> T {
        track.at(self.elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::controller::AnimationStatus;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn track(end: u64, from: f32, to: f32) -> KeyframeTrack<f32> {
        KeyframeTrack::builder()
            .at(ms(0), from)
            .at(ms(end), to)
            .build()
            .unwrap()
    }

    #[test]
    fn lasts_as_long_as_the_longest_included_track() {
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&track(200, 0.0, 1.0))
            .including(&track(400, 0.0, 1.0))
            .including(&track(100, 0.0, 1.0));

        assert_eq!(timeline.duration(), ms(400));
    }

    #[test]
    fn an_explicit_duration_is_a_lower_bound() {
        let timeline = KeyframeTimeline::new(ms(500)).including(&track(200, 0.0, 1.0));

        assert_eq!(timeline.duration(), ms(500));
    }

    #[test]
    fn samples_every_track_from_one_clock() {
        let fade = track(200, 0.0, 1.0);
        let grow = track(400, 10.0, 20.0);
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&fade)
            .including(&grow);
        timeline.forward_from_first_tick();
        let start = AnimInstant::now();
        timeline.tick(start);

        let frame = timeline.tick(start + ms(100));

        assert_eq!(frame.elapsed(), ms(100));
        assert!((frame.sample(&fade) - 0.5).abs() < 1e-3);
        assert!((frame.sample(&grow) - 12.5).abs() < 1e-3);
    }

    #[test]
    fn a_finished_track_holds_its_last_value_while_others_play() {
        let fade = track(100, 0.0, 1.0);
        let grow = track(400, 0.0, 4.0);
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&fade)
            .including(&grow);
        timeline.forward_from_first_tick();
        let start = AnimInstant::now();
        timeline.tick(start);

        let frame = timeline.tick(start + ms(300));

        assert_eq!(frame.sample(&fade), 1.0);
        assert!((frame.sample(&grow) - 3.0).abs() < 1e-3);
    }

    #[test]
    fn tracks_may_have_different_value_types() {
        let opacity = track(100, 0.0, 1.0);
        let offset = KeyframeTrack::builder()
            .at(ms(0), (0.0f32, 0.0f32))
            .at(ms(100), (10.0, 20.0))
            .build()
            .unwrap();
        let timeline = KeyframeTimeline::new(Duration::ZERO)
            .including(&opacity)
            .including(&offset);

        let frame = timeline.frame_at_progress(0.5);

        assert!((frame.sample(&opacity) - 0.5).abs() < 1e-3);
        assert_eq!(frame.sample(&offset), (5.0, 10.0));
    }

    #[test]
    fn plays_to_completion_and_reports_the_end_position() {
        let timeline = KeyframeTimeline::new(ms(100));
        timeline.forward_from_first_tick();
        let start = AnimInstant::now();
        timeline.tick(start);

        let frame = timeline.tick(start + ms(100));

        assert_eq!(frame.elapsed(), ms(100));
        assert_eq!(timeline.status(), AnimationStatus::Completed);
    }

    #[test]
    fn reverse_plays_the_timeline_backwards() {
        let timeline = KeyframeTimeline::new(ms(100));
        timeline.controller().set_value(1.0);

        timeline.reverse();
        let frame = timeline.tick(AnimInstant::now() + Duration::from_secs(1));

        assert_eq!(frame.elapsed(), Duration::ZERO);
        assert_eq!(timeline.status(), AnimationStatus::Dismissed);
    }

    #[test]
    fn frame_reports_the_current_position_without_advancing() {
        let timeline = KeyframeTimeline::new(ms(100));
        timeline.controller().set_value(0.25);

        assert_eq!(timeline.frame().elapsed(), ms(25));
        assert_eq!(timeline.frame().elapsed(), ms(25));
    }

    #[test]
    fn out_of_range_and_non_finite_progress_is_safe() {
        let timeline = KeyframeTimeline::new(ms(100));

        assert_eq!(timeline.frame_at_progress(-1.0).elapsed(), Duration::ZERO);
        assert_eq!(timeline.frame_at_progress(2.0).elapsed(), ms(100));
        assert_eq!(timeline.frame_at_progress(f32::NAN).elapsed(), Duration::ZERO);
        assert_eq!(timeline.frame_at_progress(f32::INFINITY).elapsed(), Duration::ZERO);
    }

    #[test]
    fn clones_share_one_clock() {
        let timeline = KeyframeTimeline::new(ms(100));
        let handle = timeline.clone();

        handle.forward();

        assert!(timeline.is_animating());
    }
}
