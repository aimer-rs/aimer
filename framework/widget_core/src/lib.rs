#![doc = include_str!("../README.md")]
extern crate self as aimer_widget;

pub use aimer_element::*;

pub mod diagnostics;
pub mod repaint_boundary;
#[doc(hidden)]
pub mod portable;
pub mod reconcile;
mod semantic_annotation;
#[doc(hidden)]
pub mod widget;

/// An Empty Widget that using as placeholder
///
/// ## Example
///
/// ```rust ignore
/// pub struct MyWidget<W = EmptyWidget> {
///     child: W,
///     // any fields here
/// }
/// ```
pub struct RequiredChild;

pub use crate::diagnostics::{
    ErrorElement, ErrorWidget, OverflowEdges, OverflowIndicator, detect_overflow,
};
pub use crate::repaint_boundary::RepaintBoundary;
pub use crate::semantic_annotation::Semantics;
pub use crate::widget::child_builder::ChildBuilder;
pub use crate::widget::retain::BuildContextRetain;
#[doc(hidden)]
pub use crate::widget::stateful::{
    State, StateReadGuard, StateUpdater, StatefulElement, StatefulWidget,
};
pub use crate::widget::stateless::{NamedWidget, StatelessElement, StatelessWidget};
pub use crate::widget::{AnyWidget, DynWidget, PortableWidget, Widget};
pub use aimer_macro::{PortableValue, PortableWidget, main, widget};
