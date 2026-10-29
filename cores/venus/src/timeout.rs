use std::fmt;
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;

/// Why a time-limited operation did not produce a result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeoutError {
    /// The deadline expired before the operation completed.
    Elapsed,
    /// The runtime's host did not provide a timer.
    TimerUnavailable,
}

impl fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Elapsed => "the Venus task timed out",
            Self::TimerUnavailable => "the Venus host has no timer installed",
        })
    }
}

impl std::error::Error for TimeoutError {}

pub(crate) async fn run<F: Future>(
    future: F,
    timer: Option<Pin<Box<dyn Future<Output = ()>>>>,
) -> Result<F::Output, TimeoutError> {
    let mut timer = timer.ok_or(TimeoutError::TimerUnavailable)?;
    let mut future = std::pin::pin!(future);
    // The future is polled first, so one that is already ready wins over a
    // timer that has already expired. Both are dropped when this function
    // returns, before the caller observes the result.
    poll_fn(|cx| {
        if let Poll::Ready(value) = future.as_mut().poll(cx) {
            return Poll::Ready(Ok(value));
        }
        match timer.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(TimeoutError::Elapsed)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::pin::Pin;
    use std::rc::Rc;
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    use crate::{PollContext, TimeoutError, Venus};

    #[derive(Default)]
    struct ManualClock {
        expired: Cell<bool>,
        waker: RefCell<Option<Waker>>,
        dropped: Cell<bool>,
    }

    struct ManualTimer(Rc<ManualClock>);

    impl Future for ManualTimer {
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

    impl Drop for ManualTimer {
        fn drop(&mut self) {
            self.0.dropped.set(true);
        }
    }

    impl PollContext for Rc<ManualClock> {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }

        fn sleep(&self, _duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
            Some(Box::pin(ManualTimer(self.clone())))
        }
    }

    struct DropGuard(Rc<Cell<bool>>);

    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    struct NeverExpires;

    impl PollContext for NeverExpires {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }

        fn sleep(&self, _duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
            Some(Box::pin(std::future::pending()))
        }
    }

    #[test]
    fn a_task_can_finish_before_its_timeout_with_non_send_data() {
        let venus = Venus::new();
        venus.set_poll_context(NeverExpires);
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        let value = Rc::new(42);
        let task = venus.timeout(Duration::from_secs(1), async move { value });
        venus.spawn(async move {
            output.set(Some(*task.await.expect("the task finished first")));
        });

        venus.run_microtasks();

        assert_eq!(result.get(), Some(42));
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn expiry_wakes_a_parked_task_and_drops_the_losing_future() {
        let venus = Venus::new();
        let clock = Rc::new(ManualClock::default());
        venus.set_poll_context(clock.clone());
        let dropped = Rc::new(Cell::new(false));
        let guard = DropGuard(dropped.clone());
        let task = venus.timeout(Duration::from_secs(1), async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let output = Rc::new(Cell::new(None));
        let result = output.clone();
        let observed_drop = dropped.clone();
        venus.spawn(async move {
            result.set(Some(task.await));
            assert!(observed_drop.get(), "cancel before publishing the timeout");
        });

        assert_eq!(venus.run_microtasks(), 1);
        assert!(!venus.has_ready_work(), "a pending timer must not busy poll");
        clock.expired.set(true);
        clock.waker.borrow_mut().take().expect("registered timer waker").wake();
        assert!(venus.has_ready_work());
        venus.run_microtasks();

        assert_eq!(output.get(), Some(Err(TimeoutError::Elapsed)));
        assert!(clock.dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_ready_future_wins_over_an_already_expired_timer() {
        let venus = Venus::new();
        let clock = Rc::new(ManualClock::default());
        clock.expired.set(true);
        venus.set_poll_context(clock.clone());
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        let task = venus.timeout(Duration::ZERO, std::future::ready(7));
        venus.spawn(async move { output.set(Some(task.await)) });

        venus.run_microtasks();

        assert_eq!(result.get(), Some(Ok(7)));
        assert!(clock.dropped.get());
    }

    #[test]
    fn missing_timers_are_reported_without_polling_the_future() {
        let venus = Venus::new();
        let polled = Rc::new(Cell::new(false));
        let flag = polled.clone();
        let task = venus.timeout(Duration::ZERO, async move { flag.set(true) });
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        venus.spawn(async move { output.set(Some(task.await)) });

        venus.run_microtasks();

        assert_eq!(result.get(), Some(Err(TimeoutError::TimerUnavailable)));
        assert!(!polled.get());
    }

    #[test]
    fn scope_cancellation_releases_the_future_and_timer() {
        let venus = Venus::new();
        let clock = Rc::new(ManualClock::default());
        venus.set_poll_context(clock.clone());
        let dropped = Rc::new(Cell::new(false));
        let guard = DropGuard(dropped.clone());
        let task = venus.timeout(Duration::from_secs(1), async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        let scope = venus.scope();
        venus.spawn_in(scope.id(), async move { let _ = task.await; });
        venus.run_microtasks();

        drop(scope);

        assert!(dropped.get());
        assert!(clock.dropped.get());
        assert_eq!(venus.task_count(), 0);
    }
}
