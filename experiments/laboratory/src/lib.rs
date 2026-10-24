//! A sealed workbench for Aimer's core ownership model.
//!
//! This crate exists to answer one question with running code instead of
//! reasoning: can a widget be *moved* into its element, so that building a
//! frame copies nothing and allocates nothing?
//!
//! The production trait used to borrow the widget — `fn to_element(&self, ..)`
//! — so every implementation cloned the fields it needed, even though the widget
//! is a temporary that dies on the next line. A consuming
//! `fn to_element(self, ..)` removes those clones, but it is not callable
//! through an erased handle, so the erased path has to move the payload out of
//! its owner. The prototype here does exactly that, on top of
//! [`aimer_rubick`]'s inline-or-pooled owner — and it is what
//! `aimer_widget` now ships: this crate stays as the isolated place to measure
//! the model and to try the next change to it before production sees it.
//!
//! # Isolation
//!
//! The laboratory *uses* its dependencies and *exports* none of them. No type
//! from `aimer_rubick` appears in a public signature: [`AnyWidget`] and
//! [`AnyElement`] are opaque newtypes. Nothing in the workspace depends on
//! this crate, so an experiment can be rewritten or deleted without a single
//! downstream edit.
//!
//! # What is modelled
//!
//! - [`Widget`] — a short-lived configuration value, consumed by
//!   [`Widget::to_element`].
//! - [`Element`] — the long-lived node that keeps the widget's data.
//! - [`AnyWidget`] — the erased widget handle, convertible with
//!   [`AnyWidget::into_element`], which moves the payload out of its storage
//!   and returns the storage to the pool.
//! - [`RetainedChild`] — a child a self-rebuilding parent can place into the
//!   tree again without rebuilding the subtree, which is what a consuming
//!   conversion needs in place of a second widget.
//! - [`experiment`] — the nodes the measurements are run against, and the
//!   measurements themselves.
//! - [`v2_render`] — a retained render tree with one local command list per
//!   element and damage-aware parent-first composition.
//!
//! # Example
//!
//! ```
//! use aimer_laboratory::{AnyElement, AnyWidget, Element, Widget};
//!
//! // A widget that is deliberately not `Clone`: nothing may copy it.
//! struct Label(String);
//!
//! struct LabelElement(String);
//!
//! impl Element for LabelElement {
//!     fn debug_name(&self) -> &'static str {
//!         "LabelElement"
//!     }
//! }
//!
//! impl Widget for Label {
//!     fn to_element(self) -> AnyElement {
//!         // The string is moved, not cloned: the widget is gone after this.
//!         AnyElement::new(LabelElement(self.0))
//!     }
//! }
//!
//! let widget = AnyWidget::new(Label(String::from("Aimer")));
//! let element = widget.into_element();
//!
//! assert_eq!(element.debug_name(), "LabelElement");
//! ```

mod element;
pub mod experiment;
pub mod new_event_tree;
pub mod v2_render;
mod widget;

pub use crate::element::{AnyElement, Element};
pub use crate::widget::Widget;
pub use crate::widget::erased::AnyWidget;
pub use crate::widget::retained::RetainedChild;
