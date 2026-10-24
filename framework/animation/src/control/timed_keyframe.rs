use std::time::Duration;

use crate::control::controller::AnimationController;
use crate::control::keyframe::{Keyframe, KeyframeAnimation, KeyframeError};
use crate::control::playback::controller_playback;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::{AnyCurve, Curve};
use crate::primitives::time::AnimInstant;

/// Keyframes placed at points in time rather than at fractions.
///
/// A track is a pure description of one value over time: it owns no clock, so
/// several tracks (an opacity, a scale, a color) can be sampled from one
/// [`KeyframeTimeline`](crate::KeyframeTimeline), or one track can be played
/// on its own as a [`TimedKeyframeAnimation`]. Its duration is the offset of
/// its last keyframe; before the first keyframe it holds the first value and
/// after the last it holds the last.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{curves, KeyframeTrack};
///
/// let bounce = KeyframeTrack::builder()
///     .at(Duration::ZERO, 0.0f32)
///     .at_with(Duration::from_millis(150), 1.0, curves::EASE_OUT)
///     .at(Duration::from_millis(400), 0.8)
///     .build()
///     .unwrap();
///
/// assert_eq!(bounce.duration(), Duration::from_millis(400));
/// assert_eq!(bounce.at(Duration::from_millis(150)), 1.0);
/// ```
#[derive(Debug, Clone)]
pub struct KeyframeTrack<T: Animatable + Clone> {
    /// Keyframes normalized against `duration`.
    animation: KeyframeAnimation<T>,
    duration: Duration,
}

impl<T: Animatable + Clone> KeyframeTrack<T> {
    /// Starts building a track.
    pub fn builder() -> TimedKeyframeBuilder<T> {
        TimedKeyframeBuilder::new()
    }

    /// Returns the time of the last keyframe.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the number of keyframes.
    pub fn len(&self) -> usize {
        self.animation.len()
    }

    /// Returns `true` if there are no keyframes. A built track never is.
    pub fn is_empty(&self) -> bool {
        self.animation.is_empty()
    }

    /// Returns the value `elapsed` into the track.
    ///
    /// A track whose keyframes are all at time zero has no duration to
    /// interpolate across, so every `elapsed` returns its final value.
    pub fn at(&self, elapsed: Duration) -> T {
        if self.duration.is_zero() {
            return self.animation.at(1.0);
        }
        self.animation
            .at(elapsed.as_secs_f32() / self.duration.as_secs_f32())
    }

    /// Returns the value at `progress` through the whole track, where `0.0` is
    /// the start and `1.0` the last keyframe. Out-of-range values clamp.
    pub fn at_progress(&self, progress: f32) -> T {
        self.animation.at(progress)
    }
}

/// Builds a [`KeyframeTrack`] one keyframe at a time.
///
/// Keyframes may be added in any order.
#[derive(Debug, Clone)]
pub struct TimedKeyframeBuilder<T: Animatable + Clone> {
    frames: Vec<(Duration, Keyframe<T>)>,
}

impl<T: Animatable + Clone> Default for TimedKeyframeBuilder<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Animatable + Clone> TimedKeyframeBuilder<T> {
    /// Creates a builder with no keyframes.
    pub fn new() -> Self {
        Self { frames: Vec::new() }
    }

    /// Adds `value` at `offset`, reached by linear interpolation.
    pub fn at(self, offset: Duration, value: T) -> Self {
        self.at_with(offset, value, AnyCurve::default())
    }

    /// Adds `value` at `offset`, eased by `curve` on the way in from the
    /// previous keyframe.
    pub fn at_with(mut self, offset: Duration, value: T, curve: impl Curve) -> Self {
        self.frames.push((offset, Keyframe::new(value, curve)));
        self
    }

    /// Finishes the track.
    ///
    /// # Errors
    ///
    /// [`KeyframeError::Empty`] if nothing was added.
    pub fn build(self) -> Result<KeyframeTrack<T>, KeyframeError> {
        let duration = self
            .frames
            .iter()
            .map(|(offset, _)| *offset)
            .max()
            .ok_or(KeyframeError::Empty)?;
        let frames = self
            .frames
            .into_iter()
            .map(|(offset, keyframe)| {
                // A zero-length track puts every keyframe at fraction zero;
                // `KeyframeTrack::at` handles that case explicitly.
                let fraction = if duration.is_zero() {
                    0.0
                } else {
                    offset.as_secs_f32() / duration.as_secs_f32()
                };
                (fraction, keyframe)
            })
            .collect();
        Ok(KeyframeTrack {
            animation: KeyframeAnimation::new(frames),
            duration,
        })
    }
}

/// A [`KeyframeTrack`] that plays itself.
///
/// It runs the track on a linear [`AnimationController`] whose duration is the
/// track's, so each keyframe's own curve is the only easing applied. Clones
/// share one playback.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimInstant, KeyframeTrack, TimedKeyframeAnimation};
///
/// let track = KeyframeTrack::builder()
///     .at(Duration::ZERO, 0.0f32)
///     .at(Duration::from_millis(100), 10.0)
///     .at(Duration::from_millis(300), 0.0)
///     .build()
///     .unwrap();
/// let animation = TimedKeyframeAnimation::new(track);
///
/// animation.forward_from_first_tick();
/// let start = AnimInstant::now();
/// animation.tick(start);
/// assert_eq!(animation.tick(start + Duration::from_millis(100)), 10.0);
/// ```
#[derive(Debug, Clone)]
pub struct TimedKeyframeAnimation<T: Animatable + Clone> {
    track: KeyframeTrack<T>,
    controller: AnimationController,
}

