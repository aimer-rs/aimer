use std::time::Duration;

use crate::control::controller::AnimationController;
use crate::control::playback::controller_playback;
use crate::primitives::animatable::Animatable;
use crate::primitives::curve::Curve;
use crate::primitives::time::AnimInstant;
use crate::primitives::tween::Tween;

/// A value of type `T` that animates from `begin` to `end` over time.
///
/// `TweenAnimation` pairs a [`Tween`] with the [`AnimationController`] that
/// drives it, so callers no longer hold a controller and a tween side by side
/// and remember to sample one through the other. The controller owns timing,
/// direction and easing; the tween maps its curved progress to a `T`.
///
/// Clones are handles to the same playback, like the controller they wrap.
///
/// # Example
///
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimInstant, curves, TweenAnimation};
///
/// let slide = TweenAnimation::new(0.0f32, 100.0, Duration::from_millis(200), curves::LINEAR);
/// slide.forward_from_first_tick();
///
/// let start = AnimInstant::now();
/// slide.tick(start);
/// assert_eq!(slide.tick(start + Duration::from_millis(100)), 50.0);
/// ```
#[derive(Debug, Clone)]
pub struct TweenAnimation<T: Animatable> {
    tween: Tween<T>,
    controller: AnimationController,
}

impl<T: Animatable> TweenAnimation<T> {
    /// Creates an animation from `begin` to `end`.
    ///
    /// It starts stopped at `begin`; call [`forward`](Self::forward) to play it.
    pub fn new(begin: T, end: T, duration: Duration, curve: impl Curve) -> Self {
        Self::from_tween(
            Tween::new(begin, end),
            AnimationController::new(duration, curve),
        )
    }

    /// Drives an existing `tween` with an existing `controller`.
    ///
    /// Use this to share a controller with other animations or widgets.
    pub fn from_tween(tween: Tween<T>, controller: AnimationController) -> Self {
        Self { tween, controller }
    }

    /// Returns the value range being animated.
    pub fn tween(&self) -> &Tween<T> {
        &self.tween
    }

    /// Returns the value at the controller's current progress, with the
    /// controller's curve applied, without advancing time.
    pub fn value(&self) -> T {
        self.tween
            .lerp(self.controller.curve().transform(self.controller.value()))
    }

    /// Advances to `now` and returns the resulting value.
    ///
    /// Call it once per frame while [`is_animating`](Self::is_animating).
    pub fn tick(&self, now: AnimInstant) -> T {
        self.tween.lerp(self.controller.tick(now))
    }

    controller_playback!();
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::control::controller::{AnimationStatus, StatusListener};
    use crate::primitives::curve::curves;

    const HUNDRED_MS: Duration = Duration::from_millis(100);

    fn linear(begin: f32, end: f32) -> TweenAnimation<f32> {
        TweenAnimation::new(begin, end, HUNDRED_MS, curves::LINEAR)
    }

    /// Arms playback and starts the clock at a fixed instant so assertions do
    /// not depend on wall-clock time.
    fn started(animation: &TweenAnimation<f32>) -> AnimInstant {
        animation.forward_from_first_tick();
        let start = AnimInstant::now();
        animation.tick(start);
        start
    }

    #[test]
    fn starts_stopped_at_the_begin_value() {
        let animation = linear(10.0, 20.0);

        assert_eq!(animation.value(), 10.0);
        assert_eq!(animation.status(), AnimationStatus::Dismissed);
        assert!(!animation.is_animating());
    }

    #[test]
    fn forward_interpolates_over_time_and_completes_at_the_end_value() {
        let animation = linear(0.0, 100.0);
        let start = started(&animation);

        assert!(animation.is_animating());
        assert_eq!(animation.tick(start + Duration::from_millis(50)), 50.0);
        assert_eq!(animation.tick(start + HUNDRED_MS), 100.0);
        assert_eq!(animation.status(), AnimationStatus::Completed);
        assert!(!animation.is_animating());
    }

    #[test]
    fn controller_curve_shapes_the_value() {
        let animation = TweenAnimation::new(0.0f32, 100.0, HUNDRED_MS, curves::EASE_IN);
        let start = started(&animation);

        // EaseIn is t^3, so halfway through the time is 12.5% of the range.
        let value = animation.tick(start + Duration::from_millis(50));
        assert!((value - 12.5).abs() < 1e-3, "got {value}");
    }

    #[test]
    fn value_reports_the_last_ticked_value_without_advancing() {
        let animation = TweenAnimation::new(0.0f32, 100.0, HUNDRED_MS, curves::EASE_IN);
        let start = started(&animation);

        let ticked = animation.tick(start + Duration::from_millis(50));

        assert_eq!(animation.value(), ticked);
        assert_eq!(animation.value(), ticked);
    }

    #[test]
    fn reverse_returns_to_the_begin_value() {
        let animation = linear(0.0, 100.0);
        let start = started(&animation);
        animation.tick(start + HUNDRED_MS);

        animation.reverse();
        animation.tick(AnimInstant::now() + Duration::from_secs(1));

        assert_eq!(animation.status(), AnimationStatus::Dismissed);
        assert_eq!(animation.value(), 0.0);
    }

    #[test]
    fn reset_stops_at_the_begin_value() {
        let animation = linear(5.0, 9.0);
        let start = started(&animation);
        animation.tick(start + Duration::from_millis(50));

        animation.reset();

        assert_eq!(animation.value(), 5.0);
        assert_eq!(animation.status(), AnimationStatus::Dismissed);
    }

    #[test]
    fn repeat_with_auto_reverse_turns_around_at_the_end() {
        let animation = linear(0.0, 100.0).repeat(true).auto_reverse(true);
        let start = started(&animation);

        animation.tick(start + HUNDRED_MS);

        assert_eq!(animation.status(), AnimationStatus::Reverse);
        assert!(animation.is_animating());
    }

    #[test]
    fn interpolates_any_animatable_type() {
        let animation = TweenAnimation::new((0.0f32, 10.0f32), (100.0, 30.0), HUNDRED_MS, curves::LINEAR);
        animation.forward_from_first_tick();
        let start = AnimInstant::now();
        animation.tick(start);

        assert_eq!(animation.tick(start + Duration::from_millis(50)), (50.0, 20.0));
    }

    #[test]
    fn clones_share_one_playback() {
        let animation = linear(0.0, 100.0);
        let handle = animation.clone();

        handle.forward();

        assert!(animation.is_animating());
    }

    #[test]
    fn status_listeners_observe_playback() {
        struct Counter(AtomicUsize);
        impl StatusListener for Counter {
            fn on_status_changed(&self, _: AnimationStatus) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let animation = linear(0.0, 1.0);
        let counter = Rc::new(Counter(AtomicUsize::new(0)));
        animation.add_status_listener(counter.clone());

        animation.forward();

        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn from_tween_shares_the_supplied_controller() {
        let controller = AnimationController::new(HUNDRED_MS, curves::LINEAR);
        let animation = TweenAnimation::from_tween(Tween::new(0.0f32, 4.0), controller.clone());

        controller.set_value(0.5);

        assert_eq!(animation.value(), 2.0);
    }
}
