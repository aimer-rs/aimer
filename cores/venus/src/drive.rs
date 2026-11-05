//! Driving a runtime from outside an event loop: tests, tools and headless code.
//!
//! An event loop runs the phases itself; code that has no loop still needs a way
//! to say "run until this is done". These methods are that, and they are a
//! blocking convenience for the thread that owns the runtime — never call them
//! from inside a task.

use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use aimer_time::Instant;

use crate::budget::FrameBudget;
use crate::venus::Venus;

/// How long an idle drive sleeps between looks for a wake from another thread.
const IDLE_POLL: Duration = Duration::from_micros(200);

/// How long [`Venus::run_until`] waits before deciding the future is stuck.
///
/// Generous on purpose: this only has to turn a hang into a failure, never to
/// be the thing that ends a healthy run.
const RUN_UNTIL_LIMIT: Duration = Duration::from_secs(10);

/// How many passes [`Venus::run_until_stalled`] makes before concluding that
/// something re-arms itself forever.
const STALL_ROUNDS: usize = 10_000;

impl Venus {
    /// Runs every phase once: frame tasks, then microtasks, then idle tasks
    /// with a frame's worth of room. Returns how many polls it got through.
    fn drive_round(&self) -> usize {
        let frame = self.run_frame_tasks();
        let micro = self.run_microtasks();
        let idle = self.run_idle(&FrameBudget::from_now(self.frame_time()));
        frame + micro + idle
    }

    /// Runs tasks until nothing is ready, returning how many polls it took.
    ///
    /// Work parked on something that has not happened — a timer that has not
    /// fired, a worker still running — is left parked; this does not wait for
    /// it. All three phases are driven, so idle work gets its room.
    ///
    /// # Panics
    ///
    /// Panics if the runtime never stalls, which means a task re-arms itself
    /// forever (a frame task that yields in a loop, for instance).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_venus::{Venus, yield_now};
    ///
    /// let venus = Venus::new();
    /// let steps = Rc::new(Cell::new(0));
    ///
    /// let counted = steps.clone();
    /// venus.spawn(async move {
    ///     yield_now().await;
    ///     counted.set(counted.get() + 1);
    /// });
    /// venus.spawn_idle(async {});
    ///
    /// venus.run_until_stalled();
    /// assert_eq!(steps.get(), 1);
    /// assert_eq!(venus.task_count(), 0);
    /// ```
    pub fn run_until_stalled(&self) -> usize {
        let mut polled = 0;
        for _ in 0..STALL_ROUNDS {
            polled += self.drive_round();
            if !self.has_ready_work() {
                return polled;
            }
        }
        panic!("the Venus runtime never stalled: a task re-arms itself every pass");
    }

