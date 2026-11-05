//! Sharing one in-flight operation between many waiters, and initialising a
//! value once.

use std::cell::{OnceCell as StdOnceCell, RefCell};
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll};

use crate::handle::{JoinError, JoinHandle, spawn_with_handle};
use crate::semaphore::Semaphore;
use crate::task::{Phase, ScopeId};
use crate::venus::Venus;
use crate::waiters::Waiters;

enum State<T> {
    Pending,
    Ready(T),
    Cancelled,
}

struct Inner<T> {
    state: RefCell<State<T>>,
    waiters: Waiters,
    /// Owns the work: the last [`Shared`] to go away takes the task with it.
    task: RefCell<Option<JoinHandle<()>>>,
}

/// Lives in the task and reports how it ended, without keeping the state alive.
struct Finish<T>(Weak<Inner<T>>);

impl<T> Finish<T> {
    fn settle(&self, next: State<T>) {
        let Some(inner) = self.0.upgrade() else {
            return;
        };
        {
            let mut state = inner.state.borrow_mut();
            if !matches!(*state, State::Pending) {
                return;
            }
            *state = next;
        }
        inner.waiters.wake_all();
    }

    fn done(self, value: T) {
        self.settle(State::Ready(value));
    }
}

impl<T> Drop for Finish<T> {
    fn drop(&mut self) {
        // A no-op after `done`.
        self.settle(State::Cancelled);
    }
}

/// One operation, run once, whose result any number of tasks can wait for.
///
/// The classic use is de-duplicating loads: several widgets ask for the same
/// URL, one request goes out, and every asker gets the answer. Hand each asker
/// a clone of the `Shared`; awaiting it yields a clone of the output, which is
/// why the output must be [`Clone`] — wrap an expensive value in an `Rc`.
///
/// The work starts when the `Shared` is created, not when first awaited, and is
/// *owned by the clones*: when the last one is dropped the operation is
/// cancelled. Awaiting resolves to `Err(`[`JoinError::Cancelled`]`)` if the task
/// ends early, for instance with its scope.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let requests = Rc::new(Cell::new(0));
///
/// let counted = requests.clone();
/// let image = venus.shared(async move {
///     counted.set(counted.get() + 1); // one fetch
///     Rc::new(vec![0u8; 16])
/// });
///
/// let (a, b) = (image.clone(), image);
/// let sizes = venus.run_until(async move {
///     (a.await.unwrap().len(), b.await.unwrap().len())
/// });
///
/// assert_eq!(sizes, (16, 16));
/// assert_eq!(requests.get(), 1);
/// ```
pub struct Shared<T>(Rc<Inner<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone> Shared<T> {
    /// The output if the operation has finished, without waiting.
    pub fn peek(&self) -> Option<T> {
        match &*self.0.state.borrow() {
            State::Ready(value) => Some(value.clone()),
            _ => None,
        }
    }
}

impl<T> Shared<T> {
    /// Whether the operation has finished or been cancelled.
    pub fn is_finished(&self) -> bool {
        !matches!(*self.0.state.borrow(), State::Pending)
    }

    /// How many handles share this operation.
    pub fn handles(&self) -> usize {
        Rc::strong_count(&self.0)
    }
}

impl<T: Clone> Future for Shared<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &*self.0.state.borrow() {
            State::Ready(value) => Poll::Ready(Ok(value.clone())),
            State::Cancelled => Poll::Ready(Err(JoinError::Cancelled)),
            State::Pending => {
                self.0.waiters.register(cx.waker());
                Poll::Pending
            }
        }
    }
}

impl Venus {
    /// Runs `future` as a microtask and shares its output; see [`Shared`].
    #[inline]
    pub fn shared<F>(&self, future: F) -> Shared<F::Output>
    where
        F: Future + 'static,
        F::Output: Clone + 'static,
    {
        self.shared_in(Phase::Microtask, ScopeId::ROOT, future)
    }

    /// [`shared`](Self::shared) into `phase`, owned by `scope`.
    pub fn shared_in<F>(&self, phase: Phase, scope: ScopeId, future: F) -> Shared<F::Output>
    where
        F: Future + 'static,
        F::Output: Clone + 'static,
    {
        let inner = Rc::new(Inner {
            state: RefCell::new(State::Pending),
            waiters: Waiters::default(),
            task: RefCell::new(None),
        });
        let finish = Finish(Rc::downgrade(&inner));
        let handle = spawn_with_handle(self.scheduler(), phase, scope, async move {
            let value = future.await;
            finish.done(value);
        });
        *inner.task.borrow_mut() = Some(handle);
        Shared(inner)
    }
}

