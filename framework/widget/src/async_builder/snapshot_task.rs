use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::Duration;

use aimer_venus::{TaskId, TaskScope, TimeoutError, Venus};

use super::AsyncSnapshot;

/// An operation failure or a runtime error reported by a [`SnapshotTask`].
#[derive(Debug, Eq, PartialEq)]
pub enum AsyncSnapshotError<E> {
    /// The future returned its own typed error.
    Future(E),
    /// The deadline expired, or the host could not provide a timer.
    Timeout(TimeoutError),
    /// No Venus runtime was installed on this thread.
    RuntimeUnavailable,
}

impl<E: fmt::Display> fmt::Display for AsyncSnapshotError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Future(error) => error.fmt(f),
            Self::Timeout(error) => error.fmt(f),
            Self::RuntimeUnavailable => f.write_str("no Venus runtime is installed on this thread"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for AsyncSnapshotError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Future(error) => Some(error),
            Self::Timeout(error) => Some(error),
            Self::RuntimeUnavailable => None,
        }
    }
}

struct SnapshotState<T, E> {
    snapshot: RefCell<AsyncSnapshot<T, AsyncSnapshotError<E>>>,
    task: Cell<Option<TaskId>>,
    timeout_task: Cell<Option<TaskId>>,
    runtime: Weak<Venus>,
    // The task holds only a Weak to this state: the last handle owns cancellation.
    scope: Option<TaskScope>,
}

impl<T, E> SnapshotState<T, E> {
    fn complete(&self, snapshot: AsyncSnapshot<T, AsyncSnapshotError<E>>) {
        self.task.set(None);
        if let Some(timer) = self.timeout_task.take()
            && let Some(runtime) = self.runtime.upgrade()
        {
            runtime.abort(timer);
        }
        *self.snapshot.borrow_mut() = snapshot;
    }
}

/// A live snapshot of one future running on the UI thread.
///
/// Created by [`FutureSnapshotExt::waitme`], independently of a widget builder.
/// Clones share the same operation without cloning its result. Dropping the
/// last handle cancels the operation and drops the future. Neither the future
/// nor its data or error needs to be [`Send`].
#[must_use = "dropping the last snapshot handle cancels its future"]
pub struct SnapshotTask<T, E> {
    state: Rc<SnapshotState<T, E>>,
}

impl<T, E> Clone for SnapshotTask<T, E> {
    #[inline]
    fn clone(&self) -> Self {
        Self { state: self.state.clone() }
    }
}

impl<T, E> SnapshotTask<T, E> {
    /// Borrows the current waiting, data, or error snapshot without cloning it.
    ///
    /// This is a synchronous read; it does not subscribe a widget to rebuilds.
    /// Retain the handle to keep the operation alive and read it again after
    /// Venus runs. Release the borrow before driving the runtime or awaiting:
    /// completion needs a mutable borrow of the snapshot.
    #[inline]
    pub fn read(&self) -> Ref<'_, AsyncSnapshot<T, AsyncSnapshotError<E>>> {
        self.state.snapshot.borrow()
    }
}

