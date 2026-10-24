extern crate self as aimer_animation;

pub mod control;
#[path = "layout/transition.rs"]
pub mod layout;
mod local_cell;
pub mod primitives;
pub mod widgets;

// Core primitives
// Animation orchestration
pub use control::{
    AnimationController, AnimationStatus, Keyframe, KeyframeAnimation, KeyframeBuilder,
    KeyframeError, KeyframeTimeline, KeyframeTrack, ParallelAnimation, SequentialAnimation,
    StaggeredAnimation, StatusListener, TimedKeyframeAnimation, TimedKeyframeBuilder,
    TimelineFrame, TweenAnimation,
};
pub use primitives::{
    AnimInstant, Animatable, AnimatableExt, AnyCurve, AnyMotion, Curve, CurveDescriptor, Motion,
    MotionDescriptor, MotionState, Ramp, Spring, Tween, curves, motions,
};
// Widget layer
pub use widgets::AnimatedBuilder;
#[doc(hidden)]
pub use widgets::animated_builder::ensure_portable_native_registrations;
pub use widgets::{
    Animated, AnimatedPaint, AnimatedSwitcher, AnimationEffect, ContainerTransformFrame,
    ContainerTransformSpec, ElementBounds, ExpandSpec, FadeTransition, StageSpans,
    ImplicitAnimatedBuilder, MorphStyle, MorphTransition, PaintPresentation, PresentationClip,
    RotationTransition, ScaleTransition, SharedElement, SlideTransition, last_shared_element, last_shared_element_bounds,
    shared_element_bounds, SharedContent, SharedElementSnapshot,
};
