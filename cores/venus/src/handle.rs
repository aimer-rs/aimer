//! A task you can await: [`JoinHandle`].

use std::cell::RefCell;
use std::fmt;
use std::mem;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

use crate::scheduler::LocalScheduler;
use crate::task::{Phase, ScopeId, TaskId};

/// Why a [`JoinHandle`] has no output to give.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JoinError {
    /// The task was dropped before it finished: aborted, or its scope ended.
    Cancelled,
    /// No runtime was installed on the calling thread, so nothing was spawned.
    RuntimeUnavailable,
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "the Venus task was cancelled before it finished",
            Self::RuntimeUnavailable => "no Venus runtime is installed on this thread",
        })
    }
}

impl std::error::Error for JoinError {}

enum Slot<T> {
    /// Running; holds the waker of whoever is awaiting the handle.
    Pending(Option<Waker>),
    Ready(T),
    Cancelled,
    Taken,
}

struct Shared<T> {
    slot: RefCell<Slot<T>>,
}

impl<T> Shared<T> {
    /// Moves a still-pending slot to `next`, then wakes the waiter.
    ///
    /// The waker is woken after the slot is released, so a waiter that runs
    /// inline can read it.
    fn settle(&self, next: Slot<T>) {
        let waiter = {
            let mut slot = self.slot.borrow_mut();
            if !matches!(*slot, Slot::Pending(_)) {
                return;
            }
            match mem::replace(&mut *slot, next) {
                Slot::Pending(waiter) => waiter,
                _ => None,
            }
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

/// Lives inside the spawned task. If the task is dropped without finishing,
/// this is what tells the handle.
struct Settle<T>(Rc<Shared<T>>);

impl<T> Settle<T> {
    fn finish(self, value: T) {
        self.0.settle(Slot::Ready(value));
    }
}

impl<T> Drop for Settle<T> {
    fn drop(&mut self) {
        // A no-op after `finish`: the slot is no longer pending.
        self.0.settle(Slot::Cancelled);
    }
}

/// A handle to a spawned task that resolves to the task's output.
///
/// Created by [`Venus::spawn_with_handle`](crate::Venus::spawn_with_handle). The
/// output needs not be [`Send`].
///
/// **Dropping the handle cancels the task** — the handle *owns* the work, so a
/// widget that stores one cannot leak a request past its own lifetime. Call
/// [`detach`](Self::detach) to let the task run on regardless.
///
/// Awaiting the handle yields `Err(`[`JoinError::Cancelled`]`)` if the task was
/// aborted or its scope ended before it finished.
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
/// let answer = venus.spawn_with_handle(async {
///     yield_now().await;
///     6 * 7
/// });
///
/// let seen = Rc::new(Cell::new(None));
/// let output = seen.clone();
/// venus.spawn(async move { output.set(Some(answer.await)) });
/// venus.run_microtasks();
///
/// assert_eq!(seen.get(), Some(Ok(42)));
/// ```
#[must_use = "dropping a JoinHandle cancels its task; call `detach` to let it run"]
pub struct JoinHandle<T> {
    shared: Rc<Shared<T>>,
    // Weak: a task may hold a handle to itself or to a sibling, and a strong
    // reference would make the scheduler own itself through its own tasks.
    scheduler: Weak<LocalScheduler>,
    task: TaskId,
    detached: bool,
}

impl<T> JoinHandle<T> {
    /// The id of the underlying task.
    #[inline]
    pub fn id(&self) -> TaskId {
        self.task
    }

    /// Cancels the task, returning whether it was still running.
    ///
    /// The handle then resolves to [`JoinError::Cancelled`]. Cancelling a task
    /// that already finished does nothing, and its output is still available.
    pub fn abort(&self) -> bool {
        self.scheduler
            .upgrade()
            .is_some_and(|scheduler| scheduler.abort(self.task))
    }

    /// Whether the task has finished or been cancelled, so awaiting the handle
    /// would resolve at once.
    #[inline]
    pub fn is_finished(&self) -> bool {
        !matches!(*self.shared.slot.borrow(), Slot::Pending(_))
    }

    /// Lets the task keep running after the handle is gone, giving up its
    /// output. The task still ends with its scope.
    #[inline]
    pub fn detach(mut self) {
        self.detached = true;
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.shared.slot.borrow_mut();
        match &mut *slot {
            Slot::Pending(waiter) => {
                if !waiter.as_ref().is_some_and(|known| known.will_wake(cx.waker())) {
                    *waiter = Some(cx.waker().clone());
                }
                Poll::Pending
            }
            Slot::Ready(_) => match mem::replace(&mut *slot, Slot::Taken) {
                Slot::Ready(value) => Poll::Ready(Ok(value)),
                _ => unreachable!("matched Ready above"),
            },
            Slot::Cancelled => {
                *slot = Slot::Taken;
                Poll::Ready(Err(JoinError::Cancelled))
            }
            Slot::Taken => panic!("a JoinHandle was polled after it returned its output"),
        }
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        // Only a task still running needs cancelling. One that has settled is
        // already gone, or about to be.
        if !self.detached && !self.is_finished() {
            self.abort();
        }
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle")
            .field("task", &self.task)
            .field("finished", &self.is_finished())
            .finish()
    }
}

/// Spawns `future` and returns the handle that owns it.
pub(crate) fn spawn_with_handle<F>(
    scheduler: &Rc<LocalScheduler>,
    phase: Phase,
    scope: ScopeId,
    future: F,
) -> JoinHandle<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    let shared = Rc::new(Shared {
        slot: RefCell::new(Slot::Pending(None)),
    });
    let settle = Settle(Rc::clone(&shared));
    let task = scheduler.spawn_in_phase(phase, scope, async move {
        let value = future.await;
        settle.finish(value);
    });
    JoinHandle {
        shared,
        scheduler: Rc::downgrade(scheduler),
        task,
        detached: false,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::{FrameBudget, Venus, join_all, yield_now};

    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    /// Awaits `handle` in a task of its own and returns where the result lands.
    fn watch<T: 'static>(
        venus: &Venus,
        handle: JoinHandle<T>,
    ) -> Rc<RefCell<Option<Result<T, JoinError>>>> {
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        venus.spawn(async move { *output.borrow_mut() = Some(handle.await) });
        slot
    }

    #[test]
    fn a_handle_resolves_to_the_tasks_output() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(async {
            yield_now().await;
            "done"
        });

        let result = watch(&venus, handle);
        venus.run_microtasks();

        assert_eq!(*result.borrow(), Some(Ok("done")));
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn the_output_may_be_non_send() {
        let venus = Venus::new();
        let shared = Rc::new(Cell::new(1));
        let held = shared.clone();
        let handle = venus.spawn_with_handle(async move {
            held.set(2);
            held
        });

        let result = watch(&venus, handle);
        venus.run_microtasks();

        let output = result.borrow_mut().take().expect("finished").expect("not cancelled");
        assert_eq!(output.get(), 2);
        assert!(Rc::ptr_eq(&output, &shared));
    }

    #[test]
    fn awaiting_after_the_task_already_finished_is_ready_at_once() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(async { 5 });
        venus.run_microtasks();
        assert!(handle.is_finished());

        let result = watch(&venus, handle);
        venus.run_microtasks();

        assert_eq!(*result.borrow(), Some(Ok(5)));
    }

    #[test]
    fn dropping_a_handle_cancels_its_task() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let ran = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        let flag = ran.clone();
        let handle = venus.spawn_with_handle(async move {
            let _guard = guard;
            yield_now().await;
            flag.set(true);
        });
        assert_eq!(venus.task_count(), 1);

        drop(handle);

        assert!(dropped.get(), "the future was dropped with the handle");
        assert_eq!(venus.task_count(), 0);
        venus.run_microtasks();
        assert!(!ran.get());
    }

