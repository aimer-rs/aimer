//! `FutureSnapshotExt::watch`: a future owned by the widget that is building.
//!
//! The future lives in a value slot of the build scope that asked for it (see
//! [`BuildConsumer::retain_value`]), so it outlives the build that started it,
//! is shared by the builds that follow, and is cancelled when a build stops
//! asking for it or the widget goes away. When it finishes, it marks that same
//! scope dirty and requests the frame that rebuilds it.

use std::future::Future;
use std::rc::Rc;

use super::snapshot_task::{SnapshotTask, spawn_snapshot};
use crate::Key;
use crate::base::BuildContext;
use crate::components::context::BuildConsumer;
use crate::widget::stateful::request_state_frame;

/// Implements [`FutureSnapshotExt::watch`](super::FutureSnapshotExt::watch).
pub(super) fn watch_future<F, T: 'static, E: 'static>(future: F, key: Key) -> SnapshotTask<T, E>
where
    F: Future<Output = Result<T, E>> + 'static,
{
    let Some((consumer, window)) = building_scope() else {
        debug_assert!(
            false,
            "`watch` was called outside a build, so nothing would rebuild when it finishes; \
             call it from `build`, or use `waitme` and keep the handle"
        );
        return spawn_snapshot(future, None);
    };
    if let Some(task) = consumer.retained_value::<SnapshotTask<T, E>>(&key) {
        // `future` was only a description of the request; it was never polled.
        return task;
    }

    let owner = Rc::downgrade(&consumer);
    let task = spawn_snapshot(
        future,
        Some(Box::new(move || {
            // The scope is gone once its widget is: nobody is left to rebuild.
            if let Some(owner) = owner.upgrade() {
                owner.mark_needs_rebuild();
                request_state_frame(window);
            }
        })),
    );
    consumer.retain_value(key, task.clone());
    task
}

