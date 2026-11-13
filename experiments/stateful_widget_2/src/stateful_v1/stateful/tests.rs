//! Tests that need `stateful_v1`'s own internals: element fields, the keyed
//! registry, the mutation queue, diagnostics, and the concrete `StateUpdater`
//! API. Behaviour every backend shares lives in `crate::consistency` instead;
//! these reuse its fixtures through the `V1` backend.

use std::panic::AssertUnwindSafe;

use super::*;
use crate::consistency::{
    Backend, Lifecycle, PanicPhase, build_context, has_error_child, lifecycle, traversal_element,
    LeafProbe,
};
use crate::stateful_v1::backend::{AdapterState, V1};

type LifecycleState = AdapterState<Lifecycle>;

fn lifecycle_element(
    phase: PanicPhase,
    name: &'static str,
    key: Option<aimer_widget_core::Key>,
    context: &BuildContext,
) -> AnyElement {
    V1::element::<Lifecycle>(lifecycle(phase), context, name, key)
}

fn as_stateful(element: &AnyElement) -> &StatefulElement {
    element
        .option_any()
        .and_then(|value| value.downcast_ref::<StatefulElement>())
        .expect("element should be stateful")
}

/// Takes the live state out of `element`, the way a failed adoption leaves it.
fn consume_state(element: &StatefulElement) {
    let state = unsafe { &*element.state_any.0.get() }
        .downcast_ref::<SyncState<LifecycleState>>()
        .expect("state should have the expected type");
    unsafe {
        (&mut *state.0.get()).take();
    }
}

#[test]
fn public_updater_is_copy_without_a_copy_state_bound() {
    struct NonCopyState(#[allow(dead_code)] String);
    fn assert_copy<T: Copy>() {}
    assert_copy::<StateUpdater<NonCopyState>>();
}

// -- recovery internals -------------------------------------------------------

#[test]
fn stale_rebuild_after_queued_panic_reuses_the_recovery_element() {
    let context = build_context();
    let widget = lifecycle(PanicPhase::None);
    let probe = widget.probe.clone();
    let element = V1::element::<Lifecycle>(widget, &context, "LifecycleWidget", None);
    let stateful = as_stateful(&element);
    probe.handle().set_state(|_| panic!("queued mutation failed"));

    let rebuild = unsafe { (&*stateful.rebuild_fn.0.get()).clone() };
    let first = rebuild(&context);
    assert_eq!(first.debug_name(), "ErrorWidget");

    let second = rebuild(&context);
    assert_eq!(
        second.debug_name(),
        "ErrorWidget",
        "stale callbacks must not rebuild the failed state again"
    );
}

#[test]
fn failed_keyed_reconciliation_does_not_retry_the_live_state() {
    let context = build_context();
    let _scope = KeyedStateScope::enter();
    let key = aimer_widget_core::Key::Value("consumed-live-state-key".to_owned());
    let live = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key.clone()), &context);
    let live_stateful = as_stateful(&live);
    let updater = live_stateful
        .state_updater::<LifecycleState>()
        .expect("live element should expose its state updater");
    consume_state(live_stateful);

    let recovered = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key), &context);

    assert_eq!(recovered.debug_name(), "ErrorWidget");
    updater.set_state(|_| {});
    live_stateful.rebuild_if_dirty(&context);
    assert!(has_error_child(live.as_ref()));
}

#[test]
fn carried_config_panic_becomes_error_child_with_key_hint() {
    let context = build_context();
    let key = aimer_widget_core::Key::Value("carry-panic-key".to_owned());
    let old = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key.clone()), &context);
    consume_state(as_stateful(&old));

    let new = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key), &context);

    carry_stateful(old.as_ref(), new.as_ref(), &context);

    assert!(has_error_child(new.as_ref()));
}

#[test]
fn failed_state_reconciliation_does_not_report_a_second_internal_panic() {
    let context = build_context();
    let key = aimer_widget_core::Key::Value("carry-panic-key".to_owned());
    let old = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key.clone()), &context);
    let old_stateful = as_stateful(&old);
    let updater = old_stateful
        .state_updater::<LifecycleState>()
        .expect("old element should expose its state updater");
    updater.set_state(|_| {});
    consume_state(old_stateful);

    let new = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key), &context);

    carry_stateful(old.as_ref(), new.as_ref(), &context);

    assert!(has_error_child(new.as_ref()));
    updater.set_state(|_| {});
    old_stateful.rebuild_if_dirty(&context);
    assert!(has_error_child(old.as_ref()));
}

