//! The handle holders store.

use std::fmt;
use std::sync::Arc;

use super::ramp::{RAMP_TAG, Ramp};
use super::spring::{SPRING_TAG, Spring};
use super::{Motion, MotionDescriptor, MotionState};

/// A motion of any kind, cheap to clone and compare.
///
/// This is what the framework stores wherever it holds a motion: widgets and
/// animation state accept `impl Motion` and keep an `AnyMotion`. A spring is held
/// inline, so storing one allocates nothing and evaluating one takes no virtual
/// call; only a custom motion is boxed. `AnyMotion` is itself a [`Motion`], so it
/// can be passed back to anything that takes one.
///
/// Two handles are equal when they are springs with the same parameters or
/// clones of the same custom motion.
#[derive(Clone)]
pub struct AnyMotion(Repr);

#[derive(Clone)]
enum Repr {
    Spring(Spring),
    Ramp(Ramp),
    Custom(Arc<dyn Motion>),
}

impl AnyMotion {
    /// Wraps `motion`. Equivalent to [`Motion::into_any`].
    pub fn new(motion: impl Motion) -> Self {
        motion.into_any()
    }

    /// Rebuilds a standard motion from its [`MotionDescriptor`] parts, as stored
    /// in a portable widget document.
    ///
    /// Returns `None` if `tag` is unknown or the parameters are not usable.
    pub fn from_descriptor(tag: i64, params: [f64; 2]) -> Option<Self> {
        match tag {
            SPRING_TAG => Spring::from_params(params).map(Self::spring),
            RAMP_TAG => Ramp::from_params(params).map(Self::ramp),
            _ => None,
        }
    }

    pub(super) fn spring(spring: Spring) -> Self {
        Self(Repr::Spring(spring))
    }

    pub(super) fn ramp(ramp: Ramp) -> Self {
        Self(Repr::Ramp(ramp))
    }

    pub(super) fn custom(motion: impl Motion) -> Self {
        Self(Repr::Custom(Arc::new(motion)))
    }

    /// Whether the motion is held without an allocation.
    #[cfg(test)]
    fn is_inline(&self) -> bool {
        !matches!(self.0, Repr::Custom(_))
    }
}

impl Default for AnyMotion {
    /// The critically damped [`SMOOTH`](super::motions::SMOOTH) spring.
    fn default() -> Self {
        Self::spring(super::motions::SMOOTH)
    }
}

impl Motion for AnyMotion {
    fn step(&self, state: MotionState, target: f32, dt: f32) -> MotionState {
        match &self.0 {
            Repr::Spring(spring) => spring.step(state, target, dt),
            Repr::Ramp(ramp) => ramp.step(state, target, dt),
            Repr::Custom(motion) => {
                if !motion.is_valid() || !target.is_finite() {
                    return MotionState::at_rest(if target.is_finite() { target } else { state.value });
                }
                motion.step(state, target, dt)
            }
        }
    }

    fn is_settled(&self, state: MotionState, target: f32) -> bool {
        match &self.0 {
            Repr::Spring(spring) => spring.is_settled(state, target),
            Repr::Ramp(ramp) => ramp.is_settled(state, target),
            Repr::Custom(motion) => motion.is_settled(state, target),
        }
    }

    fn descriptor(&self) -> Option<MotionDescriptor> {
        match &self.0 {
            Repr::Spring(spring) => spring.descriptor(),
            Repr::Ramp(ramp) => ramp.descriptor(),
            Repr::Custom(motion) => motion.descriptor(),
        }
    }

    fn is_valid(&self) -> bool {
        match &self.0 {
            Repr::Spring(spring) => spring.is_valid(),
            Repr::Ramp(ramp) => ramp.is_valid(),
            Repr::Custom(motion) => motion.is_valid(),
        }
    }

    fn into_any(self) -> AnyMotion {
        self
    }
}

