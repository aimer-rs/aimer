//! A slot that keeps only the newest piece of async work: [`Latest`].

use std::cell::RefCell;
use std::rc::Rc;

use crate::handle::{JoinHandle, spawn_with_handle};
use crate::scheduler::LocalScheduler;
use crate::task::{Phase, ScopeId};
use crate::venus::Venus;

/// Runs one async operation at a time, cancelling the previous when a new one
/// starts.
///
/// This is the "latest wins" rule of search-as-you-type, a typeahead, a detail
/// pane following a selection: when the input changes, the answer to the old
/// input is no longer wanted, and must not arrive *after* the answer to the new
/// one. Starting the next operation drops the previous future, so a slow
/// response for an old query cannot overwrite a fresh result — there is nothing
/// left to publish it.
///
/// Dropping the slot cancels the operation in flight, so an element that owns
/// one cleans up when it unmounts.
///
/// Cancellation drops the future; work the future already handed to
/// [`Venus::spawn_blocking`] still runs to completion on its worker and its
/// result is discarded.
///
/// # Examples
///
/// Typing into a search box, debounced, with only the newest query ever
/// answered:
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
/// use std::time::Duration;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let results = Rc::new(RefCell::new(String::new()));
///
/// let search = Rc::new(venus.latest());
/// let debounce = venus.debouncer(Duration::from_millis(250));
///
/// let on_keystroke = |query: &'static str| {
///     let (search, results) = (search.clone(), results.clone());
///     let _ = debounce.call(move || {
///         search.run(async move {
///             // fetch(query).await ...
///             *results.borrow_mut() = format!("results for {query}");
///         });
///     });
/// };
/// on_keystroke("ru");
/// on_keystroke("rust");
/// ```
pub struct Latest {
    scheduler: Rc<LocalScheduler>,
    current: RefCell<Option<JoinHandle<()>>>,
}

impl Latest {
    fn new(scheduler: Rc<LocalScheduler>) -> Self {
        Self {
            scheduler,
            current: RefCell::new(None),
        }
    }

    /// Cancels the operation in flight, if any, and starts `future` as a
    /// microtask in its place.
    ///
    /// The future publishes its own result — by updating state, say. Nothing
    /// from a cancelled operation can arrive after this returns.
    #[inline]
    pub fn run(&self, future: impl Future<Output = ()> + 'static) {
        self.run_in_phase(Phase::Microtask, future);
    }

    /// [`run`](Self::run) into a chosen phase; an idle operation is a good fit
    /// for a prefetch that should not compete with the frame.
    pub fn run_in_phase(&self, phase: Phase, future: impl Future<Output = ()> + 'static) {
        // The old operation goes first, so its destructors run before the new
        // one exists. Taken out of the cell before it is dropped: those
        // destructors may reach back into this slot.
        let previous = self.current.take();
        drop(previous);
        let handle = spawn_with_handle(&self.scheduler, phase, ScopeId::ROOT, future);
        *self.current.borrow_mut() = Some(handle);
    }

    /// Cancels the operation in flight, returning whether there was one.
    pub fn cancel(&self) -> bool {
        let running = self.is_running();
        let previous = self.current.take();
        drop(previous);
        running
    }

    /// Whether an operation has started and not yet finished or been cancelled.
    pub fn is_running(&self) -> bool {
        self.current
            .borrow()
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
    }
}

impl Venus {
    /// Creates an empty [`Latest`] slot on this runtime.
    #[inline]
    pub fn latest(&self) -> Latest {
        Latest::new(self.scheduler().clone())
    }
}

/// [`Venus::latest`] on the runtime installed for this thread; `None` when
/// there is none.
#[inline]
pub fn latest() -> Option<Latest> {
    Venus::current().map(|venus| venus.latest())
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::test_host::with_timers;
    use crate::yield_now;

    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn a_new_operation_cancels_the_previous_one() {
        let venus = Venus::new();
        let slot = venus.latest();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        slot.run(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        venus.run_microtasks();
        assert!(slot.is_running());
        assert!(!dropped.get());

        slot.run(async {});

        assert!(dropped.get(), "the old future was dropped as the new one started");
        assert_eq!(venus.task_count(), 1);
    }

    #[test]
    fn a_stale_slow_result_can_never_overwrite_a_newer_one() {
        let (venus, timers) = with_timers();
        let slot = venus.latest();
        let shown = Rc::new(RefCell::new(String::new()));

        for (query, wait) in [("rust", 300), ("rus", 10)] {
            let shown = shown.clone();
            let sleep = venus.sleep(std::time::Duration::from_millis(wait));
            slot.run(async move {
                let _ = sleep.await;
                *shown.borrow_mut() = format!("results for {query}");
            });
            venus.run_microtasks();
        }
        // The newer, faster request lands first and the older one is already
        // cancelled, so firing both timers publishes only the newest.
        timers.fire(1);
        venus.run_microtasks();
        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(*shown.borrow(), "results for rus");
    }

    #[test]
    fn a_finished_operation_is_not_running_and_the_slot_is_reusable() {
        let venus = Venus::new();
        let slot = venus.latest();
        let runs = Rc::new(Cell::new(0));

        for _ in 0..3 {
            let counted = runs.clone();
            slot.run(async move {
                yield_now().await;
                counted.set(counted.get() + 1);
            });
            assert!(slot.is_running());
            venus.run_microtasks();
            assert!(!slot.is_running());
        }

        assert_eq!(runs.get(), 3);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn cancel_stops_the_operation_and_reports_whether_there_was_one() {
        let venus = Venus::new();
        let slot = venus.latest();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        slot.run(async move { flag.set(true) });

        assert!(slot.cancel());
        assert!(!slot.cancel(), "nothing left to cancel");
        venus.run_microtasks();

        assert!(!ran.get());
        assert!(!slot.is_running());
    }

    #[test]
    fn dropping_the_slot_cancels_the_operation() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        {
            let slot = venus.latest();
            slot.run(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            venus.run_microtasks();
        }

        assert!(dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn an_operation_may_start_the_next_one_from_inside_itself() {
        let venus = Venus::new();
        let slot = Rc::new(venus.latest());
        let log = Rc::new(RefCell::new(Vec::new()));

        let (again, first, second) = (slot.clone(), log.clone(), log.clone());
        slot.run(async move {
            first.borrow_mut().push("first");
            again.run(async move { second.borrow_mut().push("second") });
        });
        venus.run_until_stalled();

        assert_eq!(*log.borrow(), ["first", "second"]);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn an_idle_operation_waits_for_room() {
        let venus = Venus::new();
        let slot = venus.latest();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        slot.run_in_phase(Phase::Idle, async move { flag.set(true) });

        venus.run_microtasks();
        assert!(!ran.get());

        venus.run_until_stalled();
        assert!(ran.get());
    }

    #[test]
    fn the_free_constructor_needs_an_installed_runtime() {
        assert!(latest().is_none());

        let venus = Venus::new();
        venus.install();
        let slot = latest().expect("a runtime is installed");
        Venus::uninstall();
        slot.run(async {});

        assert_eq!(venus.run_until_stalled(), 1);
    }
}
