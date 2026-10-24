pub mod animatable;
pub mod curve;
pub mod motion;
pub mod time;
pub mod tween;

pub use animatable::Animatable;
pub use curve::{AnyCurve, Curve, CurveDescriptor, curves};
pub use motion::{AnyMotion, Motion, MotionDescriptor, MotionState, Ramp, Spring, motions};
pub use time::AnimInstant;
pub use tween::{AnimatableExt, Tween};
