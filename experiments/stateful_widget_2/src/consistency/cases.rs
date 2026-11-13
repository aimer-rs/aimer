//! The cases. Each is generic over the backend and asserts only behaviour that
//! is observable through the element tree and a [`Handle`].
//!
//! Adding a case: write `pub fn name<B: Backend>()` here, then list `name,` in
//! `suite.rs` (a test fails if you forget).

use std::cell::{Cell, RefCell};
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_widget_core::components::element::{carry_child_state, carry_stateful};
use aimer_widget_core::key::Key;
use aimer_widget_core::{EventDispatcher, EventTreeRole, StatelessElement};

use super::*;

fn phase_recovers<B: Backend>(phase: PanicPhase) {
    let context = build_context();
    let element = B::element::<Lifecycle>(lifecycle(phase), &context, "LifecycleWidget", None);
    assert_eq!(element.debug_name(), "ErrorWidget");
}

// -- building and rebuilding -------------------------------------------------

pub fn routed_event_reaches_stateful_child_once<B: Backend>() {
    let events = Rc::new(Cell::new(0));
    let element = B::element::<Event>(
        EventProps { events: events.clone() },
        &build_context(),
        "EventProbeWidget",
        None,
    );
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);

    let _ = EventDispatcher::new().dispatch(
        &element,
        Vec2d::default(),
        &ElementEvent::PointerMove(PointerInfo::mouse(Vec2d::default(), PointerButton::Primary)),
    );

    assert_eq!(events.get(), 1);
}

pub fn set_state_is_deferred_and_coalesced<B: Backend>() {
    let context = build_context();
    let builds = Rc::new(Cell::new(0));
    let probe = Probe::default();
    let element = B::element::<Tap>(
        TapProps { builds: builds.clone(), probe: probe.clone() },
        &context,
        "Tap",
        None,
    );
    let handle = probe.handle();
    assert_eq!(builds.get(), 1, "the first build runs when the element is made");

    element.rebuild_if_dirty(&context);
    assert_eq!(builds.get(), 1, "a clean pass builds nothing");

    handle.set_state(|state| state.taps += 1);
    assert_eq!(builds.get(), 1, "set_state must not build inline");
    assert!(is_dirty(element.as_ref()));

    element.rebuild_if_dirty(&context);
    assert_eq!(builds.get(), 2);
    assert!(!is_dirty(element.as_ref()));
    assert_eq!(handle.read(|state| state.taps), 1, "the mutation applies exactly once");

    for _ in 0..10 {
        handle.set_state(|state| state.taps += 1);
    }
    element.rebuild_if_dirty(&context);
    assert_eq!(builds.get(), 3, "ten set_state calls, one build");
    assert_eq!(handle.read(|state| state.taps), 11);
}

pub fn init_runs_once_before_the_first_build<B: Backend>() {
    let context = build_context();
    let log = Rc::new(RefCell::new(Vec::new()));
    let probe = Probe::default();
    let element = B::element::<Init>(
        InitProps { log: log.clone(), probe: probe.clone() },
        &context,
        "Init",
        None,
    );
    assert_eq!(*log.borrow(), ["create", "init", "build"]);

    probe.handle().set_state(|_| {});
    element.rebuild_if_dirty(&context);
    assert_eq!(*log.borrow(), ["create", "init", "build", "build"], "a rebuild does not re-run init");
}

pub fn state_updater_rebuilds_only_its_dirty_subtree<B: Backend>() {
    let context = build_context();
    let builds = Rc::new(Cell::new(0));
    let probe = Probe::default();
    let stateful = B::element::<Tap>(
        TapProps { builds: builds.clone(), probe: probe.clone() },
        &context,
        "Tap",
        None,
    );
    let clean_rebuilds = Rc::new(Cell::new(0));
    let visits = Rc::new(RefCell::new(Vec::new()));
    let root = TraversalElement {
        id: 0,
        children: vec![
            StatelessElement::wrapper(
                RebuildCountLeaf { rebuilds: clean_rebuilds.clone() }.boxed(),
                None,
                "CleanSibling",
            )
            .boxed(),
            stateful,
        ],
        visits,
    }
    .boxed();

    root.rebuild_if_dirty(&context);
    clean_rebuilds.set(0);
    builds.set(0);

    probe.handle().set_state(|_| {});
    root.rebuild_if_dirty(&context);

    assert_eq!(clean_rebuilds.get(), 0);
    assert_eq!(builds.get(), 1);
}