/// [`Venus::shared`] on the runtime installed for this thread; `None` when
/// there is none.
#[inline]
pub fn shared<F>(future: F) -> Option<Shared<F::Output>>
where
    F: Future + 'static,
    F::Output: Clone + 'static,
{
    Venus::current().map(|venus| venus.shared(future))
}

/// A value initialised once, by whichever task gets there first.
///
/// [`get_or_init`](Self::get_or_init) runs the initialiser if the cell is empty
/// and otherwise returns the stored value. If several tasks ask at once, one
/// initialises and the others wait for it — the initialiser runs once, not once
/// per asker. It is cancellation-safe: if the task that was initialising is
/// dropped, the next waiter takes over.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{OnceCell, Venus};
///
/// let venus = Venus::new();
/// let cell = Rc::new(OnceCell::new());
/// let inits = Rc::new(Cell::new(0));
///
/// for _ in 0..3 {
///     let (cell, inits) = (cell.clone(), inits.clone());
///     venus.spawn(async move {
///         cell.get_or_init(|| async {
///             inits.set(inits.get() + 1);
///             "loaded"
///         })
///         .await;
///     });
/// }
/// venus.run_until_stalled();
///
/// assert_eq!(cell.get(), Some(&"loaded"));
/// assert_eq!(inits.get(), 1);
/// ```
pub struct OnceCell<T> {
    cell: StdOnceCell<T>,
    /// Held by the one task that is initialising.
    gate: Semaphore,
}

impl<T> Default for OnceCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> OnceCell<T> {
    /// Creates an empty cell.
    pub fn new() -> Self {
        Self {
            cell: StdOnceCell::new(),
            gate: Semaphore::new(1),
        }
    }

    /// The stored value, if it has been initialised.
    #[inline]
    pub fn get(&self) -> Option<&T> {
        self.cell.get()
    }

    /// Stores `value`, handing it back if the cell is already full.
    #[inline]
    pub fn set(&self, value: T) -> Result<(), T> {
        self.cell.set(value)
    }

    /// Takes the value out, leaving the cell empty.
    #[inline]
    pub fn take(&mut self) -> Option<T> {
        self.cell.take()
    }

