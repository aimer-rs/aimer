//! Motions: how a scalar chases a target.
//!
//! A [`Curve`](crate::Curve) shapes progress over a fixed duration, which cannot
//! express a value that is *retargeted while moving*: the new journey has to
//! start with the speed the old one had, or the motion visibly jerks. A
//! [`Motion`] owns that case. It advances a [`MotionState`] (a value and its
//! velocity) toward a target, one frame at a time, and because the velocity is
//! part of the state it survives a change of target.
//!
//! The standard motions live in [`motions`] as constants
//! (`motions::BOUNCY`) and any type that implements the trait can be used
//! wherever the framework takes one:
//!
//! ```
//! use aimer_animation::{Motion, MotionState, motions};
//!
//! /// Jumps to the target at once.
//! struct Instant;
//!
//! impl Motion for Instant {
//!     fn step(&self, _state: MotionState, target: f32, _dt: f32) -> MotionState {
//!         MotionState::at_rest(target)
//!     }
//! }
//!
//! let state = MotionState::at_rest(0.0);
//! assert_eq!(Instant.step(state, 3.0, 0.016).value, 3.0);
//! assert!(motions::SMOOTH.step(state, 3.0, 0.016).value < 3.0);
//! ```
//!
//! Holders store motions as [`AnyMotion`], a cheap handle that is itself a
//! [`Motion`]: springs are kept inline without allocating, and only a custom
//! motion is boxed.

mod any;
mod ramp;
mod spring;

pub use any::AnyMotion;
pub use ramp::Ramp;
pub use spring::Spring;

/// The standard motions.
pub mod motions {
    pub use super::ramp::ramp;
    pub use super::spring::{BOUNCY, SMOOTH, SNAPPY, spring};
}

/// A value and the speed it is moving at, in value units per second.
///
/// The speed is what makes a retarget smooth: hand the state a motion returned
/// back to it with a new target and the value carries on from where it was, at
/// the speed it had.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MotionState {
    /// The current value.
    pub value: f32,
    /// The current rate of change of `value`, per second.
    pub velocity: f32,
}

impl MotionState {
    /// A value that is not moving.
    #[inline]
    pub const fn at_rest(value: f32) -> Self {
        Self { value, velocity: 0.0 }
    }

    /// Whether both fields are finite.
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.value.is_finite() && self.velocity.is_finite()
    }
}

/// How far from the target a motion may be, in value units, and still count as
/// having arrived.
pub const REST_DISPLACEMENT: f32 = 1.0e-3;
/// How fast a motion may still be moving, in value units per second, and still
/// count as having arrived.
pub const REST_VELOCITY: f32 = 1.0e-2;

/// A serializable description of a standard motion: a tag plus its parameters.
///
/// The tags are part of the portable widget format and never change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionDescriptor {
    /// Identifies the motion: `0` is a spring, `1` a ramp.
    pub tag: i64,
    /// The motion's parameters. A spring stores `(stiffness, damping_ratio)`; a
    /// ramp stores `(seconds, 0)`.
    pub params: [f64; 2],
}

/// Moves a scalar toward a target.
///
/// Implement [`step`](Self::step) for a custom motion. It must be *frame-rate
/// independent*: advancing by `dt` once and advancing by `dt / 2` twice have to
/// agree, since a window may present at 60 Hz or 120 Hz. A motion should also
/// settle: given a fixed target and enough time it reaches the target and
/// stops, which is what lets the framework stop asking for frames.
///
/// A motion must be `Send + Sync + 'static` so that holders can share it freely.
pub trait Motion: Send + Sync + 'static {
    /// Advances `state` by `dt` seconds toward `target`.
    ///
    /// The framework only hands over a finite, non-negative `dt`. The velocity
    /// in the returned state is what a later retarget continues from.
    fn step(&self, state: MotionState, target: f32, dt: f32) -> MotionState;

    /// Whether `state` has arrived at `target`, so that no further frame is
    /// needed.
    ///
    /// The default is within [`REST_DISPLACEMENT`] of the target and slower than
    /// [`REST_VELOCITY`].
    fn is_settled(&self, state: MotionState, target: f32) -> bool {
        (state.value - target).abs() <= REST_DISPLACEMENT
            && state.velocity.abs() <= REST_VELOCITY
    }

    /// Describes this motion for a portable guest, or `None` if it cannot be
    /// described.
    ///
    /// Only the standard motions can be serialized into a portable widget
    /// document. A custom motion keeps the default `None`: it works natively,
    /// and lowering it to a guest reports an error.
    fn descriptor(&self) -> Option<MotionDescriptor> {
        None
    }

    /// Whether the motion's parameters are usable, such as a finite, positive
    /// spring stiffness. A motion that is not valid snaps to its target instead
    /// of animating, so a broken parameter never leaves a control stuck.
    fn is_valid(&self) -> bool {
        true
    }

    /// Converts this motion into the handle holders store.
    ///
    /// The default boxes the motion. Springs override it to avoid the
    /// allocation.
    fn into_any(self) -> AnyMotion
    where
        Self: Sized,
    {
        AnyMotion::custom(self)
    }
}