    #[test]
    fn a_detached_task_runs_on_without_its_handle() {
        let venus = Venus::new();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let handle = venus.spawn_with_handle(async move {
            yield_now().await;
            flag.set(true);
        });

        handle.detach();
        venus.run_microtasks();

        assert!(ran.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn aborting_resolves_the_handle_as_cancelled() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(std::future::pending::<u8>());
        venus.run_microtasks();

        assert!(handle.abort());
        assert!(!handle.abort(), "already gone");
        assert!(handle.is_finished());
        let result = watch(&venus, handle);
        venus.run_microtasks();

        assert_eq!(*result.borrow(), Some(Err(JoinError::Cancelled)));
    }

    #[test]
    fn a_waiter_parked_on_the_handle_wakes_when_the_task_is_aborted() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(std::future::pending::<u8>());
        let id = handle.id();
        let keep = Rc::new(RefCell::new(None));
        let output = keep.clone();
        let sibling = venus.clone();
        // Awaited first, so the waiter is parked when the abort happens.
        let waiter = venus.spawn(async move {
            let outcome = handle.await;
            *output.borrow_mut() = Some(outcome);
        });
        venus.run_microtasks();
        assert!(keep.borrow().is_none());

        // Aborted from outside, by id, leaving the handle where it is.
        assert!(sibling.abort(id));
        assert!(venus.has_ready_work(), "the parked waiter was woken");
        venus.run_microtasks();

        assert_eq!(*keep.borrow(), Some(Err(JoinError::Cancelled)));
        assert!(!venus.is_running(waiter));
    }

