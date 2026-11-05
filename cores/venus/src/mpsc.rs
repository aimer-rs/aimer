//! A multi-producer, single-consumer channel between tasks on one thread.
//!
//! Messages need not be [`Send`]. [`channel`] is bounded, so a producer that
//! outruns its consumer waits instead of growing a queue without limit;
//! [`unbounded`] never makes a sender wait.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::stream::Stream;
use crate::waiters::Waiters;

/// The receiver is gone; the message that could not be sent is returned.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the channel's receiver is gone")
    }
}

impl<T> std::error::Error for SendError<T> {}

/// Why [`Sender::try_send`] did not send.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum TrySendError<T> {
    /// A bounded channel is at capacity.
    Full(T),
    /// The receiver is gone.
    Closed(T),
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Full(_) => "Full(..)",
            Self::Closed(_) => "Closed(..)",
        })
    }
}

/// Why [`Receiver::try_recv`] returned nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TryRecvError {
    /// Nothing queued, but senders remain.
    Empty,
    /// Nothing queued and every sender is gone.
    Disconnected,
}

struct Inner<T> {
    queue: RefCell<VecDeque<T>>,
    capacity: Option<usize>,
    receiver: Waiters,
    senders_waiting: Waiters,
    senders: Cell<usize>,
    receiver_gone: Cell<bool>,
}

impl<T> Inner<T> {
    fn has_room(&self) -> bool {
        self.capacity
            .is_none_or(|capacity| self.queue.borrow().len() < capacity)
    }
}

/// The sending half; clone it for more producers.
pub struct Sender<T>(Rc<Inner<T>>);

/// The receiving half. Also a [`Stream`] of the messages.
pub struct Receiver<T>(Rc<Inner<T>>);

/// Creates a channel holding at most `capacity` queued messages.
///
/// # Panics
///
/// Panics if `capacity` is zero.
///
/// # Examples
///
/// ```
/// use aimer_venus::{Venus, mpsc};
///
/// let venus = Venus::new();
/// let (tx, mut rx) = mpsc::channel(8);
///
/// venus.spawn(async move {
///     for n in 0..3 {
///         tx.send(n).await.unwrap();
///     }
/// });
///
/// let received = venus.run_until(async move {
///     let mut all = Vec::new();
///     while let Some(n) = rx.recv().await {
///         all.push(n);
///     }
///     all
/// });
/// assert_eq!(received, [0, 1, 2]);
/// ```
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    assert!(capacity > 0, "a bounded channel needs room for a message");
    new(Some(capacity))
}

/// Creates a channel whose senders never wait.
pub fn unbounded<T>() -> (Sender<T>, Receiver<T>) {
    new(None)
}

fn new<T>(capacity: Option<usize>) -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(Inner {
        queue: RefCell::new(VecDeque::new()),
        capacity,
        receiver: Waiters::default(),
        senders_waiting: Waiters::default(),
        senders: Cell::new(1),
        receiver_gone: Cell::new(false),
    });
    (Sender(inner.clone()), Receiver(inner))
}

impl<T> Sender<T> {
    /// Sends `value`, waiting for room if the channel is bounded and full.
    ///
    /// Fails, returning the value, once the receiver is gone.
    pub fn send(&self, value: T) -> SendFuture<T> {
        SendFuture {
            inner: Rc::clone(&self.0),
            value: Some(value),
        }
    }

    /// Sends `value` without waiting.
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        if self.0.receiver_gone.get() {
            return Err(TrySendError::Closed(value));
        }
        if !self.0.has_room() {
            return Err(TrySendError::Full(value));
        }
        self.0.queue.borrow_mut().push_back(value);
        self.0.receiver.wake_all();
        Ok(())
    }

    /// Whether the receiver has been dropped or closed the channel.
    #[inline]
    pub fn is_closed(&self) -> bool {
        self.0.receiver_gone.get()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        self.0.senders.set(self.0.senders.get() + 1);
        Self(Rc::clone(&self.0))
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let remaining = self.0.senders.get() - 1;
        self.0.senders.set(remaining);
        if remaining == 0 {
            // The receiver is waiting for a message that can no longer come.
            self.0.receiver.wake_all();
        }
    }
}

/// The future returned by [`Sender::send`].
#[must_use = "futures do nothing unless awaited"]
pub struct SendFuture<T> {
    inner: Rc<Inner<T>>,
    value: Option<T>,
}

// The value is only ever moved out, never pinned in place.
impl<T> Unpin for SendFuture<T> {}

impl<T> Future for SendFuture<T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let value = this.value.take().expect("a SendFuture was polled after it finished");
        if this.inner.receiver_gone.get() {
            return Poll::Ready(Err(SendError(value)));
        }
        if !this.inner.has_room() {
            this.value = Some(value);
            this.inner.senders_waiting.register(cx.waker());
            return Poll::Pending;
        }
        this.inner.queue.borrow_mut().push_back(value);
        this.inner.receiver.wake_all();
        Poll::Ready(Ok(()))
    }
}

