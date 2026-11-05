use std::cell::{Ref, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::Duration;

use aimer_venus::{JoinHandle, TimeoutError, Venus};

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
    /// The task driving the future. The handles own their tasks, so dropping
    /// this state — the last [`SnapshotTask`] going away — cancels them. The
    /// tasks themselves hold only a `Weak` to the state.
    task: RefCell<Option<JoinHandle<()>>>,
    timer: RefCell<Option<JoinHandle<()>>>,
    runtime: Weak<Venus>,
    /// Runs after every completion, once the snapshot has been written and its
    /// borrow released, so it may read the snapshot.
    wake: Option<Box<dyn Fn()>>,
}

impl<T, E> SnapshotState<T, E> {
    fn complete(&self, snapshot: AsyncSnapshot<T, AsyncSnapshotError<E>>) {
        // Whichever task is completing has already detached its own handle, so
        // what is dropped here is the other one, which is cancelled.
        drop(self.task.take());
        drop(self.timer.take());
        *self.snapshot.borrow_mut() = snapshot;
        if let Some(wake) = &self.wake {
            wake();
        }
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
    /// use aimer_async_builder::FutureSnapshotExt;
    ///
    /// let request = async { Ok::<_, String>(42) }
    ///     .waitme()
    ///     .timeout(Duration::from_secs(5));
    /// ```
    pub fn timeout(self, duration: Duration) -> Self {
        if self.state.task.borrow().is_none() {
            return self;
        }
        let Some(runtime) = self.state.runtime.upgrade() else {
            self.state.complete(AsyncSnapshot::Error(AsyncSnapshotError::RuntimeUnavailable));
            return self;
        };
        let deadline = runtime.timeout(duration, std::future::pending::<()>());
        let target = Rc::downgrade(&self.state);
        let timer = runtime.spawn_with_handle(async move {
            let error = deadline.await.expect_err("a pending future cannot complete");
            if let Some(state) = target.upgrade() {
                // This timer is running: completion must not cancel it.
                if let Some(timer) = state.timer.take() {
                    timer.detach();
                }
                // Dropping the handle cancels the future.
                drop(state.task.take());
                state.complete(AsyncSnapshot::Error(AsyncSnapshotError::Timeout(error)));
            }
        });
        // Replacing the previous deadline cancels it.
        let previous = self.state.timer.replace(Some(timer));
        drop(previous);
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
/// use aimer_async_builder::{AsyncSnapshot, FutureSnapshotExt};
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
        spawn_snapshot(self, None)
    }

    /// Runs this future once for the widget being built and returns its live
    /// snapshot, rebuilding that widget when the snapshot changes.
    ///
    /// Call it from `build`. The first build that asks for `key` moves the
    /// future into Venus; every later build of the same widget that asks for
    /// the same `key` gets the same [`SnapshotTask`] back and drops the new
    /// future unpolled. When the future finishes, the widget is marked dirty
    /// and a frame is requested, so the next build reads the result.
    ///
    /// # The key is the dependency list
    ///
    /// Like [`BuildContext::retain`], the key says what the future depends on.
    /// Put whatever the future reads into it: when it changes, the previous
    /// future is cancelled and a new one starts. A key a build stops asking for
    /// cancels its future, as does unmounting the widget.
    ///
    /// A key identifies a future within one widget, not across the tree. Using
    /// the same key twice in one build returns the same task for both calls.
    /// Reusing a key for a different `T` or `E` replaces the earlier future.
    ///
    /// `watch` runs on every build, so prefer a key that is free to make: an
    /// integer, `("tag", integer)`, or [`Key::Static`](aimer_widget_core::Key::Static). A
    /// key made from a `&str` or a `String` allocates on each build.
    ///
    /// # Outside a build
    ///
    /// Nothing is retained and nothing is rebuilt: the call behaves like
    /// [`FutureSnapshotExt::waitme`], and panics in debug builds, because a
    /// widget that never rebuilds is a bug rather than a use.
    ///
    /// ```ignore
    /// fn build(&self, _ctx: &BuildContext) -> impl Widget {
    ///     let user = fetch_user(self.id).watch(("user", self.id));
    ///     match &*user.read() {
    ///         AsyncSnapshot::Waiting => Spinner::new().boxed(),
    ///         AsyncSnapshot::Data(user) => Profile::new(user).boxed(),
    ///         AsyncSnapshot::Error(_) => ErrorView::new().boxed(),
    ///     }
    /// }
    /// ```
    #[track_caller]
    fn watch(self, key: impl Into<aimer_widget_core::Key>) -> SnapshotTask<T, E> {
        super::watch::watch_future(self, key.into())
    }
}

/// Moves `future` into the installed runtime and returns its live snapshot.
///
/// `wake` runs after every completion of the snapshot, on the UI thread.
pub(super) fn spawn_snapshot<F, T: 'static, E: 'static>(
    future: F,
    wake: Option<Box<dyn Fn()>>,
) -> SnapshotTask<T, E>
where
    F: Future<Output = Result<T, E>> + 'static,
{
    let runtime = Venus::current();
    let state = Rc::new(SnapshotState {
        snapshot: RefCell::new(if runtime.is_some() {
            AsyncSnapshot::Waiting
        } else {
            AsyncSnapshot::Error(AsyncSnapshotError::RuntimeUnavailable)
        }),
        task: RefCell::new(None),
        timer: RefCell::new(None),
        runtime: runtime.as_ref().map_or_else(Weak::new, Rc::downgrade),
        wake,
    });
    if let Some(runtime) = runtime {
        let target = Rc::downgrade(&state);
        let task = runtime.spawn_with_handle(async move {
            let result = future.await;
            if let Some(state) = target.upgrade() {
                // This task is running and about to finish: completion must not
                // cancel it.
                if let Some(task) = state.task.take() {
                    task.detach();
                }
                state.complete(match result {
                    Ok(value) => AsyncSnapshot::Data(value),
                    Err(error) => AsyncSnapshot::Error(AsyncSnapshotError::Future(error)),
                });
            }
        });
        *state.task.borrow_mut() = Some(task);
    }
    SnapshotTask { state }
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
