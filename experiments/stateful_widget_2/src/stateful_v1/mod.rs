//! A copy of the framework's stateful logic, taken from
//! `framework/widget_core/src/widget/{stateful,recovery}.rs` and
//! `framework/element/src/state_slots.rs`, with `crate::` paths repointed at
//! `aimer_widget_core`. Edit freely: nothing outside this experiment uses it.

pub mod backend;
pub mod recovery;
pub mod state_slots;
pub mod stateful;