impl<T> Receiver<T> {
    /// Waits for the next message; `None` once it is empty and every sender is
    /// gone.
    pub fn recv(&mut self) -> impl Future<Output = Option<T>> + '_ {
        std::future::poll_fn(|cx| self.poll_recv(cx))
    }

    /// The polling form of [`recv`](Self::recv), for building other futures.
    pub fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let next = self.0.queue.borrow_mut().pop_front();
        match next {
            Some(value) => {
                // A slot opened up for a sender that was waiting for one.
                self.0.senders_waiting.wake_all();
                Poll::Ready(Some(value))
            }
            None if self.0.senders.get() == 0 => Poll::Ready(None),
            None => {
                self.0.receiver.register(cx.waker());
                Poll::Pending
            }
        }
    }

    /// Takes a queued message without waiting.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let next = self.0.queue.borrow_mut().pop_front();
        match next {
            Some(value) => {
                self.0.senders_waiting.wake_all();
                Ok(value)
            }
            None if self.0.senders.get() == 0 => Err(TryRecvError::Disconnected),
            None => Err(TryRecvError::Empty),
        }
    }

    /// Stops accepting messages. Those already queued can still be received.
    pub fn close(&mut self) {
        self.0.receiver_gone.set(true);
        self.0.senders_waiting.wake_all();
    }

    /// How many messages are queued.
    #[inline]
    pub fn len(&self) -> usize {
        self.0.queue.borrow().len()
    }

    /// Whether no message is queued.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.queue.borrow().is_empty()
    }
}

impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        self.poll_recv(cx)
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.close();
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::{StreamExt, Venus, yield_now};

    #[test]
    fn messages_arrive_in_order_and_end_with_none() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(4);
        venus.spawn(async move {
            for n in 0..4 {
                tx.send(Rc::new(n)).await.unwrap();
            }
        });

        let received = venus.run_until(async move {
            let mut all = Vec::new();
            while let Some(n) = rx.recv().await {
                all.push(*n);
            }
            all
        });

        assert_eq!(received, [0, 1, 2, 3]);
    }

    #[test]
    fn a_full_bounded_channel_makes_the_sender_wait() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(2);
        let sent = Rc::new(RefCell::new(Vec::new()));

        let log = sent.clone();
        venus.spawn(async move {
            for n in 0..5 {
                tx.send(n).await.unwrap();
                log.borrow_mut().push(n);
            }
        });
        venus.run_until_stalled();
        assert_eq!(*sent.borrow(), [0, 1], "the third send is parked on capacity");

        assert_eq!(rx.try_recv(), Ok(0));
        venus.run_until_stalled();
        assert_eq!(*sent.borrow(), [0, 1, 2]);
    }

    #[test]
    fn try_send_reports_full_and_closed() {
        let (tx, mut rx) = channel(1);
        assert_eq!(tx.try_send(1), Ok(()));
        assert_eq!(tx.try_send(2), Err(TrySendError::Full(2)));

        rx.close();

        assert_eq!(tx.try_send(3), Err(TrySendError::Closed(3)));
        assert!(tx.is_closed());
        assert_eq!(rx.try_recv(), Ok(1), "closing keeps what was queued");
    }

    #[test]
    fn sending_to_a_dropped_receiver_returns_the_message() {
        let venus = Venus::new();
        let (tx, rx) = channel(1);
        drop(rx);

        let result = venus.run_until(async move { tx.send("lost").await });

        assert_eq!(result, Err(SendError("lost")));
    }

    #[test]
    fn a_sender_parked_on_capacity_is_released_when_the_receiver_drops() {
        let venus = Venus::new();
        let (tx, rx) = channel(1);
        tx.try_send(0).unwrap();
        let handle = venus.spawn_with_handle(async move { tx.send(1).await });
        venus.run_until_stalled();

        drop(rx);

        let result = venus.run_until(async move { handle.await.unwrap() });
        assert_eq!(result, Err(SendError(1)));
    }

    #[test]
    fn the_receiver_ends_when_the_last_sender_is_dropped() {
        let venus = Venus::new();
        let (tx, mut rx) = unbounded();
        let second = tx.clone();
        tx.try_send(1).unwrap();
        drop(tx);

        let all = venus.spawn_with_handle(async move {
            let mut all = Vec::new();
            while let Some(n) = rx.recv().await {
                all.push(n);
            }
            all
        });
        venus.run_until_stalled();
        assert!(!all.is_finished(), "a clone still holds the channel open");

        drop(second);

        assert_eq!(venus.run_until(async move { all.await.unwrap() }), [1]);
    }

    #[test]
    fn many_senders_interleave_without_loss() {
        let venus = Venus::new();
        let (tx, rx) = unbounded();
        for sender in 0..3u32 {
            let tx = tx.clone();
            venus.spawn(async move {
                for n in 0..3 {
                    tx.send(sender * 10 + n).await.unwrap();
                    yield_now().await;
                }
            });
        }
        drop(tx);

        let mut all = venus.run_until(rx.collect_vec());
        all.sort_unstable();

        assert_eq!(all, [0, 1, 2, 10, 11, 12, 20, 21, 22]);
    }

    #[test]
    fn a_receiver_is_a_stream() {
        let venus = Venus::new();
        let (tx, rx) = unbounded();
        for n in 1..=4 {
            tx.try_send(n).unwrap();
        }
        drop(tx);

        let evens = venus.run_until(rx.filter(|n| n % 2 == 0).collect_vec());

        assert_eq!(evens, [2, 4]);
    }

    #[test]
    fn try_recv_distinguishes_empty_from_disconnected() {
        let (tx, mut rx) = unbounded::<u8>();
        assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));

        drop(tx);

        assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    }

    #[test]
    #[should_panic(expected = "needs room")]
    fn a_zero_capacity_is_rejected() {
        let _ = channel::<u8>(0);
    }
}
