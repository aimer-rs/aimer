//! Edge cases of the borrow-free [`StateView`]: copies that outlive their build,
//! slots that are reused, reconciliation, nested access and failure.

use super::*;

type Views = Rc<RefCell<Vec<StateView<CounterState>>>>;

struct CounterWidget {
    start: i32,
    views: Views,
}

struct CounterState {
    count: i32,
    views: Views,
}

impl Widget for CounterWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget::<Self, CounterState>(self, ctx, "CounterWidget", None)
    }
}

impl State<CounterWidget> for CounterState {
    fn create(widget: CounterWidget) -> Self {
        Self {
            count: widget.start,
            views: widget.views,
        }
    }

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        // A copy per build: every one must resolve to the same state.
        this.views.borrow_mut().push(this);
        // A build that panics on a poisoned count, to exercise the failure path.
        assert!(this.count != -1, "poisoned count");
        LeafProbe
    }
}

fn counter(
    context: &BuildContext,
    start: i32,
) -> (StatefulElement, StateUpdater<CounterState>, Views) {
    let views = Views::default();
    let (element, updater) = StatefulElement::new_with_name::<CounterWidget, CounterState>(
        CounterWidget {
            start,
            views: views.clone(),
        },
        context,
        "CounterWidget",
        None,
    );
    (element, updater, views)
}

fn panic_message(result: std::thread::Result<impl Sized>) -> String {
    let payload = result.err().expect("the call should have panicked");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_default()
}

#[test]
fn copies_made_by_different_builds_resolve_to_the_same_state() {
    let context = dummy_build_context();
    let (element, _updater, views) = counter(&context, 1);
    let first = views.borrow()[0];

    first.set_state(|state| state.count += 1);
    element.rebuild_if_dirty(&context);
    let second = views.borrow()[1];

    assert_eq!((first.count, second.count), (2, 2));
    second.set_state(|state| state.count *= 10);
    element.rebuild_if_dirty(&context);
    assert_eq!((first.count, second.count, views.borrow()[2].count), (20, 20, 20));
}

#[test]
fn mutations_from_several_copies_apply_in_queue_order_in_one_rebuild() {
    let context = dummy_build_context();
    let (element, _updater, views) = counter(&context, 1);
    let a = views.borrow()[0];
    let b = a;
    let builds_before = views.borrow().len();

    a.set_state(|state| state.count += 1); // 2
    b.set_state(|state| state.count *= 5); // 10
    a.set_state_with(&3, |state, by| state.count -= by); // 7
    assert_eq!(b.try_set_state(|state| state.count += 100), Some(()));
    element.rebuild_if_dirty(&context);

    assert_eq!(a.count, 107);
    assert_eq!(views.borrow().len(), builds_before + 1, "one coalesced rebuild");
}

#[test]
fn a_stale_view_never_resolves_to_an_element_that_reuses_its_slot() {
    let context = dummy_build_context();
    let (gone, _updater, views) = counter(&context, 1);
    let stale = views.borrow()[0];
    drop(gone);

    // Many live elements of the same state type take whatever slots are free.
    let live: Vec<_> = (0..64).map(|index| counter(&context, 1000 + index)).collect();

    assert!(!stale.has_state());
    assert!(!stale.mounted());
    assert_eq!(stale.try_read(|state| state.count), None);
    assert!(stale.try_read_state().is_none());
    assert_eq!(stale.try_set_state(|state| state.count = -5), None);
    let message = panic_message(std::panic::catch_unwind(AssertUnwindSafe(|| stale.count)));
    assert!(message.contains("dropped or stale"), "unexpected message: {message}");
    for (index, (element, updater, _)) in live.iter().enumerate() {
        element.rebuild_if_dirty(&context);
        assert_eq!(
            updater.try_read(|state| state.count),
            Some(1000 + index as i32),
            "the stale view must not have written to another element"
        );
    }
}

#[test]
fn a_stale_view_is_not_confused_with_a_slot_holding_another_state_type() {
    let context = dummy_build_context();
    let (gone, _updater, views) = counter(&context, 1);
    let stale = views.borrow()[0];
    drop(gone);

    // A different state type is likely to land in the freed slot.
    let others: Vec<_> = (0..16)
        .map(|_| StatefulElement::new_with_name::<AlphaWidget, AlphaState>(AlphaWidget, &context, "AlphaWidget", None))
        .collect();

    assert_eq!(stale.try_read(|state| state.count), None);
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| stale.count)).is_err());
    drop(others);
}

#[test]
fn reading_and_writing_inside_a_read_closure_and_beside_a_guard() {
    let context = dummy_build_context();
    let (element, _updater, views) = counter(&context, 4);
    let view = views.borrow()[0];

    // `read` runs its closure with the slot resolved; the view must still work.
    let total = view.read(|state| {
        view.set_state(|state| state.count += 1);
        state.count + view.count
    });
    assert_eq!(total, 8);

    // A read guard held across a deref and a queued write.
    {
        let guard = view.read_state();
        assert_eq!(guard.count + view.count, 8);
        view.set_state(|state| state.count += 1);
        assert_eq!(view.try_read_state().map(|other| other.count), Some(4));
    }
    element.rebuild_if_dirty(&context);
    assert_eq!(view.count, 6);
}

