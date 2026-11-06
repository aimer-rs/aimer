#![doc = include_str!("../README.md")]

mod affine;
mod animate;
mod ease;
mod error;
mod geometry;
// Nothing but the parser consumes the reader.
mod json;
mod matte;
mod layers;
mod model;
mod parse;
mod playback;
mod scene;
mod property;
mod shapes;
mod trim;
mod widget;

#[cfg(test)]
mod ease_tests;
#[cfg(test)]
mod eval_tests;
#[cfg(test)]
mod eval_profile;
#[cfg(test)]
mod geometry_tests;
#[cfg(test)]
mod json_tests;
#[cfg(test)]
mod matte_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod scene_tests;
#[cfg(test)]
mod trim_tests;
#[cfg(test)]
mod widget_tests;
#[cfg(test)]
mod parse_tests;

pub use affine::Affine;
pub use animate::Lerp;
pub use error::LottieError;
pub use layers::{LayerState, LocalTransform};
pub use playback::{Playback, Tick};
pub use scene::{Block, LottieFrame, LottieScene};
pub use widget::{Lottie, LottieFit, RequiredAnimation};
pub use model::{
    Bezier, Composition, Diagnostic, Ellipse, Fill, FillRule, Group, Interpolation, Keyframe,
    Layer, LayerKind, LineCap, LineJoin, MatteMode, PathShape, Position, Property, Rect, Rgba, Shape, Stroke,
    Transform, Trim, Vec2,
};