    /// Initialises the cell with `init` if it is empty, then returns the value.
    pub async fn get_or_init<F, Fut>(&self, init: F) -> &T
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        if let Some(value) = self.cell.get() {
            return value;
        }
        let _initialising = self.gate.acquire().await;
        // Someone may have finished while this task waited for the gate.
        if let Some(value) = self.cell.get() {
            return value;
        }
        let value = init().await;
        // `set` on the cell directly could have raced in; the first value wins.
        let _ = self.cell.set(value);
        self.cell.get().expect("the cell was filled just above")
    }

    /// Like [`get_or_init`](Self::get_or_init), for an initialiser that can
    /// fail. A failure leaves the cell empty, so a later call tries again.
    pub async fn get_or_try_init<F, Fut, E>(&self, init: F) -> Result<&T, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        if let Some(value) = self.cell.get() {
            return Ok(value);
        }
        let _initialising = self.gate.acquire().await;
        if let Some(value) = self.cell.get() {
            return Ok(value);
        }
        let value = init().await?;
        let _ = self.cell.set(value);
        Ok(self.cell.get().expect("the cell was filled just above"))
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::yield_now;

    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn every_clone_gets_the_one_result() {
        let venus = Venus::new();
        let runs = Rc::new(Cell::new(0));
        let counted = runs.clone();
        let shared = venus.shared(async move {
            yield_now().await;
            counted.set(counted.get() + 1);
            Rc::new(5)
        });

        let clones: Vec<_> = (0..4).map(|_| shared.clone()).collect();
        let all = venus.run_until(crate::join_all(clones));

        assert!(all.iter().all(|value| **value.as_ref().unwrap() == 5));
        assert_eq!(runs.get(), 1);
        assert_eq!(shared.peek().map(|value| *value), Some(5));
    }

    #[test]
    fn the_work_runs_without_anyone_waiting() {
        let venus = Venus::new();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let shared = venus.shared(async move { flag.set(true) });

        venus.run_until_stalled();

        assert!(ran.get());
        assert!(shared.is_finished());
    }

    #[test]
    fn dropping_the_last_clone_cancels_the_operation() {
        let venus = Venus::new();
        let dropped = Rc::new(Cell::new(false));
        let guard = DropFlag(dropped.clone());
        let shared = venus.shared(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let other = shared.clone();
        venus.run_until_stalled();

        drop(shared);
        assert!(!dropped.get(), "a clone still wants the result");
        assert_eq!(other.handles(), 1);

        drop(other);
        assert!(dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_cancelled_operation_resolves_its_waiters_with_an_error() {
        let venus = Venus::new();
        let scope = venus.scope();
        let shared = venus.shared_in(Phase::Microtask, scope.id(), std::future::pending::<u8>());
        let waiting = shared.clone();
        let outcome = venus.spawn_with_handle(async move { waiting.await });
        venus.run_until_stalled();

        drop(scope);

        assert_eq!(
            venus.run_until(async move { outcome.await.unwrap() }),
            Err(JoinError::Cancelled)
        );
        assert!(shared.is_finished());
    }

    #[test]
    fn a_late_waiter_gets_the_stored_result() {
        let venus = Venus::new();
        let shared = venus.shared(async { 9 });
        venus.run_until_stalled();

        assert_eq!(venus.run_until(shared), Ok(9));
    }

    #[test]
    fn the_free_constructor_needs_a_runtime() {
        assert!(shared(async { 1 }).is_none());
    }

    #[test]
    fn once_cell_runs_the_initialiser_once_for_many_askers() {
        let venus = Venus::new();
        let cell = Rc::new(OnceCell::new());
        let inits = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new(0));
        for _ in 0..5 {
            let (cell, inits, seen) = (cell.clone(), inits.clone(), seen.clone());
            venus.spawn(async move {
                let value = cell
                    .get_or_init(|| async {
                        inits.set(inits.get() + 1);
                        yield_now().await;
                        42
                    })
                    .await;
                seen.set(seen.get() + *value);
            });
        }

        venus.run_until_stalled();

        assert_eq!(inits.get(), 1);
        assert_eq!(seen.get(), 5 * 42);
    }

    #[test]
    fn once_cell_hands_over_when_the_initialiser_is_cancelled() {
        let venus = Venus::new();
        let cell = Rc::new(OnceCell::new());
        let scope = venus.scope();
        let first = cell.clone();
        venus.spawn_in(scope.id(), async move {
            first
                .get_or_init(|| async {
                    std::future::pending::<()>().await;
                    1
                })
                .await;
        });
        venus.run_until_stalled();
        let second = cell.clone();
        let result = venus.spawn_with_handle(async move { *second.get_or_init(|| async { 2 }).await });
        venus.run_until_stalled();
        assert!(!result.is_finished(), "waiting behind the first initialiser");

        drop(scope);

        assert_eq!(venus.run_until(async move { result.await.unwrap() }), 2);
    }

    #[test]
    fn a_failed_try_init_leaves_the_cell_empty_for_the_next_try() {
        let venus = Venus::new();
        let cell = Rc::new(OnceCell::<u8>::new());

        let first = {
            let cell = cell.clone();
            venus.run_until(async move {
                cell.get_or_try_init(|| async { Err::<u8, _>("offline") })
                    .await
                    .copied()
            })
        };
        assert_eq!(first, Err("offline"));
        assert!(cell.get().is_none());

        let second = {
            let cell = cell.clone();
            venus.run_until(async move {
                cell.get_or_try_init(|| async { Ok::<u8, &str>(8) }).await.copied()
            })
        };
        assert_eq!(second, Ok(8));
    }

    #[test]
    fn once_cell_set_wins_over_a_later_initialiser() {
        let venus = Venus::new();
        let cell = OnceCell::new();
        assert_eq!(cell.set(1), Ok(()));
        assert_eq!(cell.set(2), Err(2));

        let value = venus.run_until({
            let cell = Rc::new(cell);
            async move { *cell.get_or_init(|| async { 3 }).await }
        });

        assert_eq!(value, 1);
    }
}