#[test]
fn stale_keyed_rebuild_reuses_the_original_recovery_diagnostic() {
    let context = build_context();
    let _scope = KeyedStateScope::enter();
    let key = aimer_widget_core::Key::Value("stale-rebuild-key".to_owned());
    let live = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key.clone()), &context);
    let live_stateful = as_stateful(&live);
    consume_state(live_stateful);

    let recovered = lifecycle_element(PanicPhase::None, "LifecycleWidget", Some(key), &context);
    assert_eq!(recovered.debug_name(), "ErrorWidget");

    let rebuild = unsafe { (&*live_stateful.rebuild_fn.0.get()).clone() };
    let stale_child = rebuild(&context);
    let stale_debug = stale_child
        .option_any()
        .and_then(|value| value.downcast_ref::<aimer_widget_core::ErrorElement>())
        .map(|error| format!("{error:?}"))
        .expect("stale rebuild should return an error element");

    assert!(stale_debug.contains("adopt_config_from"), "{stale_debug}");
    assert!(!stale_debug.contains("queued state mutation"), "{stale_debug}");
    assert!(stale_debug.contains("stale-rebuild-key"), "{stale_debug}");
}

#[test]
fn consumed_keyed_state_panic_explains_where_to_check_the_key() {
    let key = aimer_widget_core::Key::Value("duplicate-key".to_owned());

    let message = consumed_state_panic_message("LifecycleWidget", Some(&key));

    assert!(message.contains("LifecycleWidget"));
    assert!(message.contains("duplicate-key"));
    assert!(message.contains("value Value(\"duplicate-key\")"));
    assert!(!message.contains("create_location"));
    assert!(!message.contains(file!()));
    assert!(message.contains("key is created and used"));
    assert!(message.contains("stable and unique"));
}

#[test]
fn consumed_keyed_state_diagnostic_points_to_key_creation() {
    let key = aimer_widget_core::Key::Value("creation-site-key".to_owned());
    let diagnostic = recover_operation("LifecycleWidget", BuildPhase::AdoptConfig, || {
        panic!("{}", consumed_state_panic_message("LifecycleWidget", Some(&key)))
    })
    .expect_err("the consumed-state diagnostic should be recovered");

    let message = diagnostic.with_site(key.diagnostic_site()).to_string();

    assert!(message.contains("at "), "{message}");
    assert!(message.contains(file!()), "{message}");
    assert!(message.contains("let key = aimer_widget_core::Key::Value"), "{message}");
    assert!(message.contains("^"), "{message}");
    assert!(!message.contains("create_location"), "{message}");
}

// -- keyed subtree registration ------------------------------------------------

#[test]
fn keyed_subtree_registration_visits_depth_first_in_child_order() {
    let visits = Rc::new(RefCell::new(Vec::new()));
    let first = traversal_element(
        1,
        vec![traversal_element(3, vec![], &visits), traversal_element(4, vec![], &visits)],
        &visits,
    );
    let second = traversal_element(2, vec![traversal_element(5, vec![], &visits)], &visits);
    let root = traversal_element(0, vec![first, second], &visits);

    register_keyed_subtree(root.as_ref());

    assert_eq!(*visits.borrow(), vec![0, 1, 3, 4, 2, 5]);
}

#[test]
fn nested_rebuilds_share_one_keyed_subtree_scan() {
    let context = build_context();
    let visits = Rc::new(RefCell::new(Vec::new()));
    let nested = lifecycle_element(PanicPhase::None, "NestedLifecycleWidget", None, &context);
    unsafe {
        *as_stateful(&nested).child.0.get() = traversal_element(2, vec![], &visits);
    }

    let outer = lifecycle_element(PanicPhase::None, "OuterLifecycleWidget", None, &context);
    let outer_stateful = as_stateful(&outer);
    unsafe {
        *outer_stateful.child.0.get() = traversal_element(1, vec![nested], &visits);
    }
    outer_stateful.dirty.borrow().set(true);

    outer_stateful.rebuild_if_dirty(&context);

    assert_eq!(*visits.borrow(), vec![1, 2, 1]);
}

#[test]
fn clean_stateful_rebuild_skips_keyed_subtree_registration() {
    let context = build_context();
    let visits = Rc::new(RefCell::new(Vec::new()));
    let outer = lifecycle_element(PanicPhase::None, "OuterLifecycleWidget", None, &context);
    let outer_stateful = as_stateful(&outer);
    unsafe {
        *outer_stateful.child.0.get() = traversal_element(1, vec![], &visits);
    }

    outer_stateful.rebuild_if_dirty(&context);

    assert!(visits.borrow().is_empty());
}