#[test]
fn a_guard_keeps_the_value_readable_after_the_element_drops_but_the_view_does_not() {
    let context = dummy_build_context();
    let (element, _updater, views) = counter(&context, 9);
    let view = views.borrow()[0];

    let guard = view.read_state();
    drop(element);

    assert_eq!(guard.count, 9, "the lease keeps the state alive");
    assert!(!view.has_state());
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| view.count)).is_err());
    drop(guard);
}

#[test]
fn a_view_stays_usable_when_reconciliation_hands_its_state_to_a_new_element() {
    let context = dummy_build_context();
    let (old, _old_updater, old_views) = counter(&context, 1);
    let (new, _new_updater, new_views) = counter(&context, 500);
    let old_view = old_views.borrow()[0];

    old_view.set_state(|state| state.count = 7);
    old.rebuild_if_dirty(&context);
    carry_stateful(&old, &new, &context);

    // `new` took over the live state (7); the view made by `old` still reaches it.
    assert_eq!(old_view.count, 7);
    // The live state keeps `old`'s widget configuration, so its builds log into
    // `old_views`, and the candidate's own, never-adopted view stays on its slot.
    let candidate_view = new_views.borrow()[0];
    assert_eq!(candidate_view.try_read(|state| state.count), None, "the candidate's state was consumed");
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| candidate_view.count)).is_err());

    // Dropping the element that merely lost its state must not kill the live one.
    drop(old);
    assert_eq!(old_view.try_read(|state| state.count), Some(7));
    old_view.set_state(|state| state.count += 1);
    new.rebuild_if_dirty(&context);
    let adopted = *old_views.borrow().last().expect("new rebuilt from the live state");
    assert_eq!(adopted.count, 8);

    drop(new);
    assert_eq!(old_view.try_read(|state| state.count), None);
}

#[test]
fn the_candidate_that_lost_reconciliation_leaves_no_dangling_view() {
    let context = dummy_build_context();
    let (old, _old_updater, _old_views) = counter(&context, 1);
    let (new, _new_updater, new_views) = counter(&context, 500);
    let candidate_view = new_views.borrow()[0];

    carry_stateful(&old, &new, &context);

    // Its state was handed over, so the checked reads say so and a plain read
    // panics instead of touching what was moved out.
    assert_eq!(candidate_view.try_read(|state| state.count), None);
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| candidate_view.count)).is_err());
    drop(new);
    assert_eq!(candidate_view.try_read(|state| state.count), None);
    drop(old);
}

#[test]
fn a_view_of_a_failed_build_reports_unusable_through_the_checked_methods() {
    let context = dummy_build_context();
    let (element, _updater, views) = counter(&context, 1);
    let view = views.borrow()[0];

    view.set_state(|state| state.count = -1);
    let rebuilt = std::panic::catch_unwind(AssertUnwindSafe(|| element.rebuild_if_dirty(&context)));
    assert!(rebuilt.is_ok(), "a build panic is recovered into an error element");

    assert!(!view.has_state());
    assert_eq!(view.try_read(|state| state.count), None);
    assert_eq!(view.try_set_state(|state| state.count = 2), None);
    assert!(view.try_read_state().is_none());
    // The element is still alive and its state intact, so the view itself is not stale.
    assert!(view.mounted() == view.has_state());
}

#[test]
fn a_zero_sized_state_works_through_a_copied_view() {
    let context = dummy_build_context();
    let (element, updater) =
        StatefulElement::new_with_name::<AlphaWidget, AlphaState>(AlphaWidget, &context, "AlphaWidget", None);
    let view = AlphaViews::take();

    view.set_state(|_| {});
    element.rebuild_if_dirty(&context);
    assert!(view.has_state());
    let _: &AlphaState = &view;
    assert_eq!(view.try_read(|_| 3), Some(3));
    drop(element);
    assert!(!updater.has_state());
    assert!(!view.has_state());
}

#[test]
fn copies_of_a_detached_view_read_the_same_state_and_are_not_mounted() {
    struct Plain(Cell<i32>);
    let state = Plain(Cell::new(3));
    // SAFETY: `state` outlives every copy below.
    let view = unsafe { StateView::detached(&state) };
    let copy = view;

    assert_eq!((view.0.get(), copy.0.get()), (3, 3));
    state.0.set(8);
    assert_eq!((view.0.get(), copy.0.get()), (8, 8), "a detached view reads the live caller state");
    assert!(!view.mounted() && !copy.mounted());
    assert!(!copy.has_state());
    assert_eq!(copy.try_read(|plain| plain.0.get()), None);
}

// A zero-sized state that hands its view out through a thread local, since a
// widget carrying an `Rc` would not be zero-sized.
thread_local! {
    static ALPHA_VIEW: Cell<Option<StateView<AlphaState>>> = const { Cell::new(None) };
}

struct AlphaViews;

impl AlphaViews {
    fn take() -> StateView<AlphaState> {
        ALPHA_VIEW.with(|slot| slot.take()).expect("the state's build stores its view")
    }
}

pub(super) fn remember_alpha_view(view: StateView<AlphaState>) {
    ALPHA_VIEW.with(|slot| slot.set(Some(view)));
}
