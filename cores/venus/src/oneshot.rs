//! A channel that carries one value between tasks on the same thread.
//!
//! The values need not be [`Send`]. For a result that arrives on a thread of the
//! platform's choosing, use [`completion`](crate::completion) instead.

use std::fmt;
use std::cell::{Cell, RefCell};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::waiters::Waiters;

/// The sender was dropped without sending a value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the sender was dropped without sending a value")
    }
}

impl std::error::Error for RecvError {}

/// Why [`Receiver::try_recv`] returned nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TryRecvError {
    /// No value yet; the sender is still alive.
    Empty,
    /// The sender was dropped without sending.
    Closed,
}

struct Inner<T> {
    value: RefCell<Option<T>>,
    waiters: Waiters,
    sender_gone: Cell<bool>,
    receiver_gone: Cell<bool>,
}

/// The sending half; see [`channel`].
pub struct Sender<T>(Rc<Inner<T>>);

/// The receiving half, a future of the value; see [`channel`].
#[must_use = "a receiver does nothing unless awaited"]
pub struct Receiver<T>(Rc<Inner<T>>);

/// Creates a one-value channel.
///
/// Awaiting the [`Receiver`] yields the value, or [`RecvError`] if the sender
/// was dropped first.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, oneshot};
///
/// let venus = Venus::new();
/// let (tx, rx) = oneshot::channel();
///
/// let answer = venus.spawn_with_handle(async move { rx.await });
/// tx.send(Rc::new(42)).unwrap();
///
/// let value = venus.run_until(async move { answer.await.unwrap() });
/// assert_eq!(*value.unwrap(), 42);
/// ```
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(Inner {
        value: RefCell::new(None),
        waiters: Waiters::default(),
        sender_gone: Cell::new(false),
        receiver_gone: Cell::new(false),
    });
    (Sender(inner.clone()), Receiver(inner))
}

impl<T> Sender<T> {
    /// Sends the value, handing it back if the receiver is already gone.
    pub fn send(self, value: T) -> Result<(), T> {
        if self.0.receiver_gone.get() {
            return Err(value);
        }
        *self.0.value.borrow_mut() = Some(value);
        // Dropping `self` marks the sender gone and wakes the receiver.
        Ok(())
    }

    /// Whether the receiver has been dropped, so a send could not be seen.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.0.receiver_gone.get()
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0.sender_gone.set(true);
        self.0.waiters.wake_all();
    }
}

impl<T> Receiver<T> {
    /// Takes the value if it has arrived, without waiting.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match self.0.value.borrow_mut().take() {
            Some(value) => Ok(value),
            None if self.0.sender_gone.get() => Err(TryRecvError::Closed),
            None => Err(TryRecvError::Empty),
        }
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(value) = self.0.value.borrow_mut().take() {
            return Poll::Ready(Ok(value));
        }
        if self.0.sender_gone.get() {
            return Poll::Ready(Err(RecvError));
        }
        self.0.waiters.register(cx.waker());
        Poll::Pending
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.0.receiver_gone.set(true);
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::Venus;

    #[test]
    fn a_sent_value_reaches_the_awaiting_task() {
        let venus = Venus::new();
        let (tx, rx) = channel();
        let got = venus.spawn_with_handle(async move { rx.await });
        venus.run_microtasks();

        assert!(tx.send(Rc::new(7)).is_ok());

        let value = venus.run_until(async move { got.await.unwrap() });
        assert_eq!(*value.unwrap(), 7);
    }

    #[test]
    fn a_value_sent_before_the_await_is_still_delivered() {
        let venus = Venus::new();
        let (tx, rx) = channel();
        tx.send("early").unwrap();

        assert_eq!(venus.run_until(rx), Ok("early"));
    }

    #[test]
    fn a_dropped_sender_resolves_the_receiver_with_an_error() {
        let venus = Venus::new();
        let (tx, rx) = channel::<u8>();
        let got = venus.spawn_with_handle(rx);
        venus.run_microtasks();

        drop(tx);

        assert_eq!(venus.run_until(async move { got.await.unwrap() }), Err(RecvError));
    }

    #[test]
    fn sending_to_a_dropped_receiver_returns_the_value() {
        let (tx, rx) = channel();
        drop(rx);

        assert!(tx.is_closed());
        assert_eq!(tx.send(5), Err(5));
    }

    #[test]
    fn try_recv_distinguishes_empty_from_closed() {
        let (tx, mut rx) = channel::<u8>();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));

        drop(tx);

        assert_eq!(rx.try_recv(), Err(TryRecvError::Closed));
    }

    #[test]
    fn try_recv_takes_a_sent_value() {
        let (tx, mut rx) = channel();
        tx.send(3).unwrap();

        assert_eq!(rx.try_recv(), Ok(3));
    }
}