#[test]
fn local_mutation_queue_preserves_order_and_accepts_reentrant_updates() {
    let queue: Rc<StateMutationQueue<Vec<i32>>> = Rc::new(StateMutationQueue::default());
    let nested_queue = queue.clone();
    let mut state = Vec::new();

    queue.push(Box::new(move |state| {
        state.push(1);
        nested_queue.push(Box::new(|state| state.push(3)));
    }));
    queue.push(Box::new(|state| state.push(2)));

    assert_eq!(queue.drain_into(&mut state, || {}), 3);
    assert_eq!(state, vec![1, 2, 3]);
    assert_eq!(queue.drain_into(&mut state, || {}), 0);
}

// -- the concrete StateUpdater API --------------------------------------------

#[test]
fn test_state_updater_empty_panic() {
    let updater: StateUpdater<i32> = StateUpdater::empty();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        updater.set_state(|_s| {});
    }));
    assert!(result.is_err());

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        updater.read(|_s| {});
    }));
    assert!(result.is_err());
}

#[test]
fn try_operations_on_an_empty_updater_are_quiet_noops() {
    let updater: StateUpdater<i32> = StateUpdater::empty();
    let callback_called = Rc::new(Cell::new(false));
    let callback_called_by_read = callback_called.clone();
    assert_eq!(
        updater.try_read(move |_| {
            callback_called_by_read.set(true);
            1
        }),
        None,
    );
    let callback_called_by_mutation = callback_called.clone();
    assert_eq!(
        updater.try_set_state(move |_| callback_called_by_mutation.set(true)),
        None,
    );
    assert!(updater.try_read_state().is_none());
    assert!(!callback_called.get());
}

struct StaleUpdaterWidget;

struct StaleUpdaterState {
    value: String,
}

impl StatefulWidget for StaleUpdaterWidget {
    type State = StaleUpdaterState;

    fn create_state(self) -> Self::State {
        Self::State { value: String::from("live") }
    }
}

impl State<StaleUpdaterWidget> for StaleUpdaterState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        LeafProbe
    }
}

#[test]
fn try_operations_reject_a_copied_updater_after_unmount() {
    let context = build_context();
    let (element, updater) =
        StatefulElement::new_with_name(StaleUpdaterWidget, &context, "StaleUpdaterWidget", None);
    let copied = updater;

    assert_eq!(copied.try_read(|state| state.value.clone()), Some(String::from("live")));
    assert_eq!(
        copied.try_set_state(|state| state.value.push_str("-updated")),
        Some(()),
    );
    assert_eq!(
        copied.try_set_state(|state| state.value.push_str("-again")),
        Some(()),
    );
    element.rebuild_if_dirty(&context);
    assert_eq!(
        updater.try_read(|state| state.value.clone()),
        Some(String::from("live-updated-again"))
    );

    let guard = copied.try_read_state().expect("live state should be readable");

    drop(element);

    assert_eq!(guard.value, "live-updated-again");
    drop(guard);

    let callback_called = Rc::new(Cell::new(false));
    let callback_called_by_mutation = callback_called.clone();
    assert_eq!(
        copied.try_set_state(move |_| callback_called_by_mutation.set(true)),
        None,
    );
    assert!(!callback_called.get());
    assert_eq!(copied.try_read(|_| String::from("unexpected")), None);
    assert!(copied.try_read_state().is_none());
}

#[test]
fn delayed_callback_rejects_all_state_operations_after_native_unmount() {
    let context = build_context();
    let (element, updater) =
        StatefulElement::new_with_name(StaleUpdaterWidget, &context, "StaleUpdaterWidget", None);
    let read_called = Rc::new(Cell::new(false));
    let mutation_called = Rc::new(Cell::new(false));
    let delayed: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));
    let delayed_callback = delayed.clone();
    let read_called_by_callback = read_called.clone();
    let mutation_called_by_callback = mutation_called.clone();
    delayed_callback.borrow_mut().replace(Box::new(move || {
        assert!(!updater.has_state());
        assert_eq!(
            updater.try_read(move |_| {
                read_called_by_callback.set(true);
            }),
            None,
        );
        assert_eq!(
            updater.try_set_state(move |_| mutation_called_by_callback.set(true)),
            None,
        );
        assert!(updater.try_read_state().is_none());
    }));

    drop(element);
    delayed
        .borrow_mut()
        .take()
        .expect("delayed callback should be retained")();

    assert!(!read_called.get());
    assert!(!mutation_called.get());
}

