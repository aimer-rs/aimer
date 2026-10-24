#![doc = include_str!("../README.md")]

mod budget;
mod completion;
// The browser has no threads to run a lane on.
#[cfg(not(target_arch = "wasm32"))]
mod lane;
mod lifecycle;
mod panic_hook;
// The browser has no threads to offload onto, and `requestIdleCallback` plus a
// budgeted idle phase is what takes its place there.
#[cfg(not(target_arch = "wasm32"))]
mod offload;
mod poll_context;
mod scheduler;
mod task;
mod timeout;
mod venus;
mod yielding;

pub use crate::budget::{
    FrameBudget, FrameGovernor, IDLE_SLICE_FLOOR, MICROTASK_BUDGET_WARNING, time_remaining_in_frame,
};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::offload::{OffloadPool, Offloaded};
pub use crate::completion::{Completer, Completion, Interrupted, completion};
#[cfg(not(target_arch = "wasm32"))]
pub use crate::lane::{Lane, LaneState};
pub use crate::lifecycle::{Lifecycle, LifecycleSubscription, notify_lifecycle, on_lifecycle};
pub use crate::panic_hook::install_panic_hook;
pub use crate::poll_context::PollContext;
pub use crate::scheduler::{LocalScheduler, is_polling_task};
pub use crate::task::{Notifier, Phase, ScopeId, TaskId, TaskScope};
pub use crate::timeout::TimeoutError;
pub use crate::venus::{Venus, spawn_local};
pub use crate::yielding::{YieldNow, yield_if_over_budget, yield_now};

/// Installs the logical time associated with one portable guest build.
///
/// This is a no-op for native and ordinary browser builds. Keeping the symbol
/// available in every build configuration lets generated guest code share its
/// build-context implementation with the host crate graph.
#[doc(hidden)]
#[inline]
pub fn set_portable_frame_time(frame: u64) {
    budget::set_portable_frame_time(frame);
}

#[cfg(test)]
mod frame_overhead {
    //! What the scheduler itself costs a frame that has nothing to do.
    //!
    //! Every frame of every Aimer application pays this, whether or not anything
    //! asynchronous is happening, so it is worth a number rather than an assumption.
    //! When an application feels slow the first question is whether the runtime is
    //! the reason, and this answers it: the phases of an empty frame cost tens of
    //! nanoseconds, roughly two clock reads, which is four orders of magnitude
    //! below a 120 Hz frame.
    //!
    //! The bound is deliberately loose. This is not a benchmark — it is a guard
    //! against a drain or a queue scan quietly becoming linear in the number of
    //! tasks, or a phase starting to allocate. A real measurement belongs in a
    //! release build with a benchmark harness.

    use std::time::{Duration, Instant};

    use crate::Venus;

    /// The most this may cost before something is structurally wrong.
    ///
    /// Two orders of magnitude above what it measures, so a loaded CI machine
    /// cannot fail it, and still two orders of magnitude below a frame.
    const CEILING: Duration = Duration::from_micros(50);

    /// One frame's worth of scheduler work, exactly as `aimer_quiver` drives it.
    fn drive_one_empty_frame(venus: &Venus) {
        venus.begin_frame();
        venus.run_frame_tasks();
        venus.run_microtasks();
        let budget = venus.idle_budget();
        venus.run_idle(&budget);
        venus.end_frame();
        let _ = venus.has_ready_work();
    }

    #[test]
    fn an_empty_frame_costs_the_scheduler_almost_nothing() {
        let venus = Venus::new();
        let rounds = 200_000;

        for _ in 0..1_000 {
            drive_one_empty_frame(&venus);
        }

        let start = Instant::now();
        for _ in 0..rounds {
            drive_one_empty_frame(&venus);
        }
        let per_frame = start.elapsed() / rounds;

        println!("empty frame: {per_frame:?} of scheduler work");
        assert!(
            per_frame < CEILING,
            "the scheduler alone costs {per_frame:?} of every frame"
        );
    }

    /// An application that is idle must let the loop sleep.
    ///
    /// A runtime reporting work it does not have turns every frame into a request
    /// for the next one, and an application that renders flat out forever reads to
    /// a user as a slow one.
    #[test]
    fn an_empty_frame_reports_no_work_to_come_back_for() {
        let venus = Venus::new();

        for _ in 0..8 {
            drive_one_empty_frame(&venus);
            assert!(
                !venus.has_ready_work(),
                "an idle frame claimed to have work"
            );
        }
    }
}

