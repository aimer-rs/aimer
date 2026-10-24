//! A one-value rendezvous between any thread and a Venus task.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

/// The producer of a [`Completion`] was dropped without delivering a value.
///
/// A worker that panicked, or a platform callback that was never invoked,
/// resolves its awaiting task with this rather than leaving it parked forever.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interrupted;

impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the work was dropped before it produced a value")
    }
}

impl std::error::Error for Interrupted {}

struct Slot<T> {
    state: Mutex<State<T>>,
}

struct State<T> {
    value: Option<T>,
    waker: Option<Waker>,
    /// Whether the producer is gone without a value.
    interrupted: bool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Nothing runs while the lock is held except plain field moves, so a
    // poisoned slot is still consistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Creates a connected producer and awaitable pair.
///
/// The producer is `Send`, so it can be moved to a worker thread or captured by
/// a platform callback that fires on a thread of the system's choosing. The
/// [`Completion`] is awaited by a task on the UI thread, and the wake it
/// receives lands in the frame phase the task belongs to.
///
/// # Examples
///
/// ```
/// use aimer_venus::completion;
///
/// let (completer, completion) = completion::<u32>();
/// std::thread::spawn(move || completer.complete(7)).join().unwrap();
/// # drop(completion);
/// ```
pub fn completion<T>() -> (Completer<T>, Completion<T>) {
    let slot = Arc::new(Slot {
        state: Mutex::new(State {
            value: None,
            waker: None,
            interrupted: false,
        }),
    });
    (
        Completer {
            slot: Arc::clone(&slot),
        },
        Completion { slot },
    )
}

/// The producing half of a [`completion`].
///
/// Dropping it without calling [`Completer::complete`] resolves the
/// [`Completion`] with [`Interrupted`].
pub struct Completer<T> {
    slot: Arc<Slot<T>>,
}

impl<T> Completer<T> {
    /// Delivers `value` and wakes the awaiting task.
    pub fn complete(self, value: T) {
        let waker = {
            let mut state = lock(&self.slot.state);
            state.value = Some(value);
            state.waker.take()
        };
        // Outside the lock: waking may run arbitrary scheduler code. The drop
        // below then finds a value and does nothing.
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Drop for Completer<T> {
    fn drop(&mut self) {
        let waker = {
            let mut state = lock(&self.slot.state);
            if state.value.is_some() {
                return;
            }
            state.interrupted = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// The awaiting half of a [`completion`].
#[must_use = "a completion is only observed by awaiting it"]
pub struct Completion<T> {
    slot: Arc<Slot<T>>,
}

impl<T> Future for Completion<T> {
    type Output = Result<T, Interrupted>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = lock(&self.slot.state);
        if let Some(value) = state.value.take() {
            return Poll::Ready(Ok(value));
        }
        if state.interrupted {
            return Poll::Ready(Err(Interrupted));
        }
        match &mut state.waker {
            Some(waker) => waker.clone_from(context.waker()),
            waker @ None => *waker = Some(context.waker().clone()),
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    use super::*;

    struct CountingWaker(AtomicUsize);

    impl Wake for CountingWaker {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn poll_once<T>(completion: &mut Completion<T>, waker: &Arc<CountingWaker>) -> Poll<Result<T, Interrupted>> {
        let waker = Waker::from(Arc::clone(waker));
        Pin::new(completion).poll(&mut Context::from_waker(&waker))
    }

    #[test]
    fn delivers_a_value_and_wakes_once() {
        let (completer, mut completion) = completion::<u32>();
        let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));

        assert!(poll_once(&mut completion, &waker).is_pending());
        completer.complete(7);

        assert_eq!(waker.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll_once(&mut completion, &waker), Poll::Ready(Ok(7)));
    }

    #[test]
    fn a_value_delivered_before_the_first_poll_is_not_lost() {
        let (completer, mut completion) = completion::<u32>();
        let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));

        completer.complete(3);

        assert_eq!(poll_once(&mut completion, &waker), Poll::Ready(Ok(3)));
    }

    #[test]
    fn a_dropped_producer_interrupts_and_wakes() {
        let (completer, mut completion) = completion::<u32>();
        let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));

        assert!(poll_once(&mut completion, &waker).is_pending());
        drop(completer);

        assert_eq!(waker.0.load(Ordering::SeqCst), 1);
        assert_eq!(poll_once(&mut completion, &waker), Poll::Ready(Err(Interrupted)));
    }

    #[test]
    fn a_dropped_producer_before_polling_interrupts() {
        let (completer, mut completion) = completion::<u32>();
        let waker = Arc::new(CountingWaker(AtomicUsize::new(0)));

        drop(completer);

        assert_eq!(poll_once(&mut completion, &waker), Poll::Ready(Err(Interrupted)));
    }

    #[test]
    fn the_latest_waker_is_the_one_woken() {
        let (completer, mut completion) = completion::<u32>();
        let first = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let second = Arc::new(CountingWaker(AtomicUsize::new(0)));

        assert!(poll_once(&mut completion, &first).is_pending());
        assert!(poll_once(&mut completion, &second).is_pending());
        completer.complete(1);

        assert_eq!(first.0.load(Ordering::SeqCst), 0);
        assert_eq!(second.0.load(Ordering::SeqCst), 1);
    }
}