impl<T: Animatable + Clone> TimedKeyframeAnimation<T> {
    /// Plays `track` on a new controller. It starts stopped at the first value.
    pub fn new(track: KeyframeTrack<T>) -> Self {
        let controller = AnimationController::new(track.duration(), AnyCurve::default());
        Self { track, controller }
    }

    /// Returns the keyframes being played.
    pub fn track(&self) -> &KeyframeTrack<T> {
        &self.track
    }

    /// Returns the value at the controller's current progress without
    /// advancing time.
    pub fn value(&self) -> T {
        self.track.at_progress(self.controller.value())
    }

    /// Advances to `now` and returns the resulting value.
    ///
    /// Call it once per frame while [`is_animating`](Self::is_animating).
    pub fn tick(&self, now: AnimInstant) -> T {
        self.track.at_progress(self.controller.tick(now))
    }

    controller_playback!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::controller::AnimationStatus;
    use crate::primitives::curve::curves;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn pulse() -> KeyframeTrack<f32> {
        KeyframeTrack::builder()
            .at(ms(0), 0.0f32)
            .at(ms(100), 10.0)
            .at(ms(300), 0.0)
            .build()
            .unwrap()
    }

    #[test]
    fn duration_is_the_last_keyframe_offset() {
        assert_eq!(pulse().duration(), ms(300));
        assert_eq!(pulse().len(), 3);
    }

    #[test]
    fn samples_between_keyframes_by_time() {
        let track = pulse();

        assert!((track.at(ms(50)) - 5.0).abs() < 1e-4);
        assert!((track.at(ms(100)) - 10.0).abs() < 1e-4);
        assert!((track.at(ms(200)) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn holds_the_end_values_outside_the_track() {
        let track = KeyframeTrack::builder()
            .at(ms(100), 1.0f32)
            .at(ms(200), 3.0)
            .build()
            .unwrap();

        assert_eq!(track.at(ms(0)), 1.0);
        assert_eq!(track.at(ms(999)), 3.0);
    }

    #[test]
    fn keyframes_may_be_added_out_of_order() {
        let track = KeyframeTrack::builder()
            .at(ms(200), 20.0f32)
            .at(ms(0), 0.0)
            .build()
            .unwrap();

        assert!((track.at(ms(100)) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn per_keyframe_curves_ease_the_incoming_segment() {
        let track = KeyframeTrack::builder()
            .at(ms(0), 0.0f32)
            .at_with(ms(100), 100.0, curves::EASE_IN)
            .build()
            .unwrap();

        // EaseIn is t^3: halfway through the segment is 12.5% of the change.
        assert!((track.at(ms(50)) - 12.5).abs() < 1e-3);
    }

    #[test]
    fn a_zero_length_track_is_always_its_last_value() {
        let track = KeyframeTrack::builder()
            .at(ms(0), 1.0f32)
            .at(ms(0), 7.0)
            .build()
            .unwrap();

        assert_eq!(track.duration(), Duration::ZERO);
        assert_eq!(track.at(ms(0)), 7.0);
        assert_eq!(track.at(ms(50)), 7.0);
    }

    #[test]
    fn building_nothing_is_an_error() {
        let result = KeyframeTrack::<f32>::builder().build();

        assert_eq!(result.unwrap_err(), KeyframeError::Empty);
    }

    #[test]
    fn a_track_is_never_empty_once_built() {
        assert!(!pulse().is_empty());
    }

    #[test]
    fn at_progress_covers_the_whole_track() {
        let track = pulse();

        assert_eq!(track.at_progress(0.0), 0.0);
        assert_eq!(track.at_progress(1.0), 0.0);
        assert!((track.at_progress(1.0 / 3.0) - 10.0).abs() < 1e-4);
        assert_eq!(track.at_progress(f32::INFINITY), 0.0);
    }

    #[test]
    fn the_animation_plays_the_track_in_real_time() {
        let animation = TimedKeyframeAnimation::new(pulse());
        assert_eq!(animation.value(), 0.0);

        animation.forward_from_first_tick();
        let start = AnimInstant::now();
        animation.tick(start);

        assert!((animation.tick(start + ms(100)) - 10.0).abs() < 1e-3);
        assert!((animation.tick(start + ms(200)) - 5.0).abs() < 1e-3);
        assert_eq!(animation.tick(start + ms(300)), 0.0);
        assert_eq!(animation.status(), AnimationStatus::Completed);
    }

    #[test]
    fn the_controller_duration_matches_the_track() {
        let animation = TimedKeyframeAnimation::new(pulse());

        assert_eq!(animation.controller().duration(), ms(300));
        assert_eq!(animation.controller().curve(), curves::LINEAR.into_any());
    }

    #[test]
    fn value_reports_the_last_ticked_value_without_advancing() {
        let animation = TimedKeyframeAnimation::new(pulse());
        animation.forward_from_first_tick();
        let start = AnimInstant::now();
        animation.tick(start);

        let ticked = animation.tick(start + ms(150));

        assert_eq!(animation.value(), ticked);
    }

    #[test]
    fn repeat_restarts_the_track() {
        let animation = TimedKeyframeAnimation::new(pulse()).repeat(true);
        animation.forward_from_first_tick();
        let start = AnimInstant::now();
        animation.tick(start);

        animation.tick(start + ms(300));

        assert!(animation.is_animating());
        assert_eq!(animation.status(), AnimationStatus::Forward);
    }
}