pub fn a_nested_stateful_child_keeps_its_state_when_the_parent_updates<B: Backend>() {
    let context = build_context();
    let nested_value = Rc::new(Cell::new(usize::MAX));
    let nested_probe = Probe::default();
    let parent_probe = Probe::default();
    let element = B::element::<NestingParent>(
        NestingParentProps {
            nested_value: nested_value.clone(),
            nested_probe: nested_probe.clone(),
            probe: parent_probe.clone(),
        },
        &context,
        "NestingParent",
        None,
    );

    // Give the nested state something a rebuild must not lose.
    nested_probe.handle().set_state(|state| state.value = 7);
    element.rebuild_if_dirty(&context);
    assert_eq!(nested_value.get(), 7);

    parent_probe.handle().set_state(|state| state.taps += 1);
    element.rebuild_if_dirty(&context);

    assert_eq!(nested_value.get(), 7, "the state the user can see survives the parent's update");
}

// -- panic recovery ----------------------------------------------------------

pub fn create_panic_becomes_error_element<B: Backend>() {
    phase_recovers::<B>(PanicPhase::CreateState);
}

pub fn init_panic_becomes_error_element<B: Backend>() {
    phase_recovers::<B>(PanicPhase::InitState);
}

pub fn initial_build_panic_becomes_error_element<B: Backend>() {
    phase_recovers::<B>(PanicPhase::Build);
}

pub fn initial_child_conversion_panic_becomes_error_element<B: Backend>() {
    phase_recovers::<B>(PanicPhase::ToElement);
}

pub fn queued_mutation_panic_installs_a_stable_error_child<B: Backend>() {
    let context = build_context();
    let widget = lifecycle(PanicPhase::None);
    let probe = widget.probe.clone();
    let attempts = Rc::new(Cell::new(0));
    let observed = attempts.clone();
    let element = B::element::<Lifecycle>(widget, &context, "LifecycleWidget", None);
    probe.handle().set_state(move |_| {
        observed.set(observed.get() + 1);
        panic!("queued mutation failed");
    });

    element.rebuild_if_dirty(&context);
    assert!(has_error_child(element.as_ref()));
    element.rebuild_if_dirty(&context);
    assert_eq!(attempts.get(), 1, "recovered mutation must not be retried");
}

pub fn dirty_build_panic_installs_a_stable_error_child<B: Backend>() {
    let context = build_context();
    let widget = lifecycle(PanicPhase::None);
    let probe = widget.probe.clone();
    let builds = widget.builds.clone();
    let element = B::element::<Lifecycle>(widget, &context, "LifecycleWidget", None);
    probe.handle().set_state(|state| state.phase = PanicPhase::Build);

    element.rebuild_if_dirty(&context);
    assert!(has_error_child(element.as_ref()));
    element.rebuild_if_dirty(&context);
    assert_eq!(builds.get(), 2, "recovered build must not be retried while clean");
}

pub fn rebuild_child_reconciliation_failure_becomes_a_stable_error_element<B: Backend>() {
    let context = build_context();
    let widget = lifecycle(PanicPhase::None);
    let probe = widget.probe.clone();
    let element = B::element::<Lifecycle>(widget, &context, "LifecycleWidget", None);
    let handle = probe.handle();
    handle.set_state(|state| state.phase = PanicPhase::ChildRuntimeAdoption);

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        element.rebuild_if_dirty(&context);
    }));

    assert!(result.is_ok(), "child reconciliation failure must be recovered");
    assert!(has_error_child(element.as_ref()));
    assert_eq!(handle.try_read(|_| ()), None);
    element.rebuild_if_dirty(&context);
    assert!(has_error_child(element.as_ref()));
}

pub fn keyed_config_panic_becomes_error_element<B: Backend>() {
    let context = build_context();
    let _scope = B::key_scope();
    let key = Key::Value("panic-recovery-key".to_owned());
    let live = B::element::<Lifecycle>(
        lifecycle(PanicPhase::None),
        &context,
        "LifecycleWidget",
        Some(key.clone()),
    );
    assert_eq!(live.debug_name(), "LifecycleWidget");

    let recovered = B::element::<Lifecycle>(
        lifecycle(PanicPhase::AdoptConfig),
        &context,
        "LifecycleWidget",
        Some(key),
    );
    assert_eq!(recovered.debug_name(), "ErrorWidget");
}

// -- carrying state between elements -----------------------------------------