impl PartialEq for AnyMotion {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Spring(a), Repr::Spring(b)) => a == b,
            (Repr::Ramp(a), Repr::Ramp(b)) => a == b,
            (Repr::Custom(a), Repr::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for AnyMotion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Spring(spring) => write!(f, "{spring:?}"),
            Repr::Ramp(ramp) => write!(f, "{ramp:?}"),
            Repr::Custom(_) => f.write_str("Custom"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::motion::motions;

    /// Reaches the target after exactly one step.
    struct Jump;

    impl Motion for Jump {
        fn step(&self, _state: MotionState, target: f32, _dt: f32) -> MotionState {
            MotionState::at_rest(target)
        }
    }

    struct Broken;

    impl Motion for Broken {
        fn step(&self, state: MotionState, _target: f32, _dt: f32) -> MotionState {
            state
        }

        fn is_valid(&self) -> bool {
            false
        }
    }

    #[test]
    fn springs_are_held_inline_and_custom_motions_are_boxed() {
        assert!(motions::BOUNCY.into_any().is_inline());
        assert!(AnyMotion::default().is_inline());
        assert!(!Jump.into_any().is_inline());
    }

    #[test]
    fn handles_compare_by_value_for_springs_and_by_identity_for_custom_motions() {
        assert_eq!(motions::BOUNCY.into_any(), motions::BOUNCY.into_any());
        assert_ne!(motions::BOUNCY.into_any(), motions::SMOOTH.into_any());
        let custom = Jump.into_any();
        assert_eq!(custom, custom.clone());
        assert_ne!(custom, Jump.into_any());
        assert_ne!(custom, motions::SMOOTH.into_any());
    }

    #[test]
    fn a_handle_steps_exactly_like_the_motion_it_wraps() {
        let state = MotionState { value: 0.1, velocity: 1.0 };
        assert_eq!(
            motions::BOUNCY.into_any().step(state, 1.0, 0.01),
            motions::BOUNCY.step(state, 1.0, 0.01)
        );
        assert_eq!(Jump.into_any().step(state, 4.0, 0.01), MotionState::at_rest(4.0));
    }

    #[test]
    fn descriptors_round_trip_for_springs_and_are_absent_for_custom_motions() {
        let any = motions::spring(123.0, 0.5).into_any();
        let descriptor = any.descriptor().expect("a spring is describable");
        assert_eq!(AnyMotion::from_descriptor(descriptor.tag, descriptor.params), Some(any));
        assert_eq!(Jump.into_any().descriptor(), None);
    }

    #[test]
    fn a_ramp_is_held_inline_compares_by_value_and_round_trips() {
        use std::time::Duration;
        let fade = motions::ramp(Duration::from_millis(120)).into_any();
        assert!(fade.is_inline());
        assert_eq!(fade, motions::ramp(Duration::from_millis(120)).into_any());
        assert_ne!(fade, motions::ramp(Duration::from_millis(121)).into_any());
        assert_ne!(fade, motions::SMOOTH.into_any());
        let descriptor = fade.descriptor().expect("a ramp is describable");
        assert_eq!(AnyMotion::from_descriptor(descriptor.tag, descriptor.params), Some(fade.clone()));
        let state = MotionState::at_rest(0.0);
        assert_eq!(
            fade.step(state, 1.0, 0.03),
            motions::ramp(Duration::from_millis(120)).step(state, 1.0, 0.03)
        );
    }

    #[test]
    fn unknown_or_unusable_descriptors_are_refused() {
        assert_eq!(AnyMotion::from_descriptor(99, [100.0, 1.0]), None);
        assert_eq!(AnyMotion::from_descriptor(0, [f64::NAN, 1.0]), None);
        assert_eq!(AnyMotion::from_descriptor(0, [-1.0, 1.0]), None);
    }

    #[test]
    fn a_broken_custom_motion_snaps_instead_of_sticking() {
        let state = MotionState { value: 0.0, velocity: 5.0 };
        let any = Broken.into_any();
        assert!(!any.is_valid());
        assert_eq!(any.step(state, 2.0, 0.01), MotionState::at_rest(2.0));
    }

    #[test]
    fn a_non_finite_target_leaves_a_custom_motion_where_it_was() {
        let state = MotionState { value: 0.5, velocity: 5.0 };
        assert_eq!(Jump.into_any().step(state, f32::NAN, 0.01), MotionState::at_rest(0.5));
    }
}
