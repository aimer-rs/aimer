use std::rc::Rc;
use std::time::Duration;

use crate::local_cell::LocalCell;
use crate::primitives::curve::{AnyCurve, Curve};
use crate::primitives::time::AnimInstant;

/// The current status of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationStatus {
    /// Animation is playing forward (0 → 1).
    Forward,
    /// Animation is playing in reverse (1 → 0).
    Reverse,
    /// Animation completed in the forward direction (value = 1.0).
    Completed,
    /// Animation completed in the reverse direction (value = 0.0).
    Dismissed,
}

/// A trait for observing animation status changes.
///
/// Implement this to react to animation lifecycle events.
pub trait StatusListener: Send + Sync {
    fn on_status_changed(&self, status: AnimationStatus);
}

/// Controls an animation's timing, progress, and direction.
///
/// The controller produces a `value` in `[0.0, 1.0]` that changes over
/// `duration` according to the specified `curve`. Call [`Self::tick`] each
/// frame to advance.
///
/// # Listeners
///
/// Register a [`StatusListener`] to be notified when the animation status
/// changes (e.g. when it completes or is dismissed). Listeners are stored as
/// `Rc<dyn StatusListener>` so the controller remains `Clone`-able. Clones are
/// handles to the same playback state, allowing a widget and its owner to
/// control one animation.
#[derive(Clone)]
pub struct AnimationController {
    state: Rc<LocalCell<ControllerState>>,
    listeners: Rc<LocalCell<Vec<Rc<dyn StatusListener>>>>,
}

#[derive(Debug)]
struct ControllerState {
    duration: Duration,
    curve: AnyCurve,
    value: f32,
    status: AnimationStatus,
    start_time: Option<AnimInstant>,
    start_value: f32,
    repeat: bool,
    auto_reverse: bool,
}

impl std::fmt::Debug for AnimationController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.state.with(|state| {
            f.debug_struct("AnimationController")
                .field("duration", &state.duration)
                .field("curve", &state.curve)
                .field("value", &state.value)
                .field("status", &state.status)
                .field("repeat", &state.repeat)
                .field("auto_reverse", &state.auto_reverse)
                .finish()
        })
    }
}

impl AnimationController {
    /// Create a new controller with the given duration and curve.
    pub fn new(duration: Duration, curve: impl Curve) -> Self {
        Self {
            state: Rc::new(LocalCell::new(ControllerState {
                duration,
                curve: curve.into_any(),
                value: 0.0,
                status: AnimationStatus::Dismissed,
                start_time: None,
                start_value: 0.0,
                repeat: false,
                auto_reverse: false,
            })),
            listeners: Rc::new(LocalCell::new(Vec::new())),
        }
    }

    /// Create a controller with a duration in milliseconds.
    pub fn with_millis(millis: u64, curve: impl Curve) -> Self {
        Self::new(Duration::from_millis(millis), curve)
    }

    /// Register a status listener. The listener will be called whenever
    /// the animation status changes during `tick()`.
    pub fn add_status_listener(&self, listener: Rc<dyn StatusListener>) {
        self.listeners
            .with_mut(|listeners| listeners.push(listener));
    }

    /// Start playing the animation forward from the current value.
    pub fn forward(&self) {
        let changed = self.state.with_mut(|state| {
            state.start_value = state.value;
            state.start_time = Some(AnimInstant::now());
            Self::set_status(state, AnimationStatus::Forward)
        });
        self.notify_if_changed(changed, AnimationStatus::Forward);
    }

    /// Arm forward playback while deferring the elapsed-time clock until the
    /// first [`Self::tick`]. This prevents widget construction between starting
    /// an animation and rendering its first frame from consuming its duration.
    pub fn forward_from_first_tick(&self) {
        let changed = self.state.with_mut(|state| {
            state.start_value = state.value;
            state.start_time = None;
            Self::set_status(state, AnimationStatus::Forward)
        });
        self.notify_if_changed(changed, AnimationStatus::Forward);
    }

