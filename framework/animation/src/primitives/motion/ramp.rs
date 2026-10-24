//! The constant-speed motion.

use std::time::Duration;

use super::{AnyMotion, Motion, MotionDescriptor, MotionState};

/// The descriptor tag of a ramp.
pub(super) const RAMP_TAG: i64 = 1;

/// A motion at constant speed: it covers one unit of distance in a set time.
///
/// Where a [`Spring`](super::Spring) accelerates and overshoots, a ramp just
/// moves, at the same pace from the first frame to the last, and stops dead on
/// the target. That makes its duration exact, which is what a fade needs: a
/// ramp of 150 ms takes the value from `0.0` to `1.0` in 150 ms, and from `0.0`
/// to `0.5` in half that. A shorter move takes proportionally less time.
///
/// A zero duration jumps to the target.
///
/// ```
/// use std::time::Duration;
/// use aimer_animation::{Motion, MotionState, motions};
///
/// let fade = motions::ramp(Duration::from_millis(100));
/// let halfway = fade.step(MotionState::at_rest(0.0), 1.0, 0.05);
/// assert!((halfway.value - 0.5).abs() < 1.0e-6);
/// assert_eq!(fade.step(halfway, 1.0, 0.1), MotionState::at_rest(1.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ramp {
    duration: Duration,
}

/// Creates a [`Ramp`] that covers one unit of distance in `duration`.
pub const fn ramp(duration: Duration) -> Ramp {
    Ramp { duration }
}

impl Ramp {
    /// The time the ramp takes to cover one unit of distance.
    #[inline]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Rebuilds a ramp from its [`MotionDescriptor`] parameters, or `None` if
    /// they are not usable.
    pub(super) fn from_params(params: [f64; 2]) -> Option<Self> {
        let seconds = params[0];
        (seconds.is_finite() && (0.0..=Duration::MAX.as_secs_f64()).contains(&seconds))
            .then(|| ramp(Duration::from_secs_f64(seconds)))
    }
}

impl Motion for Ramp {
    fn step(&self, state: MotionState, target: f32, dt: f32) -> MotionState {
        // A target that is not a number gives the ramp nothing to move toward.
        if !target.is_finite() {
            return state;
        }
        // A state or step that cannot be advanced, and a ramp with no duration,
        // go straight to the target: a control must never be left stuck.
        let seconds = self.duration.as_secs_f32();
        if !state.is_finite() || dt == f32::INFINITY || seconds <= 0.0 {
            return MotionState::at_rest(target);
        }
        if dt.is_nan() || dt <= 0.0 {
            return state;
        }

        let remaining = target - state.value;
        let pace = 1.0 / seconds;
        if remaining.abs() <= pace * dt {
            return MotionState::at_rest(target);
        }
        let direction = remaining.signum();
        MotionState {
            value: state.value + direction * pace * dt,
            velocity: direction * pace,
        }
    }

    fn descriptor(&self) -> Option<MotionDescriptor> {
        Some(MotionDescriptor {
            tag: RAMP_TAG,
            params: [self.duration.as_secs_f64(), 0.0],
        })
    }

    fn into_any(self) -> AnyMotion {
        AnyMotion::ramp(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f32 = 1.0 / 120.0;

    fn ramp_ms(millis: u64) -> Ramp {
        ramp(Duration::from_millis(millis))
    }

    #[test]
    fn a_ramp_covers_one_unit_in_exactly_its_duration() {
        let fade = ramp_ms(150);
        let mut state = MotionState::at_rest(0.0);
        let mut elapsed = 0.0;
        while state.value < 1.0 {
            state = fade.step(state, 1.0, FRAME);
            elapsed += FRAME;
            assert!(elapsed < 0.2, "never arrived: {state:?}");
        }
        assert!((elapsed - 0.15).abs() <= FRAME, "took {elapsed} s");
        assert_eq!(state, MotionState::at_rest(1.0));
    }

    #[test]
    fn it_moves_at_a_constant_pace_both_ways() {
        let fade = ramp_ms(100);
        let up = fade.step(MotionState::at_rest(0.0), 1.0, 0.025);
        assert!((up.value - 0.25).abs() < 1.0e-6, "{up:?}");
        assert!((up.velocity - 10.0).abs() < 1.0e-4, "{up:?}");
        let down = fade.step(MotionState::at_rest(1.0), 0.0, 0.025);
        assert!((down.value - 0.75).abs() < 1.0e-6, "{down:?}");
        assert!((down.velocity + 10.0).abs() < 1.0e-4, "{down:?}");
    }

    #[test]
    fn a_shorter_move_takes_proportionally_less_time() {
        let fade = ramp_ms(100);
        assert_eq!(fade.step(MotionState::at_rest(0.0), 0.3, 0.031), MotionState::at_rest(0.3));
        let part = fade.step(MotionState::at_rest(0.0), 0.3, 0.02);
        assert!((part.value - 0.2).abs() < 1.0e-6, "{part:?}");
    }

    #[test]
    fn it_stops_on_the_target_without_overshooting() {
        let fade = ramp_ms(100);
        let done = fade.step(MotionState { value: 0.99, velocity: 10.0 }, 1.0, 1.0);
        assert_eq!(done, MotionState::at_rest(1.0));
        assert!(fade.is_settled(done, 1.0));
        assert!(!fade.is_settled(fade.step(MotionState::at_rest(0.0), 1.0, 0.01), 1.0));
    }

    #[test]
    fn the_result_does_not_depend_on_the_frame_rate() {
        let fade = ramp_ms(200);
        let one = fade.step(MotionState::at_rest(0.0), 1.0, 0.1);
        let mut many = MotionState::at_rest(0.0);
        for _ in 0..100 {
            many = fade.step(many, 1.0, 0.001);
        }
        assert!((one.value - many.value).abs() < 1.0e-4, "{one:?} vs {many:?}");
    }

    #[test]
    fn a_zero_duration_jumps_to_the_target() {
        let jump = ramp(Duration::ZERO);
        assert!(jump.is_valid());
        assert_eq!(jump.step(MotionState::at_rest(0.0), 1.0, 0.001), MotionState::at_rest(1.0));
    }

    #[test]
    fn bad_input_never_poisons_the_state() {
        let fade = ramp_ms(100);
        let state = MotionState { value: 0.25, velocity: 1.0 };
        assert_eq!(fade.step(state, 1.0, f32::NAN), state);
        assert_eq!(fade.step(state, 1.0, -1.0), state);
        assert_eq!(fade.step(state, 1.0, 0.0), state);
        assert_eq!(fade.step(state, f32::NAN, 0.01), state);
        assert_eq!(fade.step(state, 1.0, f32::INFINITY), MotionState::at_rest(1.0));
        assert_eq!(
            fade.step(MotionState { value: f32::NAN, velocity: 0.0 }, 1.0, 0.01),
            MotionState::at_rest(1.0)
        );
    }

    #[test]
    fn it_describes_itself_for_a_portable_guest() {
        let fade = ramp_ms(250);
        let descriptor = fade.descriptor().expect("a ramp is describable");
        assert_eq!(descriptor.tag, RAMP_TAG);
        assert_eq!(Ramp::from_params(descriptor.params), Some(fade));
        assert_eq!(Ramp::from_params([f64::NAN, 0.0]), None);
        assert_eq!(Ramp::from_params([-1.0, 0.0]), None);
    }
}
