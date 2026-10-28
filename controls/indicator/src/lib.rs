//! A sliding selection indicator.
//!
//! An indicator marks which of several slots is selected and slides between
//! them when the selection changes, the way the highlight in an iOS tab bar
//! does. Three small traits decide how, each following the shape of
//! [`aimer_animation::Curve`]: a trait, constants for the standard
//! implementations, an `Any*` handle that holders store, and `impl Trait` at the
//! API.
//!
//! - [`Motion`](aimer_animation::Motion) decides how the indicator travels: a
//!   spring, say, that overshoots a little and keeps its speed if retargeted.
//! - [`IndicatorStyle`] decides how it looks at rest and while *lifted*, and
//!   [`indicator_styles::LIQUID_GLASS`] is the lens that magnifies what it
//!   passes over.
//! - [`IndicatorTrack`] decides where the slots are.
//!
//! [`SlidingIndicator`] wires them to some content, and [`TabBar`] is the
//! ready-made bar built on it. Both can be dragged: the indicator follows the
//! pointer, held lifted, for as long as it is down, and lands on the nearest
//! slot when let go.
//!
//! Portable lowering is intentionally unsupported. These native widgets use the
//! default [`aimer_widget::PortableWidget`] diagnostic until a schema is added.

mod controller;
mod layer;
mod sliding_indicator;
mod style;
mod surface;
mod tab_bar;
#[cfg(test)]
mod test_support;
mod track;

pub use controller::{IndicatorController, IndicatorSample};
pub use style::{
    AnyIndicatorStyle, FlatPill, IndicatorLayer, IndicatorStyle, LayerPresentation, LiquidGlass,
    indicator_styles,
};
pub use sliding_indicator::SlidingIndicator;
pub use tab_bar::{Orientation, TabBar, TabBarState, TabItem};
pub use track::{AnyIndicatorTrack, IndicatorTrack, Linear, tracks};