    /// Start playing the animation in reverse from the current value.
    pub fn reverse(&self) {
        let changed = self.state.with_mut(|state| {
            state.start_value = state.value;
            state.start_time = Some(AnimInstant::now());
            Self::set_status(state, AnimationStatus::Reverse)
        });
        self.notify_if_changed(changed, AnimationStatus::Reverse);
    }

    /// Reset the animation to the beginning (value = 0, dismissed).
    pub fn reset(&self) {
        let changed = self.state.with_mut(|state| {
            state.value = 0.0;
            state.start_value = 0.0;
            state.start_time = None;
            Self::set_status(state, AnimationStatus::Dismissed)
        });
        self.notify_if_changed(changed, AnimationStatus::Dismissed);
    }

    /// Returns `true` if the animation is currently running (Forward or
    /// Reverse).
    pub fn is_animating(&self) -> bool {
        matches!(
            self.status(),
            AnimationStatus::Forward | AnimationStatus::Reverse
        )
    }

    pub fn duration(&self) -> Duration {
        self.state.with(|state| state.duration)
    }

    pub fn set_duration(&self, duration: Duration) {
        self.state.with_mut(|state| state.duration = duration);
    }

    /// Returns the easing curve.
    pub fn curve(&self) -> AnyCurve {
        self.state.with(|state| state.curve.clone())
    }

    /// Replaces the easing curve. Clones of the controller share it.
    pub fn set_curve(&self, curve: impl Curve) {
        let curve = curve.into_any();
        self.state.with_mut(|state| state.curve = curve);
    }

    pub fn value(&self) -> f32 {
        self.state.with(|state| state.value)
    }

    pub fn set_value(&self, value: f32) {
        self.state.with_mut(|state| {
            state.value = value.clamp(0.0, 1.0);
            state.start_value = state.value;
        });
    }

    pub fn status(&self) -> AnimationStatus {
        self.state.with(|state| state.status)
    }

    pub fn repeat(&self) -> bool {
        self.state.with(|state| state.repeat)
    }

    pub fn set_repeat(&self, repeat: bool) {
        self.state.with_mut(|state| state.repeat = repeat);
    }

    pub fn auto_reverse(&self) -> bool {
        self.state.with(|state| state.auto_reverse)
    }

    pub fn set_auto_reverse(&self, auto_reverse: bool) {
        self.state
            .with_mut(|state| state.auto_reverse = auto_reverse);
    }

    fn set_status(state: &mut ControllerState, new_status: AnimationStatus) -> bool {
        if state.status == new_status {
            false
        } else {
            state.status = new_status;
            true
        }
    }

    fn notify_if_changed(&self, changed: bool, status: AnimationStatus) {
        if changed {
            let listeners = self.listeners.with(Clone::clone);
            for listener in &listeners {
                listener.on_status_changed(status);
            }
        }
    }