    #[test]
    fn ending_the_tasks_scope_cancels_the_handle() {
        let venus = Venus::new();
        let scope = venus.scope();
        let handle = venus.spawn_with_handle_in(scope.id(), std::future::pending::<u8>());
        let result = watch(&venus, handle);
        venus.run_microtasks();
        assert!(result.borrow().is_none());

        drop(scope);
        venus.run_microtasks();

        assert_eq!(*result.borrow(), Some(Err(JoinError::Cancelled)));
    }

    #[test]
    fn is_finished_tracks_the_task() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(async {
            yield_now().await;
        });
        assert!(!handle.is_finished());

        venus.run_microtasks();

        assert!(handle.is_finished());
    }

    #[test]
    fn a_handle_can_run_its_task_in_another_phase() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle_in_phase(Phase::Idle, ScopeId::ROOT, async { 9 });
        let result = watch(&venus, handle);

        venus.run_microtasks();
        assert!(result.borrow().is_none(), "idle work waits for room");

        venus.run_idle(&FrameBudget::from_now(std::time::Duration::from_millis(4)));
        venus.run_microtasks();

        assert_eq!(*result.borrow(), Some(Ok(9)));
    }

    #[test]
    fn handles_compose_with_join_all() {
        let venus = Venus::new();
        let handles: Vec<_> = (0..4u32)
            .map(|n| {
                venus.spawn_with_handle(async move {
                    for _ in 0..(4 - n) {
                        yield_now().await;
                    }
                    n * 10
                })
            })
            .collect();
        let result = Rc::new(RefCell::new(Vec::new()));
        let output = result.clone();
        venus.spawn(async move {
            *output.borrow_mut() = join_all(handles).await;
        });

        venus.run_microtasks();

        let values: Vec<_> = result.borrow().iter().map(|r| r.unwrap()).collect();
        assert_eq!(values, [0, 10, 20, 30]);
    }

    // The scheduler used to drop an aborted task while it was still borrowed, so
    // a future holding a handle would abort its child into a re-entrant borrow
    // and panic.
    #[test]
    fn aborting_a_task_that_owns_a_handle_cancels_the_child_too() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        let child = venus.spawn_with_handle(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let parent = venus.spawn(async move {
            let _child = child;
            std::future::pending::<()>().await;
        });
        venus.run_microtasks();
        assert_eq!(venus.task_count(), 2);

        assert!(venus.abort(parent));

        assert!(dropped.get(), "the child went with its parent");
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn ending_a_scope_whose_task_owns_a_handle_does_not_re_enter_the_scheduler() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        let child = venus.spawn_with_handle(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let scope = venus.scope();
        venus.spawn_in(scope.id(), async move {
            let _child = child;
            std::future::pending::<()>().await;
        });
        venus.run_microtasks();

        drop(scope);

        assert!(dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_task_that_cancels_itself_while_running_may_own_a_handle() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        let child = venus.spawn_with_handle(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let scope = venus.scope();
        // Dropped while the task is being polled, so the scheduler has to drop
        // its future once it yields, outside its own borrow.
        let slot = Rc::new(RefCell::new(Some(scope)));
        let own_scope = slot.clone();
        venus.spawn_in(slot.borrow().as_ref().unwrap().id(), async move {
            let _child = child;
            own_scope.borrow_mut().take();
            yield_now().await;
        });

        venus.run_microtasks();

        assert!(dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_handle_outliving_its_runtime_drops_quietly() {
        let venus = Venus::new();
        let handle = venus.spawn_with_handle(std::future::pending::<()>());

        drop(venus);

        assert!(!handle.abort());
        drop(handle);
    }

    #[test]
    fn the_free_function_needs_an_installed_runtime() {
        assert!(crate::spawn_with_handle(async {}).is_none());

        let venus = Venus::new();
        venus.install();
        let handle = crate::spawn_with_handle(async { 3 }).expect("a runtime is installed");
        Venus::uninstall();

        let result = watch(&venus, handle);
        venus.run_microtasks();
        assert_eq!(*result.borrow(), Some(Ok(3)));
    }
}