impl<T: 'static, E: 'static> SnapshotTask<T, E> {
    /// Sets or replaces the deadline of this operation, starting now.
    ///
    /// All clones observe the same deadline and result. Expiry drops the
    /// pending future and publishes [`AsyncSnapshotError::Timeout`]. A host
    /// without timers reports [`TimeoutError::TimerUnavailable`] instead.
    /// Calling this after completion leaves the snapshot unchanged.
    ///
    /// The deadline is cooperative and cannot interrupt blocking work in one
    /// poll. Dropping the last handle cancels both the future and its timer.
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use aimer_widget::FutureSnapshotExt;
    ///
    /// let request = async { Ok::<_, String>(42) }
    ///     .waitme()
    ///     .timeout(Duration::from_secs(5));
    /// ```
    pub fn timeout(self, duration: Duration) -> Self {
        if self.state.task.get().is_none() {
            return self;
        }
        let Some(runtime) = self.state.runtime.upgrade() else {
            self.state.complete(AsyncSnapshot::Error(AsyncSnapshotError::RuntimeUnavailable));
            return self;
        };
        if let Some(timer) = self.state.timeout_task.take() {
            runtime.abort(timer);
        }
        let deadline = runtime.timeout(duration, std::future::pending::<()>());
        let target = Rc::downgrade(&self.state);
        let scope = self.state.scope.as_ref().expect("a running task owns a scope").id();
        let runtime_ref = Rc::downgrade(&runtime);
        let timer = runtime.spawn_in(scope, async move {
            let error = deadline.await.expect_err("a pending future cannot complete");
            if let Some(state) = target.upgrade() {
                // The timer is currently running; completion must not abort it.
                state.timeout_task.set(None);
                if let Some(task) = state.task.take()
                    && let Some(runtime) = runtime_ref.upgrade()
                {
                    runtime.abort(task);
                }
                state.complete(AsyncSnapshot::Error(AsyncSnapshotError::Timeout(error)));
            }
        });
        self.state.timeout_task.set(Some(timer));
        self
    }
}

/// Runs result-returning futures on Venus and exposes their live snapshots.
///
/// Import this trait to call `.waitme()` on any owned, `'static` future whose
/// output is `Result<T, E>`. The future is moved into Venus immediately and is
/// first polled in its microtask phase, before the next build.
///
/// ```
/// use aimer_venus::Venus;
/// use aimer_widget::{AsyncSnapshot, FutureSnapshotExt};
///
/// let venus = Venus::new();
/// venus.install();
/// let request = async { Ok::<_, &'static str>(42) }.waitme();
/// assert!(matches!(&*request.read(), AsyncSnapshot::Waiting));
/// venus.run_microtasks();
/// assert!(matches!(&*request.read(), AsyncSnapshot::Data(42)));
/// Venus::uninstall();
/// ```
pub trait FutureSnapshotExt<T: 'static, E: 'static>:
    Future<Output = Result<T, E>> + Sized + 'static
{
    /// Moves this future into the runtime installed on the calling thread.
    ///
    /// Returns immediately with a waiting snapshot. If no runtime is installed,
    /// the future is dropped without polling and the snapshot contains
    /// [`AsyncSnapshotError::RuntimeUnavailable`]. Errors from the operation are
    /// preserved inside [`AsyncSnapshotError::Future`].
    fn waitme(self) -> SnapshotTask<T, E> {
        let runtime = Venus::current();
        let scope = runtime.as_ref().map(|runtime| runtime.scope());
        let state = Rc::new(SnapshotState {
            snapshot: RefCell::new(if runtime.is_some() {
                AsyncSnapshot::Waiting
            } else {
                AsyncSnapshot::Error(AsyncSnapshotError::RuntimeUnavailable)
            }),
            task: Cell::new(None),
            timeout_task: Cell::new(None),
            runtime: runtime.as_ref().map_or_else(Weak::new, Rc::downgrade),
            scope,
        });
        if let Some(runtime) = runtime {
            let target = Rc::downgrade(&state);
            let scope = state.scope.as_ref().expect("a runtime owns a task scope").id();
            state.task.set(Some(runtime.spawn_in(scope, async move {
                let result = self.await;
                if let Some(state) = target.upgrade() {
                    state.complete(match result {
                        Ok(value) => AsyncSnapshot::Data(value),
                        Err(error) => AsyncSnapshot::Error(AsyncSnapshotError::Future(error)),
                    });
                }
            })));
        }
        SnapshotTask { state }
    }
}