    /// Advance the animation to the current time. Returns the new curved value.
    ///
    /// Call this once per frame. When the animation completes, the status is
    /// updated to `Completed` or `Dismissed` and `is_animating()` returns
    /// false.
    pub fn tick(&self, now: AnimInstant) -> f32 {
        let (curved, notification) = self.state.with_mut(|state| {
            let Some(start) = state.start_time else {
                if matches!(
                    state.status,
                    AnimationStatus::Forward | AnimationStatus::Reverse
                ) {
                    state.start_time = Some(now);
                }
                return (state.curve.transform(state.value), None);
            };
            if !matches!(
                state.status,
                AnimationStatus::Forward | AnimationStatus::Reverse
            ) {
                return (state.curve.transform(state.value), None);
            }

            let elapsed = now.duration_since(start).as_secs_f32();
            let linear_delta = if state.duration.is_zero() {
                1.0
            } else {
                elapsed / state.duration.as_secs_f32()
            };
            let status = state.status;
            let mut notification = None;

            match status {
                AnimationStatus::Forward => {
                    state.value = (state.start_value + linear_delta).min(1.0);
                    if state.value >= 1.0 {
                        if state.repeat {
                            if state.auto_reverse {
                                if Self::set_status(state, AnimationStatus::Reverse) {
                                    notification = Some(AnimationStatus::Reverse);
                                }
                                state.start_value = 1.0;
                            } else {
                                state.start_value = 0.0;
                                state.value = 0.0;
                            }
                            state.start_time = Some(now);
                        } else {
                            state.start_time = None;
                            if Self::set_status(state, AnimationStatus::Completed) {
                                notification = Some(AnimationStatus::Completed);
                            }
                        }
                    }
                }
                AnimationStatus::Reverse => {
                    state.value = (state.start_value - linear_delta).max(0.0);
                    if state.value <= 0.0 {
                        if state.repeat {
                            if state.auto_reverse {
                                if Self::set_status(state, AnimationStatus::Forward) {
                                    notification = Some(AnimationStatus::Forward);
                                }
                                state.start_value = 0.0;
                            } else {
                                state.start_value = 1.0;
                                state.value = 1.0;
                            }
                            state.start_time = Some(now);
                        } else {
                            state.start_time = None;
                            if Self::set_status(state, AnimationStatus::Dismissed) {
                                notification = Some(AnimationStatus::Dismissed);
                            }
                        }
                    }
                }
                _ => {}
            }

            (state.curve.transform(state.value), notification)
        });

        if let Some(status) = notification {
            self.notify_if_changed(true, status);
        }
        curved
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use super::*;
    use crate::primitives::curve::curves;

    #[test]
    fn test_forward_completes() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.forward();
        assert!(ctrl.is_animating());

        let start = AnimInstant::now();
        let end = start + Duration::from_millis(150);
        ctrl.tick(end);

        assert_eq!(ctrl.status(), AnimationStatus::Completed);
        assert!(!ctrl.is_animating());
        assert!((ctrl.value() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_reverse_completes() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.set_value(1.0);
        ctrl.reverse();

        let start = AnimInstant::now();
        let end = start + Duration::from_millis(150);
        ctrl.tick(end);

        assert_eq!(ctrl.status(), AnimationStatus::Dismissed);
        assert!((ctrl.value() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_reset() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.forward();
        ctrl.set_value(0.5);
        ctrl.reset();

        assert_eq!(ctrl.status(), AnimationStatus::Dismissed);
        assert!((ctrl.value() - 0.0).abs() < 1e-9);
        assert!(!ctrl.is_animating());
    }

    #[test]
    fn test_repeat_forward() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.set_repeat(true);
        ctrl.forward();

        let start = AnimInstant::now();
        let end = start + Duration::from_millis(150);
        ctrl.tick(end);

        assert!(ctrl.is_animating());
        assert_eq!(ctrl.status(), AnimationStatus::Forward);
    }

    #[test]
    fn test_repeat_auto_reverse() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.set_repeat(true);
        ctrl.set_auto_reverse(true);
        ctrl.forward();

        let start = AnimInstant::now();
        let end = start + Duration::from_millis(150);
        ctrl.tick(end);

        assert!(ctrl.is_animating());
        assert_eq!(ctrl.status(), AnimationStatus::Reverse);
    }

    #[test]
    fn test_status_listener_called_on_complete() {
        struct TestListener(AtomicU8);
        impl StatusListener for TestListener {
            fn on_status_changed(&self, _status: AnimationStatus) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let listener = Rc::new(TestListener(AtomicU8::new(0)));
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.add_status_listener(listener.clone());

        ctrl.forward();
        assert_eq!(listener.0.load(Ordering::Relaxed), 1); // Forward

        let start = AnimInstant::now();
        let end = start + Duration::from_millis(150);
        ctrl.tick(end);

        // Two transitions: Forward → (no change in tick) → Completed
        assert_eq!(listener.0.load(Ordering::Relaxed), 2);
        assert_eq!(ctrl.status(), AnimationStatus::Completed);
    }

    #[test]
    fn test_status_listener_shared_across_clones() {
        struct TestListener(AtomicU8);
        impl StatusListener for TestListener {
            fn on_status_changed(&self, _status: AnimationStatus) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }

        let listener = Rc::new(TestListener(AtomicU8::new(0)));
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.add_status_listener(listener.clone());

        let ctrl2 = ctrl.clone();
        ctrl2.forward();
        assert_eq!(listener.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn clone_shares_progress_and_playback_state() {
        let first = AnimationController::with_millis(100, curves::LINEAR);
        let second = first.clone();

        first.forward();
        let halfway =
            first.state.with(|state| state.start_time.unwrap()) + Duration::from_millis(50);
        first.tick(halfway);

        assert!((second.value() - 0.5).abs() < 0.01);
        assert_eq!(second.status(), AnimationStatus::Forward);

        second.reverse();
        assert_eq!(first.status(), AnimationStatus::Reverse);
    }

    #[test]
    fn forward_continues_from_current_value() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.set_value(0.4);
        ctrl.forward();

        let halfway_through_remaining =
            ctrl.state.with(|state| state.start_time.unwrap()) + Duration::from_millis(30);
        let value = ctrl.tick(halfway_through_remaining);

        assert!((value - 0.7).abs() < 0.01, "expected 0.7, got {value}");
    }

    #[test]
    fn reverse_continues_from_current_value() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        ctrl.set_value(0.6);
        ctrl.reverse();

        let halfway_through_remaining =
            ctrl.state.with(|state| state.start_time.unwrap()) + Duration::from_millis(30);
        let value = ctrl.tick(halfway_through_remaining);

        assert!((value - 0.3).abs() < 0.01, "expected 0.3, got {value}");
    }

    #[test]
    fn sub_millisecond_duration_advances_without_nan() {
        let ctrl = AnimationController::new(Duration::from_micros(500), curves::LINEAR);
        ctrl.forward();

        let quarter =
            ctrl.state.with(|state| state.start_time.unwrap()) + Duration::from_micros(125);
        let value = ctrl.tick(quarter);

        assert!(value.is_finite());
        assert!((value - 0.25).abs() < 0.01, "expected 0.25, got {value}");
    }

    /// A user's curve: it reaches the end at half time and stays there.
    struct EarlyFinish;

    impl Curve for EarlyFinish {
        fn transform(&self, t: f32) -> f32 {
            (t * 2.0).min(1.0)
        }
    }

    #[test]
    fn a_custom_curve_shapes_the_controller_output() {
        let ctrl = AnimationController::with_millis(100, EarlyFinish);
        ctrl.forward_from_first_tick();
        let start = AnimInstant::now();
        ctrl.tick(start);

        let quarter = ctrl.tick(start + Duration::from_millis(25));
        let three_quarters = ctrl.tick(start + Duration::from_millis(75));

        assert!((quarter - 0.5).abs() < 1e-4, "got {quarter}");
        assert_eq!(three_quarters, 1.0);
    }

    #[test]
    fn a_custom_curve_can_replace_a_standard_one_and_back() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);

        ctrl.set_curve(EarlyFinish);
        assert_ne!(ctrl.curve(), curves::LINEAR.into_any());
        assert_eq!(ctrl.curve().transform(0.25), 0.5);

        ctrl.set_curve(curves::LINEAR);
        assert_eq!(ctrl.curve(), curves::LINEAR.into_any());
    }

    #[test]
    fn clones_share_a_custom_curve() {
        let ctrl = AnimationController::with_millis(100, curves::LINEAR);
        let handle = ctrl.clone();

        handle.set_curve(EarlyFinish);

        assert_eq!(ctrl.curve().transform(0.25), 0.5);
    }
}
