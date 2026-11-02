//! Controlled choice and selection controls.
//!
//! Every control is a widget that owns only transient interaction state —
//! hover, press, focus, an open popup. The value always stays with the caller:
//! activating a control calls its `on_change` with the proposed value, and the
//! owner rebuilds the control with the value it accepted.
//!
//! Each control also takes a `builder(|state| widget)` that replaces its default
//! look while keeping its pointer, keyboard, focus, and callback behavior.
//!
//! Semantic snapshots, available from each control's `semantics()`, are
//! independent of `aimer_accessibility`; that crate may adapt
//! [`SelectionSemantics`] when it is present.
//!
//! Portable lowering is intentionally unsupported. Native widgets use the
//! default [`aimer_widget::PortableWidget`] diagnostic until a schema is added.

mod autocomplete;
mod binary;
mod checkbox;
mod chrome;
mod combo;
mod common;
mod indicator;
mod interaction;
mod keys;
mod nav;
mod option;
mod popup;
mod radio;
mod radio_group;
mod select;
mod semantics;
mod switch;
mod theme;
mod visual;

pub use autocomplete::Autocomplete;
pub use checkbox::{Checkbox, CheckboxValue};
pub use option::{ChoiceOption, OptionError};
pub use radio::Radio;
pub use radio_group::RadioGroup;
pub use select::Select;
pub use semantics::{SelectionSemantics, SemanticRole};
pub use switch::Switch;
pub use visual::{
    CheckboxVisualState, OptionState, ToggleVisualState, TriggerState, VisualState,
};
