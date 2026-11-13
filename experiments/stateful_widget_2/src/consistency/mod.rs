//! Backend-independent consistency suite for stateful widgets.
//!
//! The behaviour every stateful implementation must share (deferred and
//! coalesced `set_state`, panic recovery, keyed and unkeyed state carry,
//! handles going stale on unmount) is written once, here, against two small
//! traits:
//!
//! - [`Spec`] describes a stateful widget in neutral terms: how its state is
//!   created, initialised, given new props, and built.
//! - [`Backend`] turns a `Spec` into a real element using one implementation of
//!   the stateful machinery.
//!
//! A backend plugs in by implementing `Backend` (see `stateful_v1/backend.rs`
//! for the one that adapts the current API) and invoking
//! [`consistency_suite!`], which stamps out one `#[test]` per case:
//!
//! ```text
//! consistency_suite!(V1);
//! ```
//!
//! Tests that need a backend's internals (its element fields, registries,
//! queues, diagnostics) do not belong here: they stay next to that backend,
//! and may still reuse the fixtures.
//!
//! The suite targets implementations built on the real `aimer_widget_core`
//! element API. The std-only mock behind `design_a` has its own
//! `Widget` type and is covered by its own tests.

pub mod cases;
mod fixtures;
mod handle;
mod suite;

pub use fixtures::*;
pub use handle::{Handle, Ops, Probe};

use std::any::Any;

use aimer_widget_core::base::*;
use aimer_widget_core::key::Key;
use aimer_widget_core::{AnyElement, Element, Widget};

/// A stateful widget described independently of any backend.
///
/// Props are passed by reference because a backend may need them again later
/// (to adopt them into a live state) after the first use.
pub trait Spec: 'static {
    /// The widget configuration a parent passes down.
    type Props: 'static;
    /// What must survive a rebuild.
    type State: 'static;

    /// Builds the state from the props.
    fn create(props: &Self::Props) -> Self::State;

    /// Runs once, right after `create` and before the first build. Mutates the
    /// state directly; there is no updater yet.
    fn init(_state: &mut Self::State) {}

    /// A parent re-emitted the widget: take the new props into the live state.
    fn adopt(_state: &mut Self::State, _props: &Self::Props) {}

    /// Describes the UI. `this` reaches the state from callbacks; `B` is the
    /// backend, for fixtures that nest further stateful widgets.
    fn build<B: Backend>(
        state: &Self::State,
        this: Handle<Self::State>,
        ctx: &BuildContext,
    ) -> impl Widget;
}

/// One implementation of the stateful machinery, under test.
pub trait Backend: 'static {
    /// Shown in failure reports.
    const NAME: &'static str;

    /// Isolates the keyed-state registry for the lifetime of the returned
    /// guard, so keyed tests do not see each other's live states.
    ///
    /// Erased to `Box<dyn Any>`: a concrete guard type would have to be public.
    fn key_scope() -> Box<dyn Any>;

    /// Builds the element a stateful widget of `Sp` becomes, the way the
    /// widget's own `to_element` would.
    fn element<Sp: Spec>(
        props: Sp::Props,
        ctx: &BuildContext,
        name: &'static str,
        key: Option<Key>,
    ) -> AnyElement;
}

/// A headless build context.
pub fn build_context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        Default::default(),
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        #[cfg(not(target_arch = "wasm32"))]
        async_handle(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn async_handle() -> tokio::runtime::Handle {
    use std::sync::OnceLock;

    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime always builds")
    });
    let _guard = runtime.enter();
    tokio::runtime::Handle::current()
}

/// Whether one of `element`'s direct children is the error element a failed
/// stateful widget is replaced with.
pub fn has_error_child(element: &dyn Element) -> bool {
    let mut found = false;
    element.visit_children(&mut |child| found |= child.debug_name() == "ErrorWidget");
    found
}

/// Whether `element` has a state mutation waiting for the next rebuild.
pub fn is_dirty(element: &dyn Element) -> bool {
    use aimer_widget_core::components::rebuildable::SelfRebuilding;

    element.self_rebuilding().is_some_and(|element| element.is_dirty())
}

pub(crate) use suite::consistency_suite;

#[cfg(test)]
mod tests {
    /// `consistency_suite!` lists the cases by hand; fail if one is forgotten.
    #[test]
    fn the_suite_runs_every_case() {
        let cases = include_str!("cases.rs");
        let suite = include_str!("suite.rs");
        let mut checked = 0;
        for name in cases
            .lines()
            .filter_map(|line| line.strip_prefix("pub fn "))
            .filter_map(|rest| rest.split('<').next())
        {
            assert!(
                suite.lines().any(|line| line.trim() == format!("{name},")),
                "case `{name}` is not listed in suite.rs"
            );
            checked += 1;
        }
        assert!(checked > 0, "no cases found in cases.rs");
    }
}
