//! The framework's own stateful implementation (`aimer_widget_core`, the new
//! `State` / `StateView` shape) as a consistency-suite [`Backend`].
//!
//! The suite's neutral `Spec` maps onto the real `State` trait almost directly:
//! `create`, `init_state`, `adopt_config_from` and `build` have the same shape.
//! The only translation is `adopt_config_from(new: Self)`, which hands over a
//! whole new *state*, while a spec adopts *props*; the adapter keeps the props
//! beside the state for that reason.

use std::any::Any;

use aimer_widget_core::base::BuildContext;
use aimer_widget_core::key::Key;
use aimer_widget_core::{AnyElement, KeyedStateScope, State, StateUpdater, StateView, StatefulElement, Widget};

use crate::consistency::{Backend, Handle, Ops, Spec};

/// The backend: the framework's real `StatefulElement`.
pub struct Framework;

/// The widget half: just the props.
pub struct Adapter<Sp: Spec> {
    props: Sp::Props,
}

impl<Sp: Spec> Widget for Adapter<Sp> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget::<Self, AdapterState<Sp>>(self, ctx, "Adapter", None)
    }
}

/// The state half: the spec's state plus the props it was last given.
pub struct AdapterState<Sp: Spec> {
    state: Sp::State,
    props: Sp::Props,
}

impl<Sp: Spec> State<Adapter<Sp>> for AdapterState<Sp> {
    fn create(widget: Adapter<Sp>) -> Self {
        AdapterState { state: Sp::create(&widget.props), props: widget.props }
    }

    fn init_state(&mut self) {
        Sp::init(&mut self.state);
    }

    fn adopt_config_from(&mut self, new: Self) {
        Sp::adopt(&mut self.state, &new.props);
        self.props = new.props;
    }

    fn build(this: &StateView<Self>, ctx: &BuildContext) -> impl Widget {
        Sp::build::<Framework>(&this.state, Handle::new(FrameworkOps::<Sp>(this.updater())), ctx)
    }
}

/// The native updater, seen through the suite's [`Handle`].
struct FrameworkOps<Sp: Spec>(StateUpdater<AdapterState<Sp>>);

impl<Sp: Spec> Ops<Sp::State> for FrameworkOps<Sp> {
    fn set_state(&self, mutation: Box<dyn FnOnce(&mut Sp::State)>) {
        self.0.set_state(move |adapter| mutation(&mut adapter.state));
    }

    fn try_set_state(&self, mutation: Box<dyn FnOnce(&mut Sp::State)>) -> bool {
        self.0.try_set_state(move |adapter| mutation(&mut adapter.state)).is_some()
    }

    fn read(&self, read: &mut dyn FnMut(&Sp::State)) {
        self.0.read(|adapter| read(&adapter.state));
    }

    fn try_read(&self, read: &mut dyn FnMut(&Sp::State)) -> bool {
        self.0.try_read(|adapter| read(&adapter.state)).is_some()
    }

    fn has_state(&self) -> bool {
        self.0.has_state()
    }
}

impl Backend for Framework {
    const NAME: &'static str = "framework";

    fn key_scope() -> Box<dyn Any> {
        Box::new(KeyedStateScope::enter())
    }

    fn element<Sp: Spec>(
        props: Sp::Props,
        ctx: &BuildContext,
        name: &'static str,
        key: Option<Key>,
    ) -> AnyElement {
        StatefulElement::from_widget::<Adapter<Sp>, AdapterState<Sp>>(Adapter { props }, ctx, name, key)
    }
}

#[cfg(test)]
mod suite {
    crate::consistency::consistency_suite!(super::Framework);
}

#[cfg(test)]
mod chain_cost {
    use std::rc::Rc;

    use super::*;
    use crate::consistency::{Chain, ChainCounts, ChainProps, build_context};

    /// A rebuild of a nested chain builds each of its `d` levels once, and creates
    /// and initialises one state for each level below the one that rebuilt: linear in
    /// the depth. It was `d(d+1)/2` builds before the first build of an element
    /// waited for the carry, and `2^d - 1` before that. The copy of the old machinery
    /// in `stateful_v1` still pins the exponential numbers, for comparison.
    #[test]
    fn a_nested_chain_pays_linearly_in_its_depth_through_the_framework() {
        let context = build_context();
        for depth in 1..=8u32 {
            let counts = Rc::new(ChainCounts::default());
            let element = Framework::element::<Chain>(
                ChainProps { depth: depth as usize, counts: counts.clone() },
                &context,
                "Chain",
                None,
            );
            assert_eq!(counts.take(), (depth, depth, depth), "depth {depth}: first build");

            counts.handle(depth as usize).set_state(|_| {});
            element.rebuild_if_dirty(&context);

            assert_eq!(counts.take(), (depth - 1, depth - 1, depth), "depth {depth}: set_state on the outermost level");
        }
    }

    /// Time of one outermost `set_state` + rebuild as the chain gets deeper, on the
    /// framework. Debug numbers are the ones that matter here, so run it
    /// unoptimised and alone:
    ///
    /// ```text
    /// cargo nextest run -p stateful_widget_2 framework_chain_cost_profile --run-ignored only --no-capture
    /// ```
    #[test]
    #[ignore = "profiling harness; run explicitly with --run-ignored only"]
    fn framework_chain_cost_profile() {
        use aimer_time::{Duration, Instant};

        fn median(mut samples: Vec<Duration>) -> Duration {
            samples.sort();
            samples[samples.len() / 2]
        }

        println!("\nset_state on the outermost of D nested stateful levels, then rebuild (median)");
        println!("{:>6} {:>12} {:>9} {:>9} {:>14}", "D", "time", "builds", "creates", "ns per build");
        let context = build_context();
        for depth in [1usize, 2, 4, 6, 8, 10] {
            let counts = Rc::new(ChainCounts::default());
            let element = Framework::element::<Chain>(
                ChainProps { depth, counts: counts.clone() },
                &context,
                "Chain",
                None,
            );
            let iterations = (40_000usize >> depth).clamp(200, 4_000);
            let mut samples = Vec::with_capacity(iterations);
            let mut last = (0, 0, 0);
            for _ in 0..iterations {
                counts.take();
                let start = Instant::now();
                counts.handle(depth).set_state(|_| {});
                element.rebuild_if_dirty(&context);
                samples.push(start.elapsed());
                last = counts.take();
            }
            let time = median(samples);
            println!(
                "{depth:>6} {time:>12.1?} {:>9} {:>9} {:>14.0}",
                last.2,
                last.0,
                time.as_nanos() as f64 / last.2.max(1) as f64
            );
        }
    }
}