#[test]
fn has_state_and_unchecked_updates_require_a_live_updater() {
    let context = build_context();
    let (element, updater) =
        StatefulElement::new_with_name(StaleUpdaterWidget, &context, "StaleUpdaterWidget", None);
    assert!(updater.has_state());

    unsafe {
        updater.set_state_unchecked(|state| state.value.push_str("-unchecked"));
    }
    element.rebuild_if_dirty(&context);
    assert_eq!(
        updater.try_read(|state| state.value.clone()),
        Some(String::from("live-unchecked"))
    );
    let unchecked_guard = unsafe { updater.read_state_unchecked() };
    assert_eq!(unchecked_guard.value, "live-unchecked");

    drop(element);
    assert!(!updater.has_state());
    assert_eq!(unchecked_guard.value, "live-unchecked");
    drop(unchecked_guard);
}

struct OwnedConfigWidget;

struct OwnedConfigState {
    config: String,
}

impl StatefulWidget for OwnedConfigWidget {
    type State = OwnedConfigState;

    fn create_state(self) -> Self::State {
        OwnedConfigState { config: String::from("new") }
    }
}

impl State<OwnedConfigWidget> for OwnedConfigState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.config = new.config;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        LeafProbe
    }
}

#[test]
fn adopt_config_from_consumes_the_new_state() {
    let mut state = OwnedConfigState { config: String::from("old") };

    state.adopt_config_from(OwnedConfigState { config: String::from("new") });

    assert_eq!(state.config, "new");
}

// -- cost of a nested chain -----------------------------------------------------

/// Counts for one chain of `depth` levels: first build, then a fresh chain carried
/// over the old one (what a parent re-emitting the widget does), then a
/// `set_state` on the outermost level followed by the ordinary rebuild pass.
fn chain_costs(depth: u32) -> [(u32, u32, u32); 3] {
    use crate::consistency::{Chain, ChainCounts, ChainProps};

    let context = build_context();
    let counts = Rc::new(ChainCounts::default());
    let chain = || {
        V1::element::<Chain>(
            ChainProps { depth: depth as usize, counts: counts.clone() },
            &context,
            "Chain",
            None,
        )
    };
    let old = chain();
    let first = counts.take();

    let new = chain();
    counts.take();
    carry_stateful(old.as_ref(), new.as_ref(), &context);
    let re_emitted = counts.take();

    counts.handle(depth as usize).set_state(|_| {});
    old.rebuild_if_dirty(&context);
    let set_state = counts.take();
    [first, re_emitted, set_state]
}

/// Measured, not derived: every nested stateful level doubles the work of a
/// rebuild that reaches it. Both the carry of a re-emitted chain and a `set_state`
/// on the outermost level (the ordinary rebuild pass) cost `2^(d-1) - 1`
/// throwaway creates and inits and `2^d - 1` builds, for `d` levels that need `d`
/// builds. This pins today's numbers so a change to the reconciler or to how a
/// stateful element is constructed shows up here.
///
/// Mechanism (a hypothesis from a trace of which pairs get adopted, not proven):
/// an element is built with a child eagerly, so adopting a state pairs the freshly
/// materialized child with that throwaway child as well as with the live one.
#[test]
fn nested_chain_rebuild_cost_doubles_with_each_level() {
    for depth in 1..=6u32 {
        let [first, re_emitted, set_state] = chain_costs(depth);
        let throwaway = (1 << (depth - 1)) - 1;
        let builds = (1 << depth) - 1;
        assert_eq!(first, (depth, depth, depth), "depth {depth}: first build");
        assert_eq!(re_emitted, (throwaway, throwaway, builds), "depth {depth}: re-emitted chain");
        assert_eq!(set_state, (throwaway, throwaway, builds), "depth {depth}: set_state on the outermost level");
    }
}

/// Time of one outermost `set_state` + rebuild as the chain gets deeper. Debug
/// numbers are the ones that matter here, so run it unoptimised and alone:
///
/// ```text
/// cargo nextest run -p stateful_widget_2 chain_cost_profile --run-ignored only --no-capture
/// ```
#[test]
#[ignore = "profiling harness; run explicitly with --run-ignored only"]
fn chain_cost_profile() {
    use crate::consistency::{Chain, ChainCounts, ChainProps};
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
        let element = V1::element::<Chain>(
            ChainProps { depth, counts: counts.clone() },
            &context,
            "Chain",
            None,
        );
        let outer = counts.handle(depth);
        let iterations = (40_000usize >> depth).clamp(20, 2_000);
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
        let _ = outer;
        let time = median(samples);
        println!(
            "{depth:>6} {time:>12.1?} {:>9} {:>9} {:>14.0}",
            last.2,
            last.0,
            time.as_nanos() as f64 / last.2.max(1) as f64
        );
    }
}