/// The scope of the build running on this thread and the window it renders to.
fn building_scope() -> Option<(Rc<BuildConsumer>, aimer_events::window::WindowKey)> {
    let current = BuildContext::try_current().ok()?;
    let ctx = current.get();
    Some((ctx.current_build_consumer()?, ctx.window.key()))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::future::pending;
    use std::rc::Rc;

    use aimer_attribute::ResolvedSize;
    use aimer_venus::Venus;

    use crate::base::{BuildContext, WindowHandle};
    use crate::widget::stateless::StatelessElement;
    use crate::{
        AnyElement, AsyncSnapshot, AsyncSnapshotError, Drawable, Element, EventElement,
        FutureSnapshotExt, LayoutElement, Rebuildable, State, StateUpdater, StatefulElement,
        StatefulWidget, VisitorElement, Widget,
    };

    struct Leaf;

    impl VisitorElement for Leaf {
        fn debug_name(&self) -> &'static str {
            "Leaf"
        }
    }
    impl Drawable for Leaf {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl LayoutElement for Leaf {}
    impl EventElement for Leaf {}
    impl Rebuildable for Leaf {}

    /// Every test runs on a thread of its own, so the installed runtime and the
    /// redraw requester belong to that test alone.
    fn context() -> (Rc<Venus>, BuildContext<'static>) {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
        });
        let _entered = runtime.enter();

        let venus = Venus::new();
        venus.install();
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let ctx = BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        (venus, ctx)
    }

    fn redraw_counter() -> Rc<Cell<usize>> {
        let count = Rc::new(Cell::new(0));
        let counted = count.clone();
        aimer_events::window::set_thread_redraw_requester(move || counted.set(counted.get() + 1));
        count
    }

    type Snapshot = AsyncSnapshot<u32, AsyncSnapshotError<&'static str>>;

    fn label(snapshot: &Snapshot) -> &'static str {
        match snapshot {
            AsyncSnapshot::Waiting => "waiting",
            AsyncSnapshot::Data(_) => "data",
            AsyncSnapshot::Error(_) => "error",
        }
    }

    /// A widget that watches one future per `key`, and records what each of its
    /// builds saw.
    #[derive(Clone, Default)]
    struct Probe {
        key: Rc<Cell<u32>>,
        builds: Rc<Cell<usize>>,
        polls: Rc<Cell<usize>>,
        seen: Rc<RefCell<Vec<&'static str>>>,
        fail: Rc<Cell<bool>>,
        skip: Rc<Cell<bool>>,
    }

    impl Probe {
        fn element(&self, ctx: &BuildContext) -> StatelessElement {
            let probe = self.clone();
            StatelessElement::from_builder(
                ctx,
                move |_| {
                    probe.builds.set(probe.builds.get() + 1);
                    if probe.skip.get() {
                        return Leaf.boxed();
                    }
                    let id = probe.key.get();
                    let polls = probe.polls.clone();
                    let fail = probe.fail.get();
                    let task = async move {
                        polls.set(polls.get() + 1);
                        if fail { Err("failed") } else { Ok::<u32, &'static str>(id) }
                    }
                    .watch(format!("job:{id}"));
                    probe.seen.borrow_mut().push(label(&task.read()));
                    Leaf.boxed()
                },
                None,
                "Probe",
            )
        }

        fn seen(&self) -> Vec<&'static str> {
            self.seen.borrow().clone()
        }
    }

    fn rebuild(element: &StatelessElement, ctx: &BuildContext) {
        element.mark_needs_rebuild();
        element.rebuild_if_dirty(ctx);
    }

    #[test]
    fn a_finished_future_rebuilds_the_widget_that_watched_it() {
        let (venus, ctx) = context();
        let redraws = redraw_counter();
        let probe = Probe::default();
        let element = probe.element(&ctx);

        assert_eq!(probe.seen(), ["waiting"]);
        assert!(!element.dirty.get());

        venus.run_microtasks();

        assert!(element.dirty.get(), "completion marks the owner dirty");
        assert_eq!(redraws.get(), 1, "completion asks for a frame");
        element.rebuild_if_dirty(&ctx);
        assert_eq!(probe.seen(), ["waiting", "data"]);
        assert_eq!(probe.builds.get(), 2);
        assert_eq!(probe.polls.get(), 1);
    }

    #[test]
    fn an_error_result_is_a_snapshot_error_and_rebuilds() {
        let (venus, ctx) = context();
        let probe = Probe::default();
        probe.fail.set(true);
        let element = probe.element(&ctx);

        venus.run_microtasks();
        assert!(element.dirty.get());
        element.rebuild_if_dirty(&ctx);

        assert_eq!(probe.seen(), ["waiting", "error"]);
    }

    #[test]
    fn rebuilds_that_ask_for_the_same_key_share_one_future() {
        let (venus, ctx) = context();
        let probe = Probe::default();
        let element = probe.element(&ctx);

        rebuild(&element, &ctx);
        assert_eq!(venus.task_count(), 1, "the unused future was not spawned");
        venus.run_microtasks();
        rebuild(&element, &ctx);
        rebuild(&element, &ctx);

        assert_eq!(probe.polls.get(), 1, "the future ran once");
        assert_eq!(probe.builds.get(), 4);
        assert_eq!(probe.seen(), ["waiting", "waiting", "data", "data"]);
    }

    #[test]
    fn a_widget_owns_its_futures_even_when_another_uses_the_same_key() {
        let (venus, ctx) = context();
        let first = Probe::default();
        let second = Probe::default();
        let _first = first.element(&ctx);
        let _second = second.element(&ctx);

        venus.run_microtasks();

        assert_eq!((first.polls.get(), second.polls.get()), (1, 1));
    }

    /// Pending future whose cancellation is observable: the id is recorded when
    /// the future is dropped after having started.
    fn cancellable(id: u32, dropped: Rc<RefCell<Vec<u32>>>) -> impl std::future::Future<Output = Result<u32, ()>> {
        struct Guard(u32, Rc<RefCell<Vec<u32>>>);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.1.borrow_mut().push(self.0);
            }
        }
        async move {
            let _guard = Guard(id, dropped);
            pending::<Result<u32, ()>>().await
        }
    }

    struct Watcher {
        element: StatelessElement,
        key: Rc<Cell<u32>>,
        ask: Rc<Cell<bool>>,
        dropped: Rc<RefCell<Vec<u32>>>,
    }

    fn watcher(ctx: &BuildContext) -> Watcher {
        let key = Rc::new(Cell::new(1));
        let ask = Rc::new(Cell::new(true));
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let element = {
            let (key, ask, dropped) = (key.clone(), ask.clone(), dropped.clone());
            StatelessElement::from_builder(
                ctx,
                move |_| {
                    if ask.get() {
                        let id = key.get();
                        let _ = cancellable(id, dropped.clone()).watch(format!("job:{id}"));
                    }
                    Leaf.boxed()
                },
                None,
                "Watcher",
            )
        };
        Watcher { element, key, ask, dropped }
    }

    #[test]
    fn a_changed_key_cancels_the_old_future_and_starts_a_new_one() {
        let (venus, ctx) = context();
        let watcher = watcher(&ctx);
        venus.run_microtasks();
        assert!(watcher.dropped.borrow().is_empty());

        watcher.key.set(2);
        rebuild(&watcher.element, &ctx);

        assert_eq!(*watcher.dropped.borrow(), [1], "the old future was cancelled");
        venus.run_microtasks();
        assert_eq!(venus.task_count(), 1, "the new future is running");
    }

    #[test]
    fn a_build_that_stops_asking_cancels_its_future() {
        let (venus, ctx) = context();
        let watcher = watcher(&ctx);
        venus.run_microtasks();

        watcher.ask.set(false);
        rebuild(&watcher.element, &ctx);

        assert_eq!(*watcher.dropped.borrow(), [1]);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn unmounting_the_widget_cancels_its_future() {
        let (venus, ctx) = context();
        let Watcher { element, dropped, .. } = watcher(&ctx);
        venus.run_microtasks();
        assert_eq!(venus.task_count(), 1);

        drop(element);

        assert_eq!(*dropped.borrow(), [1]);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn reusing_a_key_for_another_result_type_replaces_the_future() {
        let (venus, ctx) = context();
        let wide = Rc::new(Cell::new(false));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let element = {
            let (wide, seen) = (wide.clone(), seen.clone());
            StatelessElement::from_builder(
                &ctx,
                move |_| {
                    if wide.get() {
                        let task = async { Ok::<u64, ()>(7) }.watch("job");
                        seen.borrow_mut().push(matches!(&*task.read(), AsyncSnapshot::Data(_)));
                    } else {
                        let task = async { Ok::<u32, ()>(7) }.watch("job");
                        seen.borrow_mut().push(matches!(&*task.read(), AsyncSnapshot::Data(_)));
                    }
                    Leaf.boxed()
                },
                None,
                "Retyped",
            )
        };
        venus.run_microtasks();
        element.rebuild_if_dirty(&ctx);
        wide.set(true);

        rebuild(&element, &ctx);
        assert_eq!(*seen.borrow(), [false, true, false], "the new type starts waiting");
        venus.run_microtasks();
        element.rebuild_if_dirty(&ctx);
        assert_eq!(seen.borrow().last(), Some(&true));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "outside a build")]
    fn watching_outside_a_build_is_a_bug() {
        let (_venus, _ctx) = context();
        let _ = async { Ok::<u32, ()>(1) }.watch("job");
    }

    struct LeafWidget;

    impl Widget for LeafWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Leaf.boxed()
        }
    }

    impl crate::widget::PortableWidget for LeafWidget {}

    struct Fetching {
        seen: Rc<RefCell<Vec<&'static str>>>,
    }

    struct FetchingState {
        seen: Rc<RefCell<Vec<&'static str>>>,
    }

    impl StatefulWidget for Fetching {
        type State = FetchingState;

        fn create_state(self) -> Self::State {
            FetchingState { seen: self.seen }
        }
    }

    impl State<Fetching> for FetchingState {
        fn init_state(&mut self, _updater: StateUpdater<Self>) {}

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            let task = async { Ok::<u32, &'static str>(1) }.watch("fetch");
            self.seen.borrow_mut().push(label(&task.read()));
            LeafWidget
        }
    }

    #[test]
    fn a_stateful_widget_is_rebuilt_when_its_watched_future_finishes() {
        let (venus, ctx) = context();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let (element, _updater) =
            StatefulElement::new_with_name(Fetching { seen: seen.clone() }, &ctx, "Fetching", None);
        assert_eq!(*seen.borrow(), ["waiting"]);
        assert!(!element.is_dirty());

        venus.run_microtasks();

        assert!(element.is_dirty());
        element.rebuild_if_dirty(&ctx);
        assert_eq!(*seen.borrow(), ["waiting", "data"]);
    }
}
