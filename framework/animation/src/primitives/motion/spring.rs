//! The spring motion.

use super::{AnyMotion, Motion, MotionDescriptor, MotionState};

/// The descriptor tag of a spring.
pub(super) const SPRING_TAG: i64 = 0;

/// Damping ratios this close to `1.0` use the critically damped formula: the
/// under- and over-damped ones divide by a term that vanishes there.
const CRITICAL_BAND: f64 = 1.0e-4;

/// A damped spring.
///
/// A spring pulls the value toward the target with a force proportional to the
/// distance, and resists with a force proportional to the speed. It is described
/// by its `stiffness` (how hard it pulls, which sets how fast it moves) and its
/// `damping_ratio` (how much it resists): below `1.0` it overshoots and rings, at
/// `1.0` it arrives as fast as it can without overshooting, above `1.0` it
/// creeps in.
///
/// [`step`](Motion::step) evaluates the exact solution of the spring equation
/// rather than integrating it, so the result does not depend on how the time is
/// divided into frames, and a long frame cannot make it unstable.
///
/// ```
/// use aimer_animation::{Motion, MotionState, motions};
///
/// let mut state = MotionState::at_rest(0.0);
/// for _ in 0..120 {
///     state = motions::BOUNCY.step(state, 1.0, 1.0 / 120.0);
/// }
/// assert!(motions::BOUNCY.is_settled(state, 1.0));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    stiffness: f32,
    damping_ratio: f32,
}

/// Creates a [`Spring`]. `stiffness` is in 1/s² (a unit mass is assumed) and must
/// be positive; `damping_ratio` is dimensionless and must not be negative.
/// [`is_valid`](Motion::is_valid) reports whether both are usable.
pub const fn spring(stiffness: f32, damping_ratio: f32) -> Spring {
    Spring { stiffness, damping_ratio }
}

/// A critically damped spring: quick, and never overshoots.
pub const SMOOTH: Spring = spring(300.0, 1.0);
/// An under-damped spring that overshoots its target by about seven percent
/// before settling, as a lifted glass lens does when it lands.
pub const BOUNCY: Spring = spring(260.0, 0.65);
/// A stiff, nearly critically damped spring that settles fast with at most a
/// hint of overshoot.
pub const SNAPPY: Spring = spring(600.0, 0.85);

impl Spring {
    /// How hard the spring pulls toward the target, in 1/s².
    #[inline]
    pub const fn stiffness(&self) -> f32 {
        self.stiffness
    }

    /// How strongly the spring resists motion; `1.0` is critical damping.
    #[inline]
    pub const fn damping_ratio(&self) -> f32 {
        self.damping_ratio
    }

    /// Rebuilds a spring from its [`MotionDescriptor`] parameters, or `None` if
    /// they are not usable.
    pub(super) fn from_params(params: [f64; 2]) -> Option<Self> {
        let spring = spring(params[0] as f32, params[1] as f32);
        spring.is_valid().then_some(spring)
    }
}

impl Motion for Spring {
    fn step(&self, state: MotionState, target: f32, dt: f32) -> MotionState {
        // A target that is not a number gives the spring nothing to chase.
        if !target.is_finite() {
            return state;
        }
        // A broken spring, or a state or step that cannot be integrated, snaps
        // to the target: a control must never be left stuck mid-flight.
        if !self.is_valid() || !state.is_finite() || dt == f32::INFINITY {
            return MotionState::at_rest(target);
        }
        if dt.is_nan() || dt <= 0.0 {
            return state;
        }

        // The exact solution of x'' = -k x - c x' for the displacement `x` from
        // the target, in f64 so that a long frame loses no precision. A unit
        // mass is assumed, so the natural frequency is sqrt(k) and c = 2 z w.
        let omega = f64::from(self.stiffness).sqrt();
        let zeta = f64::from(self.damping_ratio);
        let t = f64::from(dt);
        let x0 = f64::from(state.value) - f64::from(target);
        let v0 = f64::from(state.velocity);

        let (x, v) = if (zeta - 1.0).abs() < CRITICAL_BAND {
            let decay = (-omega * t).exp();
            let b = v0 + omega * x0;
            (decay * (x0 + b * t), decay * (v0 - omega * b * t))
        } else if zeta < 1.0 {
            let damped = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * t).exp();
            let (sin, cos) = (damped * t).sin_cos();
            let b = (v0 + zeta * omega * x0) / damped;
            (
                decay * (x0 * cos + b * sin),
                decay * (v0 * cos - (zeta * omega * v0 + omega * omega * x0) / damped * sin),
            )
        } else {
            let root = omega * (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-zeta * omega + root, -zeta * omega - root);
            let c2 = (v0 - r1 * x0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        };

        MotionState {
            value: (f64::from(target) + x) as f32,
            velocity: v as f32,
        }
    }

    fn descriptor(&self) -> Option<MotionDescriptor> {
        Some(MotionDescriptor {
            tag: SPRING_TAG,
            params: [self.stiffness as f64, self.damping_ratio as f64],
        })
    }

    fn is_valid(&self) -> bool {
        self.stiffness.is_finite()
            && self.stiffness > 0.0
            && self.damping_ratio.is_finite()
            && self.damping_ratio >= 0.0
    }

