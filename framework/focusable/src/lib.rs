#![doc = include_str!("../README.md")]
// Derives generate paths under `aimer_widget`; name the crate that defines them.
extern crate aimer_widget_core as aimer_widget;

pub mod focus_scope;
pub mod focusable;

pub use focus_scope::FocusScope;
pub use focusable::{Focusable, FocusableState, RawFocusable};
