//! Experiment: a `State` that does not hold its own `StateUpdater`, and a
//! `StatefulWidget` that does not leak the visibility of its `State`.
//!
//! - `mock`     std-only mock of the framework pieces.
//! - `design_a` `fn build(this: StateUpdater<Self>, ctx)`, state read through the updater.
//!
//! - `consistency` backend-independent test suite for stateful implementations built on the
//!   real element API; a backend implements `Backend` and invokes `consistency_suite!`.
//! - `framework_backend` the framework's own (new-shape) stateful implementation as a `Backend`.
//! - `stateful_v1` verbatim copy of the OLD-shape stateful logic (before the migration) (`StatefulElement`, state slots,
//!   panic recovery) for experimenting against the real element machinery.
//!
//! The design shares the visibility fix: the widget links to its state inside
//! `to_element`'s body, so there is no `type State` for a private state to leak through.

mod consistency;
mod framework_backend;
mod mock;
mod stateful_v1;
mod design_a;

use mock::{BuildContext, Widget};

fn main() {
    let ctx = BuildContext;

    let mut a = design_a::Counter { start: 0, step: 5 }.to_element();
    let tree = a.render(&ctx);
    println!("A before: {}", tree.render());
    tree.press("+");
    println!("A after:  {}", a.render(&ctx).render());
}