    fn into_any(self) -> AnyMotion {
        AnyMotion::spring(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::motion::{REST_DISPLACEMENT, REST_VELOCITY};

    const FRAME: f32 = 1.0 / 120.0;

    /// Runs `spring` from `state` for `seconds`, in frames of `dt`, and returns
    /// every state it passed through.
    fn run(spring: Spring, mut state: MotionState, target: f32, dt: f32, seconds: f32) -> Vec<MotionState> {
        let frames = (seconds / dt).round() as usize;
        let mut trace = Vec::with_capacity(frames);
        for _ in 0..frames {
            state = spring.step(state, target, dt);
            trace.push(state);
        }
        trace
    }

    #[test]
    fn presets_are_valid_and_describe_themselves() {
        for preset in [SMOOTH, BOUNCY, SNAPPY] {
            assert!(preset.is_valid());
            let descriptor = preset.descriptor().expect("a spring is describable");
            assert_eq!(descriptor.tag, SPRING_TAG);
            assert_eq!(Spring::from_params(descriptor.params), Some(preset));
        }
    }

    #[test]
    fn zero_time_changes_nothing() {
        let state = MotionState { value: 0.3, velocity: 2.0 };
        assert_eq!(BOUNCY.step(state, 1.0, 0.0), state);
    }

    #[test]
    fn a_spring_arrives_at_its_target_and_stops() {
        for preset in [SMOOTH, BOUNCY, SNAPPY] {
            let trace = run(preset, MotionState::at_rest(0.0), 2.0, FRAME, 3.0);
            let last = *trace.last().unwrap();
            assert!(preset.is_settled(last, 2.0), "{preset:?} ended at {last:?}");
        }
    }

    #[test]
    fn a_critically_damped_spring_never_overshoots() {
        let trace = run(SMOOTH, MotionState::at_rest(0.0), 1.0, FRAME, 2.0);
        assert!(trace.iter().all(|state| state.value <= 1.0 + 1.0e-5));
        assert!(trace.windows(2).all(|pair| pair[1].value >= pair[0].value - 1.0e-6));
    }

    #[test]
    fn an_over_damped_spring_never_overshoots() {
        let trace = run(spring(300.0, 1.6), MotionState::at_rest(0.0), 1.0, FRAME, 3.0);
        assert!(trace.iter().all(|state| state.value <= 1.0 + 1.0e-5));
    }

    #[test]
    fn an_under_damped_spring_overshoots_by_the_textbook_amount() {
        let trace = run(BOUNCY, MotionState::at_rest(0.0), 1.0, 1.0 / 1000.0, 1.0);
        let peak = trace.iter().map(|state| state.value).fold(f32::MIN, f32::max);
        // The first overshoot of a damped oscillator is exp(-pi z / sqrt(1 - z^2)).
        let zeta = BOUNCY.damping_ratio();
        let expected = (-std::f32::consts::PI * zeta / (1.0 - zeta * zeta).sqrt()).exp();
        assert!((peak - 1.0 - expected).abs() < 1.0e-3, "peak {peak}, expected {expected}");
    }

    #[test]
    fn the_result_does_not_depend_on_the_frame_rate() {
        let start = MotionState { value: 0.2, velocity: -1.5 };
        for preset in [SMOOTH, BOUNCY, spring(300.0, 1.6)] {
            let one = preset.step(start, 1.0, 0.25);
            let many = run(preset, start, 1.0, 1.0 / 240.0, 0.25).pop().unwrap();
            assert!((one.value - many.value).abs() < 1.0e-4, "{preset:?}: {one:?} vs {many:?}");
            assert!((one.velocity - many.velocity).abs() < 1.0e-3, "{preset:?}: {one:?} vs {many:?}");
        }
    }

    #[test]
    fn a_retarget_keeps_the_speed_it_had() {
        // Moving up at 4/s, the target is pulled back below the value. The value
        // must keep rising for a moment instead of reversing instantly.
        let moving = MotionState { value: 0.5, velocity: 4.0 };
        let next = BOUNCY.step(moving, 0.0, 0.002);
        assert!(next.value > moving.value, "the value reversed at once: {next:?}");
        assert!(next.velocity < moving.velocity, "the pull must slow it down: {next:?}");
    }

    #[test]
    fn settling_needs_both_position_and_speed() {
        let target = 1.0;
        assert!(BOUNCY.is_settled(MotionState::at_rest(target), target));
        assert!(BOUNCY.is_settled(
            MotionState { value: target + REST_DISPLACEMENT * 0.5, velocity: REST_VELOCITY * 0.5 },
            target
        ));
        // At the target but still moving fast: about to overshoot.
        assert!(!BOUNCY.is_settled(MotionState { value: target, velocity: 1.0 }, target));
        // Stopped but far away.
        assert!(!BOUNCY.is_settled(MotionState::at_rest(0.0), target));
    }

    #[test]
    fn a_broken_spring_snaps_to_the_target() {
        for broken in [
            spring(f32::NAN, 1.0),
            spring(0.0, 1.0),
            spring(-5.0, 1.0),
            spring(100.0, -0.1),
            spring(100.0, f32::INFINITY),
        ] {
            assert!(!broken.is_valid());
            let state = MotionState { value: 0.25, velocity: 3.0 };
            assert_eq!(broken.step(state, 1.0, FRAME), MotionState::at_rest(1.0), "{broken:?}");
        }
    }

    #[test]
    fn non_finite_input_never_poisons_the_state() {
        let state = MotionState { value: 0.25, velocity: 1.0 };
        assert_eq!(BOUNCY.step(state, 1.0, f32::NAN), state);
        assert_eq!(BOUNCY.step(state, 1.0, f32::INFINITY), MotionState::at_rest(1.0));
        assert_eq!(BOUNCY.step(state, 1.0, -1.0), state);
        let poisoned = MotionState { value: f32::NAN, velocity: 0.0 };
        assert_eq!(BOUNCY.step(poisoned, 1.0, FRAME), MotionState::at_rest(1.0));
        assert_eq!(BOUNCY.step(state, f32::NAN, FRAME), state);
    }
}