impl<F, T: 'static, E: 'static> FutureSnapshotExt<T, E> for F
where
    F: Future<Output = Result<T, E>> + 'static,
{}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use aimer_venus::{PollContext, TimeoutError, Venus};

    use crate::{AsyncSnapshot, AsyncSnapshotError, FutureSnapshotExt};

    struct ExpiredTimer;

    impl PollContext for ExpiredTimer {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }

        fn sleep(&self, duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
            assert_eq!(duration, Duration::ZERO);
            Some(Box::pin(std::future::ready(())))
        }
    }

    struct DropGuard(Rc<Cell<usize>>);

    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[derive(Default)]
    struct Clock {
        expired: Cell<bool>,
        waker: RefCell<Option<Waker>>,
        drops: Cell<usize>,
    }

    impl Clock {
        fn expire(&self) {
            self.expired.set(true);
            let waker = self.waker.borrow_mut().take();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    struct Timer(Rc<Clock>);

    impl Future for Timer {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0.expired.get() {
                Poll::Ready(())
            } else {
                self.0.waker.borrow_mut().replace(cx.waker().clone());
                Poll::Pending
            }
        }
    }

    impl Drop for Timer {
        fn drop(&mut self) {
            self.0.drops.set(self.0.drops.get() + 1);
        }
    }

    #[derive(Clone, Default)]
    struct Timers(Rc<RefCell<Vec<Rc<Clock>>>>);

    impl PollContext for Timers {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }

        fn sleep(&self, _duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
            let clock = Rc::new(Clock::default());
            self.0.borrow_mut().push(clock.clone());
            Some(Box::pin(Timer(clock)))
        }
    }

    #[test]
    fn waitme_runs_a_non_send_future_and_exposes_its_live_snapshot() {
        let venus = Venus::new();
        venus.install();
        let value = Rc::new(Cell::new(41));
        let request = async move { Ok::<_, &'static str>(value.get() + 1) }.waitme();

        assert!(matches!(&*request.read(), AsyncSnapshot::Waiting));
        assert_eq!(venus.task_count(), 1);
        venus.run_microtasks();

        assert!(matches!(&*request.read(), AsyncSnapshot::Data(42)));
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_timeout_drops_the_pending_future_and_publishes_an_error() {
        let venus = Venus::new();
        venus.install();
        venus.set_poll_context(ExpiredTimer);
        let drops = Rc::new(Cell::new(0));
        let guard = DropGuard(drops.clone());
        let request = async move {
            let _guard = guard;
            std::future::pending::<Result<usize, &'static str>>().await
        }.waitme().timeout(Duration::ZERO);

        venus.run_microtasks();

        assert!(matches!(
            &*request.read(),
            AsyncSnapshot::Error(AsyncSnapshotError::Timeout(TimeoutError::Elapsed))
        ));
        assert_eq!(drops.get(), 1);
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_preserves_non_send_typed_errors() {
        let venus = Venus::new();
        venus.install();
        let error = Rc::new(Cell::new(7));
        let output = error.clone();
        let request = async move { Err::<(), _>(output) }.waitme();
        venus.run_microtasks();

        match &*request.read() {
            AsyncSnapshot::Error(AsyncSnapshotError::Future(value)) => {
                assert!(Rc::ptr_eq(value, &error));
            }
            _ => panic!("the original error must be retained"),
        }
        Venus::uninstall();
    }

    #[test]
    fn waitme_without_a_runtime_drops_the_future_without_polling() {
        Venus::uninstall();
        let drops = Rc::new(Cell::new(0));
        let polled = Rc::new(Cell::new(false));
        let guard = DropGuard(drops.clone());
        let flag = polled.clone();
        let request = async move {
            let _guard = guard;
            flag.set(true);
            Ok::<_, ()>(42)
        }.waitme();

        assert!(matches!(
            &*request.read(),
            AsyncSnapshot::Error(AsyncSnapshotError::RuntimeUnavailable)
        ));
        assert!(!polled.get());
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn waitme_clones_keep_the_future_alive_until_the_last_handle_drops() {
        let venus = Venus::new();
        venus.install();
        let drops = Rc::new(Cell::new(0));
        let guard = DropGuard(drops.clone());
        let request = async move {
            let _guard = guard;
            std::future::pending::<Result<Rc<Cell<usize>>, Rc<Cell<usize>>>>().await
        }.waitme();
        let retained = request.clone();
        venus.run_microtasks();

        drop(request);
        assert_eq!(venus.task_count(), 1);
        assert_eq!(drops.get(), 0);
        drop(retained);
        assert_eq!(venus.task_count(), 0);
        assert_eq!(drops.get(), 1);
        Venus::uninstall();
    }

    #[test]
    fn waitme_can_cancel_before_the_first_poll() {
        let venus = Venus::new();
        venus.install();
        let drops = Rc::new(Cell::new(0));
        let guard = DropGuard(drops.clone());
        let request = async move {
            let _guard = guard;
            std::future::pending::<Result<(), ()>>().await
        }.waitme();

        drop(request);

        assert_eq!(venus.run_microtasks(), 0);
        assert_eq!(drops.get(), 1);
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_completion_cancels_its_timer_and_cannot_be_overwritten() {
        let venus = Venus::new();
        venus.install();
        let timers = Timers::default();
        venus.set_poll_context(timers.clone());
        let request = async { Ok::<_, ()>(42) }.waitme().timeout(Duration::from_secs(1));
        let clock = timers.0.borrow()[0].clone();
        venus.run_microtasks();

        assert!(matches!(&*request.read(), AsyncSnapshot::Data(42)));
        assert_eq!(clock.drops.get(), 1);
        clock.expire();
        venus.run_microtasks();
        let request = request.timeout(Duration::ZERO);
        assert!(matches!(&*request.read(), AsyncSnapshot::Data(42)));
        assert_eq!(timers.0.borrow().len(), 1, "settled tasks need no new timer");
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_replaces_deadlines_and_wakes_a_parked_task_on_expiry() {
        let venus = Venus::new();
        venus.install();
        let timers = Timers::default();
        venus.set_poll_context(timers.clone());
        let request = std::future::pending::<Result<(), ()>>().waitme()
            .timeout(Duration::from_secs(1));
        venus.run_microtasks();
        assert!(!venus.has_ready_work());
        let old = timers.0.borrow()[0].clone();
        let request = request.timeout(Duration::from_secs(2));
        let current = timers.0.borrow()[1].clone();
        assert_eq!(old.drops.get(), 1);
        old.expire();
        venus.run_microtasks();
        assert!(matches!(&*request.read(), AsyncSnapshot::Waiting));
        assert!(!venus.has_ready_work());

        current.expire();
        venus.run_microtasks();

        assert!(matches!(
            &*request.read(),
            AsyncSnapshot::Error(AsyncSnapshotError::Timeout(TimeoutError::Elapsed))
        ));
        assert_eq!(current.drops.get(), 1);
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_dropping_the_last_handle_cancels_its_timer_too() {
        let venus = Venus::new();
        venus.install();
        let timers = Timers::default();
        venus.set_poll_context(timers.clone());
        let request = std::future::pending::<Result<(), ()>>().waitme()
            .timeout(Duration::from_secs(1));
        venus.run_microtasks();

        drop(request);

        assert_eq!(timers.0.borrow()[0].drops.get(), 1);
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }

    #[test]
    fn waitme_reports_a_host_without_timer_support() {
        let venus = Venus::new();
        venus.install();
        let request = std::future::pending::<Result<(), ()>>().waitme().timeout(Duration::ZERO);
        venus.run_microtasks();

        assert!(matches!(
            &*request.read(),
            AsyncSnapshot::Error(AsyncSnapshotError::Timeout(TimeoutError::TimerUnavailable))
        ));
        assert_eq!(venus.task_count(), 0);
        Venus::uninstall();
    }
}