pub fn carry_keeps_the_live_state_and_adopts_the_new_props<B: Backend>() {
    let context = build_context();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let old_probe = Probe::default();
    let old = B::element::<Carry>(
        CarryProps { label: "old", seen: seen.clone(), probe: old_probe.clone() },
        &context,
        "Carry",
        None,
    );
    old_probe.handle().set_state(|state| state.taps = 3);
    old.rebuild_if_dirty(&context);
    assert_eq!(seen.borrow().last(), Some(&("old", 3)));

    let new = B::element::<Carry>(
        CarryProps { label: "new", seen: seen.clone(), probe: Probe::default() },
        &context,
        "Carry",
        None,
    );
    assert_eq!(seen.borrow().last(), Some(&("new", 0)), "a fresh element builds from fresh state");

    carry_stateful(old.as_ref(), new.as_ref(), &context);

    assert_eq!(
        seen.borrow().last(),
        Some(&("new", 3)),
        "the live state keeps its taps and takes the new props"
    );
}

pub fn failed_unkeyed_adoption_keeps_the_old_owner_and_rejects_the_candidate<B: Backend>() {
    let context = build_context();
    let old_widget = lifecycle(PanicPhase::None);
    let old_probe = old_widget.probe.clone();
    let old = B::element::<Lifecycle>(old_widget, &context, "LifecycleWidget", None);
    let new_widget = lifecycle(PanicPhase::AdoptConfig);
    let candidate_probe = new_widget.probe.clone();
    let new = B::element::<Lifecycle>(new_widget, &context, "LifecycleWidget", None);
    let old_handle = old_probe.handle();
    let candidate = candidate_probe.handle();

    carry_stateful(old.as_ref(), new.as_ref(), &context);

    assert!(has_error_child(new.as_ref()));
    assert_eq!(candidate.try_read(|_| ()), None);
    drop(new);

    assert!(old_handle.read(|state| state.phase) == PanicPhase::None);
    assert_eq!(old_handle.try_read(|_| ()), None);
}

pub fn child_reconciliation_failure_keeps_the_old_owner_and_rejects_the_candidate<B: Backend>() {
    let context = build_context();
    let old_widget = lifecycle(PanicPhase::None);
    let old_probe = old_widget.probe.clone();
    let old = B::element::<Lifecycle>(old_widget, &context, "LifecycleWidget", None);
    let new_widget = lifecycle(PanicPhase::ChildRuntimeAdoption);
    let candidate_probe = new_widget.probe.clone();
    let new = B::element::<Lifecycle>(new_widget, &context, "LifecycleWidget", None);
    let old_handle = old_probe.handle();
    let candidate = candidate_probe.handle();

    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        carry_stateful(old.as_ref(), new.as_ref(), &context);
    }));

    assert!(result.is_ok(), "child reconciliation failure must be recovered");
    assert!(has_error_child(new.as_ref()));
    assert_eq!(candidate.try_read(|_| ()), None);
    drop(new);

    assert!(old_handle.read(|state| state.phase == PanicPhase::ChildRuntimeAdoption));
    assert_eq!(old_handle.try_read(|_| ()), None);
}

/// A shared name and key say which element a state *was*, not what it holds.
/// Generic widgets reuse one name for every type parameter, so a replaced
/// subtree pairs elements whose states are different types; neither may take
/// the other's state.
pub fn elements_with_the_same_name_but_another_state_type_keep_their_own_state<B: Backend>() {
    let context = build_context();
    let old_probe = Probe::default();
    let old = B::element::<Alpha>(PlainProps { probe: old_probe.clone() }, &context, "Shared", None);
    let new_probe = Probe::default();
    let new = B::element::<Beta>(BetaProps { probe: new_probe.clone() }, &context, "Shared", None);
    let (old_handle, new_handle) = (old_probe.handle(), new_probe.handle());

    carry_stateful(old.as_ref(), new.as_ref(), &context);

    assert_eq!(new_handle.try_read(|_| ()), Some(()));
    assert_eq!(old_handle.try_read(|_| ()), Some(()));
    // Still its own element after the walk: a later pairing would not mistake
    // it for the other widget either.
    new.rebuild_if_dirty(&context);
    old.rebuild_if_dirty(&context);
    assert_eq!(new_handle.try_read(|_| ()), Some(()));
}