#[cfg(test)]
mod runtime_spec {
    //! The properties that justify Venus existing at all.
    //!
    //! Each test here pins one thing a general-purpose runtime cannot do: keep a
    //! non-`Send` capture, land an effect *before* the frame's build phase, stop
    //! when the frame budget runs out, and forget a task when the element that
    //! owned it went away.

    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use crate::{FrameBudget, Venus, yield_now};

    /// Code that runs inside a task can tell, and code that runs outside one
    /// can tell that too. A state update uses this to decide how much of the
    /// next frame it may leave alone.
    #[test]
    fn a_task_can_tell_it_is_being_polled() {
        let venus = Venus::new();
        let seen = Rc::new(Cell::new(None));

        assert!(
            !crate::is_polling_task(),
            "nothing is polling before a task runs"
        );
        let observed = seen.clone();
        venus.spawn(async move {
            observed.set(Some(crate::is_polling_task()));
        });
        venus.run_microtasks();

        assert_eq!(
            seen.get(),
            Some(true),
            "the task saw that it was being polled"
        );
        assert!(
            !crate::is_polling_task(),
            "and it is over once the poll returns"
        );
    }

    /// The bound the whole design exists to delete: a task captures an `Rc` — a
    /// `StateUpdater`, a controller — and still runs.
    #[test]
    fn a_local_task_keeps_a_non_send_capture() {
        let venus = Venus::new();
        let calls = Rc::new(Cell::new(0));

        let counted = calls.clone();
        venus.spawn(async move {
            counted.set(counted.get() + 1);
        });

        venus.run_microtasks();

        assert_eq!(calls.get(), 1);
        assert_eq!(venus.task_count(), 0);
    }

    /// The one-frame-latency bug, stated as a test: an effect produced by a
    /// resolved future has to be visible to *this* frame's build, not the next.
    #[test]
    fn a_resolved_future_lands_before_the_build_phase() {
        let venus = Venus::new();
        let state = Rc::new(Cell::new(0));
        let observed_by_build = Rc::new(Cell::new(-1));

        let mutated = state.clone();
        venus.spawn(async move {
            yield_now().await;
            mutated.set(7);
        });

        let read = state.clone();
        let seen = observed_by_build.clone();
        venus.drive_frame(|| seen.set(read.get()));

        assert_eq!(observed_by_build.get(), 7);
    }

    /// Idle work is budget-gated: nothing runs without measured room, and what did
    /// run keeps its state for the next frame.
    #[test]
    fn idle_work_waits_for_room_and_resumes_with_its_state_intact() {
        let venus = Venus::new();
        let progress = Rc::new(Cell::new(0));

        let counted = progress.clone();
        venus.spawn_idle(async move {
            for _ in 0..3 {
                counted.set(counted.get() + 1);
                yield_now().await;
            }
        });

        assert_eq!(venus.run_idle(&FrameBudget::exhausted()), 0);
        assert_eq!(progress.get(), 0);

        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(4)));

        assert_eq!(progress.get(), 3);
        assert_eq!(venus.task_count(), 0);
    }

    /// Structural cancellation: an unmounted element drops its scope, and the tasks
    /// it spawned are simply gone — no abort handle, no generation counter.
    #[test]
    fn dropping_a_scope_forgets_its_tasks() {
        let venus = Venus::new();
        let ran = Rc::new(Cell::new(false));

        let scope = venus.scope();
        let flag = ran.clone();
        venus.spawn_in(scope.id(), async move {
            flag.set(true);
        });
        assert_eq!(venus.task_count(), 1);

        drop(scope);
        venus.run_microtasks();

        assert!(!ran.get());
        assert_eq!(venus.task_count(), 0);
    }

    /// The scheduler must never be the reason a frame is missed: draining a
    /// pathological number of ready microtasks costs a fraction of one frame.
    #[test]
    fn draining_ten_thousand_microtasks_costs_far_less_than_a_frame() {
        let venus = Venus::new();
        let calls = Rc::new(Cell::new(0));

        for _ in 0..10_000 {
            let counted = calls.clone();
            venus.spawn(async move {
                counted.set(counted.get() + 1);
            });
        }

        let started = web_time::Instant::now();
        venus.run_microtasks();
        let elapsed = started.elapsed();

        assert_eq!(calls.get(), 10_000);
        assert!(
            elapsed < Duration::from_millis(4),
            "draining 10_000 microtasks took {elapsed:?}"
        );
    }

    /// Blocking work leaves the UI thread and comes back as a microtask, so the
    /// awaiting task is still holding its `Rc` when the value arrives.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn offloaded_work_comes_back_to_the_ui_thread() {
        let venus = Venus::new();
        let state = Rc::new(Cell::new(0));

        let venus_for_task = venus.clone();
        let mutated = state.clone();
        venus.spawn(async move {
            let sum = venus_for_task
                .spawn_blocking(|| (1..=10).sum::<u32>())
                .await;
            mutated.set(sum);
        });

        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        assert_eq!(state.get(), 55);
    }

    /// Blocking work runs on a pool worker instead of the UI thread.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn spawn_blocking_runs_work_off_the_ui_thread() {
        use std::thread;

        let venus = Venus::new();
        let ui_thread = thread::current().id();
        let ran_off_thread = Rc::new(Cell::new(false));

        let venus_for_task = venus.clone();
        let observed = ran_off_thread.clone();
        venus.spawn(async move {
            let worker_thread = venus_for_task
                .spawn_blocking(|| thread::current().id())
                .await;
            observed.set(worker_thread != ui_thread);
        });

        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        assert!(ran_off_thread.get());
    }

    /// A worker finishing while the event loop sleeps has to be able to wake it.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_wake_from_a_worker_notifies_the_event_loop() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let venus = Venus::new();
        let pings = Arc::new(AtomicUsize::new(0));

        let counted = pings.clone();
        venus.set_notifier(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });

        let venus_for_task = venus.clone();
        venus.spawn(async move {
            venus_for_task.offload(|| ()).await;
        });

        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        eprintln!("{}", pings.load(Ordering::SeqCst));
        assert!(pings.load(Ordering::SeqCst) >= 1);
    }
}