    /// Drives the runtime until `future` completes, returning its output.
    ///
    /// The future is spawned as a task, so it may hold non-`Send` values and
    /// await anything a task can: offloaded work, host timers, other tasks. If
    /// nothing is ready the thread sleeps briefly and looks again, which is how
    /// a result from a worker thread gets picked up.
    ///
    /// # Panics
    ///
    /// Panics if the future has not finished after ten seconds, so a stuck test
    /// fails instead of hanging. Use [`run_until_for`](Self::run_until_for) to
    /// choose the limit or to handle it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_venus::{Venus, yield_now};
    ///
    /// let venus = Venus::new();
    /// let value = venus.run_until(async {
    ///     yield_now().await;
    ///     6 * 7
    /// });
    /// assert_eq!(value, 42);
    /// ```
    pub fn run_until<F>(&self, future: F) -> F::Output
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        self.run_until_for(future, RUN_UNTIL_LIMIT)
            .expect("the future did not finish within the run_until limit")
    }

    /// Drives the runtime until `future` completes or `limit` has passed.
    ///
    /// Returns `None` on timeout, after cancelling the future: it is dropped,
    /// not left running.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_venus::Venus;
    ///
    /// let venus = Venus::new();
    /// let stuck = venus.run_until_for(std::future::pending::<()>(), Duration::from_millis(5));
    /// assert!(stuck.is_none());
    /// assert_eq!(venus.task_count(), 0, "and the stuck future was cancelled");
    /// ```
    pub fn run_until_for<F>(&self, future: F, limit: Duration) -> Option<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        // The handle owns the task: returning early drops it and cancels.
        let handle = self.spawn_with_handle(future);
        let deadline = Instant::now().checked_add(limit);
        loop {
            self.drive_round();
            if handle.is_finished() {
                break;
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return None;
            }
            if !self.has_ready_work() {
                thread::sleep(IDLE_POLL);
            }
        }

        // Finished, so this poll resolves at once and needs no real waker.
        let mut handle = pin!(handle);
        match handle.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(Ok(output)) => Some(output),
            Poll::Ready(Err(error)) => panic!("the future was cancelled: {error}"),
            Poll::Pending => unreachable!("a finished handle resolves on its first poll"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::test_host::with_timers;
    use crate::{Phase, ScopeId, spawn_with_handle, yield_now};

    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn run_until_returns_the_output_of_a_non_send_future() {
        let venus = Venus::new();
        let shared = Rc::new(Cell::new(1));

        let held = shared.clone();
        let output = venus.run_until(async move {
            yield_now().await;
            held.set(2);
            held
        });

        assert!(Rc::ptr_eq(&output, &shared));
        assert_eq!(shared.get(), 2);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn run_until_waits_for_a_worker_thread() {
        let venus = Venus::new();
        let runtime = venus.clone();

        let sum = venus.run_until(async move {
            runtime
                .spawn_blocking(|| {
                    std::thread::sleep(Duration::from_millis(20));
                    (1..=10).sum::<u32>()
                })
                .await
        });

        assert_eq!(sum, 55);
    }

    #[test]
    fn run_until_drives_idle_work_the_future_depends_on() {
        let venus = Venus::new();
        let runtime = venus.clone();

        let value = venus.run_until(async move {
            let idle = runtime.spawn_with_handle_in_phase(Phase::Idle, ScopeId::ROOT, async { 9 });
            idle.await.expect("not cancelled")
        });

        assert_eq!(value, 9);
    }

    #[test]
    fn run_until_drives_frame_tasks_the_future_depends_on() {
        let venus = Venus::new();
        let ticks = Rc::new(Cell::new(0));

        let counted = ticks.clone();
        let runtime = venus.clone();
        let seen = venus.run_until(async move {
            let ticker = runtime.spawn_with_handle_in_phase(Phase::Frame, ScopeId::ROOT, {
                let counted = counted.clone();
                async move {
                    for _ in 0..3 {
                        counted.set(counted.get() + 1);
                        yield_now().await;
                    }
                }
            });
            ticker.await.expect("not cancelled");
            counted.get()
        });

        assert_eq!(seen, 3);
    }

    #[test]
    fn run_until_for_cancels_a_future_that_never_finishes() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());

        let outcome = venus.run_until_for(
            async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            },
            Duration::from_millis(10),
        );

        assert!(outcome.is_none());
        assert!(dropped.get(), "the stuck future was dropped");
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn run_until_can_wait_for_a_host_timer_that_fires_from_the_test() {
        let (venus, timers) = with_timers();
        let sleep = venus.sleep(Duration::from_secs(60));
        let fired = Rc::new(RefCell::new(false));

        // Not fired: the drive gives up, and says so.
        let gave_up = venus.run_until_for(async move { sleep.await }, Duration::from_millis(5));
        assert!(gave_up.is_none());

        let sleep = venus.sleep(Duration::from_secs(60));
        timers.fire(1);
        let flag = fired.clone();
        let outcome = venus.run_until(async move {
            let result = sleep.await;
            *flag.borrow_mut() = true;
            result
        });

        assert_eq!(outcome, Ok(()));
        assert!(*fired.borrow());
    }

    #[test]
    fn run_until_stalled_runs_every_phase_then_stops() {
        let venus = Venus::new();
        let order = Rc::new(RefCell::new(Vec::new()));
        for (phase, name) in [(Phase::Idle, "idle"), (Phase::Microtask, "micro"), (Phase::Frame, "frame")] {
            let log = order.clone();
            venus.spawn_in_phase(phase, ScopeId::ROOT, async move { log.borrow_mut().push(name) });
        }

        let polled = venus.run_until_stalled();

        assert_eq!(polled, 3);
        assert_eq!(*order.borrow(), ["frame", "micro", "idle"]);
        assert!(!venus.has_ready_work());
    }

    #[test]
    fn run_until_stalled_leaves_parked_work_parked() {
        let (venus, _timers) = with_timers();
        let sleep = venus.sleep(Duration::from_secs(60));
        venus.spawn(async move {
            let _ = sleep.await;
        });

        venus.run_until_stalled();

        assert_eq!(venus.task_count(), 1, "still waiting on its timer");
    }

    #[test]
    #[should_panic(expected = "never stalled")]
    fn run_until_stalled_reports_a_task_that_rearms_itself_forever() {
        let venus = Venus::new();
        venus.spawn_frame(async {
            loop {
                yield_now().await;
            }
        });

        venus.run_until_stalled();
    }

    #[test]
    fn a_spawn_with_handle_free_function_future_can_be_driven_to_completion() {
        let venus = Venus::new();
        venus.install();

        let value = venus.run_until(async {
            spawn_with_handle(async { 4 })
                .expect("a runtime is installed")
                .await
                .expect("not cancelled")
        });

        Venus::uninstall();
        assert_eq!(value, 4);
    }
}