/// A keyed element is found by key, so it keeps its state wherever it
/// reappears, but the subtree it rebuilds from that state is built anew, and
/// what lives below it is unkeyed.
///
/// This is the route transition an application wraps every page in: a theme
/// change rebuilds the shell above it, and the request the page completed, an
/// `AsyncBuilder`'s snapshot, must not start over because the switcher between
/// them was rebuilt.
pub fn state_below_a_keyed_element_outlives_a_rebuild_of_its_ancestor<B: Backend>() {
    let context = build_context();
    let built_by = Rc::new(Cell::new(usize::MAX));
    let page = probe_page::<B>(
        "ProbePageOne",
        InnerProps { next_id: Rc::new(Cell::new(0)), built_by: built_by.clone() },
    );
    let keyed = || {
        B::element::<KeyedOuter>(
            KeyedOuterProps { page: page.clone() },
            &context,
            "KeyedOuter",
            Some(Key::from("keyed-outer")),
        )
    };

    let old = keyed();
    assert_eq!(built_by.get(), 0, "the first subtree builds from its state");

    let new = keyed();
    assert_eq!(built_by.get(), 1, "a fresh subtree builds from fresh state");

    carry_child_state(old.as_ref(), new.as_ref(), &context);

    assert_eq!(
        built_by.get(),
        0,
        "the state below the keyed element was replaced by a fresh one"
    );
}

/// The other half of the same rule: a keyed element outlives changes to what it
/// contains, so what it contains must be free to change.
///
/// One route transition keeps its identity across every route. When the page
/// inside it is replaced, the outgoing page's state must stay with the outgoing
/// page, handing it to the page that replaced it would render the route that
/// was navigated away from.
pub fn a_replaced_page_below_a_keyed_element_keeps_its_own_state<B: Backend>() {
    let context = build_context();
    let built_by = Rc::new(Cell::new(usize::MAX));
    let next_id = Rc::new(Cell::new(0));
    let keyed = |name| {
        B::element::<KeyedOuter>(
            KeyedOuterProps {
                page: probe_page::<B>(
                    name,
                    InnerProps { next_id: next_id.clone(), built_by: built_by.clone() },
                ),
            },
            &context,
            "KeyedOuter",
            Some(Key::from("keyed-outer")),
        )
    };

    let old = keyed("ProbePageOne");
    assert_eq!(built_by.get(), 0);

    let new = keyed("ProbePageTwo");
    assert_ne!(built_by.get(), 0, "the second page builds from its own state");

    carry_child_state(old.as_ref(), new.as_ref(), &context);

    assert_ne!(
        built_by.get(),
        0,
        "the replaced page's state was handed to the page that replaced it"
    );
}

// -- handles after unmount ---------------------------------------------------

pub fn a_copied_handle_is_rejected_after_unmount<B: Backend>() {
    let context = build_context();
    let probe = Probe::default();
    let element = B::element::<Stale>(StaleProps { probe: probe.clone() }, &context, "Stale", None);
    let handle = probe.handle();
    let copied = handle.clone();

    assert_eq!(copied.try_read(|state| state.value.clone()), Some(String::from("live")));
    assert_eq!(copied.try_set_state(|state| state.value.push_str("-updated")), Some(()));
    assert_eq!(copied.try_set_state(|state| state.value.push_str("-again")), Some(()));
    element.rebuild_if_dirty(&context);
    assert_eq!(
        handle.try_read(|state| state.value.clone()),
        Some(String::from("live-updated-again"))
    );

    drop(element);

    let called = Rc::new(Cell::new(false));
    let called_by_mutation = called.clone();
    assert_eq!(copied.try_set_state(move |_| called_by_mutation.set(true)), None);
    assert!(!called.get());
    assert_eq!(copied.try_read(|_| String::from("unexpected")), None);
}

pub fn a_delayed_callback_is_rejected_after_unmount<B: Backend>() {
    let context = build_context();
    let probe = Probe::default();
    let element = B::element::<Stale>(StaleProps { probe: probe.clone() }, &context, "Stale", None);
    let handle = probe.handle();
    let read_called = Rc::new(Cell::new(false));
    let mutation_called = Rc::new(Cell::new(false));
    let delayed: Rc<RefCell<Option<Box<dyn FnOnce()>>>> = Rc::new(RefCell::new(None));
    let read_called_by_callback = read_called.clone();
    let mutation_called_by_callback = mutation_called.clone();
    delayed.borrow_mut().replace(Box::new(move || {
        assert!(!handle.has_state());
        assert_eq!(handle.try_read(move |_| read_called_by_callback.set(true)), None);
        assert_eq!(handle.try_set_state(move |_| mutation_called_by_callback.set(true)), None);
    }));

    drop(element);
    delayed.borrow_mut().take().expect("the delayed callback is retained")();

    assert!(!read_called.get());
    assert!(!mutation_called.get());
}