#[cfg(test)]
mod scope_teardown {
    //! What tearing down a widget subtree costs the scheduler.
    //!
    //! The framework keeps one [`aimer_venus::TaskScope`] per element and drops it
    //! on unmount, so a route navigation or a closing dialog is a burst of scope
    //! drops — most of them for elements that never spawned a task. Cancellation
    //! therefore has to cost what the *scope* contains, not what the arena has
    //! ever contained: a scheduler that sweeps every slot per drop turns one busy
    //! afternoon of tasks into a permanent teardown tax, quadratic in the size of
    //! the subtree.
    //!
    //! The bound is deliberately loose, in the same spirit as `frame_overhead.rs`:
    //! this is not a benchmark, it is a guard against scope cancellation quietly
    //! becoming linear in the number of slots.

    use std::time::{Duration, Instant};

    use crate::Venus;

    /// The most a subtree's worth of empty-scope drops may cost.
    ///
    /// The guarded regression — a full arena sweep per drop — costs hundreds of
    /// milliseconds at this scale, two orders of magnitude above this ceiling,
    /// while the intended cost is single-digit milliseconds, well below it.
    const CEILING: Duration = Duration::from_millis(100);

    #[test]
    fn dropping_scopes_costs_the_scope_not_the_arena() {
        let venus = Venus::new();

        // One busy burst, long finished: the arena keeps the slots around for
        // reuse, and teardown must not pay for them ever after.
        for _ in 0..20_000 {
            venus.spawn(async {});
        }
        venus.run_microtasks();
        assert_eq!(venus.task_count(), 0);

        // A subtree's worth of elements unmounting, none of which spawned a task.
        let scopes = 20_000;
        let start = Instant::now();
        for _ in 0..scopes {
            drop(venus.scope());
        }
        let elapsed = start.elapsed();

        println!("{scopes} empty scope drops: {elapsed:?}");
        assert!(
            elapsed < CEILING,
            "tearing down {scopes} empty scopes took {elapsed:?} — cancellation is \
             scaling with the arena, not the scope"
        );
    }

    #[test]
    fn a_scope_teardown_cancels_exactly_its_own_tasks() {
        let venus = Venus::new();

        let doomed = venus.scope();
        let kept = venus.scope();

        for _ in 0..8 {
            venus.spawn_in(doomed.id(), async { unreachable!("cancelled") });
            venus.spawn_in(kept.id(), async {});
        }

        drop(doomed);
        assert_eq!(venus.task_count(), 8);

        venus.run_microtasks();
        assert_eq!(venus.task_count(), 0);
    }
}
