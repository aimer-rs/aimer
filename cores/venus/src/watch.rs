//! A single shared value that observers are told about when it changes.
//!
//! Receivers always see the *latest* value: several updates between two looks
//! collapse into one notification. That is the right shape for state a UI
//! mirrors — a theme, a connection status, a selected id — where the history
//! does not matter, only the present.

use std::cell::{Cell, Ref, RefCell};
use std::fmt;
use std::rc::Rc;
use std::task::Poll;

use crate::waiters::Waiters;

/// The sender was dropped, so the value will never change again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the watch sender was dropped")
    }
}

impl std::error::Error for RecvError {}

/// Every receiver is gone; the value that could not be delivered is returned.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SendError<T>(pub T);

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SendError(..)")
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("every watch receiver is gone")
    }
}

impl<T> std::error::Error for SendError<T> {}

struct Inner<T> {
    value: RefCell<T>,
    /// Bumped on every change; a receiver compares it with the one it last saw.
    version: Cell<u64>,
    waiters: Waiters,
    sender_gone: Cell<bool>,
    receivers: Cell<usize>,
}

/// The sending half; see [`channel`].
pub struct Sender<T>(Rc<Inner<T>>);

/// An observer of the value; clone it for more observers.
pub struct Receiver<T> {
    inner: Rc<Inner<T>>,
    seen: u64,
}

/// Creates a watch channel holding `initial`.
///
/// A new receiver considers `initial` already seen; it is told about later
/// changes only.
///
/// # Examples
///
/// ```
/// use aimer_venus::{Venus, watch};
///
/// let venus = Venus::new();
/// let (tx, mut rx) = watch::channel("light");
///
/// let theme = venus.spawn_with_handle(async move {
///     rx.changed().await.unwrap();
///     *rx.borrow()
/// });
/// venus.run_until_stalled();
///
/// tx.send("dark").unwrap();
/// assert_eq!(venus.run_until(async move { theme.await.unwrap() }), "dark");
/// ```
pub fn channel<T>(initial: T) -> (Sender<T>, Receiver<T>) {
    let inner = Rc::new(Inner {
        value: RefCell::new(initial),
        version: Cell::new(0),
        waiters: Waiters::default(),
        sender_gone: Cell::new(false),
        receivers: Cell::new(1),
    });
    (
        Sender(inner.clone()),
        Receiver {
            inner,
            seen: 0,
        },
    )
}

impl<T> Sender<T> {
    /// Replaces the value and notifies receivers, failing if none remain.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        if self.0.receivers.get() == 0 {
            return Err(SendError(value));
        }
        self.send_replace(value);
        Ok(())
    }

    /// Replaces the value and notifies receivers, returning the old value. Works
    /// whether or not anyone is listening.
    pub fn send_replace(&self, value: T) -> T {
        let old = self.0.value.replace(value);
        self.changed();
        old
    }

    /// Changes the value in place and notifies receivers.
    pub fn send_modify(&self, modify: impl FnOnce(&mut T)) {
        modify(&mut self.0.value.borrow_mut());
        self.changed();
    }

    /// Borrows the current value. Do not hold the borrow across a send.
    #[inline]
    pub fn borrow(&self) -> Ref<'_, T> {
        self.0.value.borrow()
    }

    /// How many receivers are listening.
    #[inline]
    pub fn receiver_count(&self) -> usize {
        self.0.receivers.get()
    }

    /// Creates a receiver that treats the current value as already seen.
    pub fn subscribe(&self) -> Receiver<T> {
        self.0.receivers.set(self.0.receivers.get() + 1);
        Receiver {
            inner: Rc::clone(&self.0),
            seen: self.0.version.get(),
        }
    }

    fn changed(&self) {
        self.0.version.set(self.0.version.get() + 1);
        self.0.waiters.wake_all();
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0.sender_gone.set(true);
        self.0.waiters.wake_all();
    }
}

impl<T> Receiver<T> {
    /// Borrows the current value without marking it seen. Do not hold the
    /// borrow across an `await`.
    #[inline]
    pub fn borrow(&self) -> Ref<'_, T> {
        self.inner.value.borrow()
    }

    /// Borrows the current value and marks it seen, so the next
    /// [`changed`](Self::changed) waits for a newer one.
    pub fn borrow_and_update(&mut self) -> Ref<'_, T> {
        self.seen = self.inner.version.get();
        self.inner.value.borrow()
    }

    /// Whether the value changed since this receiver last looked.
    #[inline]
    pub fn has_changed(&self) -> bool {
        self.inner.version.get() != self.seen
    }

    /// Waits until the value differs from the one last seen, then marks it
    /// seen. Resolves at once if it already has changed.
    ///
    /// Fails once the sender is gone *and* nothing unseen remains.
    pub async fn changed(&mut self) -> Result<(), RecvError> {
        std::future::poll_fn(|cx| {
            let version = self.inner.version.get();
            if version != self.seen {
                self.seen = version;
                return Poll::Ready(Ok(()));
            }
            if self.inner.sender_gone.get() {
                return Poll::Ready(Err(RecvError));
            }
            self.inner.waiters.register(cx.waker());
            Poll::Pending
        })
        .await
    }

    /// Waits until the value satisfies `predicate`, returning once it does —
    /// immediately if it already does. Fails if the sender is dropped first.
    pub async fn wait_for(&mut self, mut predicate: impl FnMut(&T) -> bool) -> Result<(), RecvError> {
        loop {
            if predicate(&self.borrow_and_update()) {
                return Ok(());
            }
            self.changed().await?;
        }
    }
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        self.inner.receivers.set(self.inner.receivers.get() + 1);
        Self {
            inner: Rc::clone(&self.inner),
            seen: self.seen,
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        self.inner.receivers.set(self.inner.receivers.get() - 1);
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::Venus;

    #[test]
    fn a_receiver_is_told_when_the_value_changes() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(1);
        let seen = venus.spawn_with_handle(async move {
            rx.changed().await.unwrap();
            *rx.borrow()
        });
        venus.run_until_stalled();
        assert!(!seen.is_finished());

        tx.send(2).unwrap();

        assert_eq!(venus.run_until(async move { seen.await.unwrap() }), 2);
    }

    #[test]
    fn several_updates_between_looks_collapse_into_the_latest() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(0);
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();

        let rx = venus.run_until(async move {
            rx.changed().await.unwrap();
            rx
        });

        assert_eq!(*rx.borrow(), 3);
        assert!(!rx.has_changed(), "one notification covered all three");
    }

    #[test]
    fn a_new_receiver_does_not_see_the_initial_value_as_a_change() {
        let (tx, rx) = channel(5);
        assert!(!rx.has_changed());
        assert!(!tx.subscribe().has_changed());
    }

    #[test]
    fn changed_fails_after_the_sender_is_dropped_and_nothing_is_unseen() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(0);
        tx.send(1).unwrap();
        drop(tx);

        let (first, rx) = venus.run_until(async move {
            let first = rx.changed().await;
            (first, rx)
        });
        assert_eq!(first, Ok(()), "the last value is still delivered");

        let second = venus.run_until(async move {
            let mut rx = rx;
            rx.changed().await
        });
        assert_eq!(second, Err(RecvError));
    }

    #[test]
    fn a_parked_receiver_is_woken_by_the_sender_dropping() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(0);
        let outcome = venus.spawn_with_handle(async move { rx.changed().await });
        venus.run_until_stalled();

        drop(tx);

        assert_eq!(venus.run_until(async move { outcome.await.unwrap() }), Err(RecvError));
    }

    #[test]
    fn every_receiver_sees_a_change() {
        let venus = Venus::new();
        let (tx, rx) = channel(0);
        let woken = Rc::new(Cell::new(0));
        for _ in 0..3 {
            let mut rx = rx.clone();
            let woken = woken.clone();
            venus.spawn(async move {
                rx.changed().await.unwrap();
                woken.set(woken.get() + 1);
            });
        }
        venus.run_until_stalled();

        tx.send(1).unwrap();
        venus.run_until_stalled();

        assert_eq!(woken.get(), 3);
    }

    #[test]
    fn wait_for_returns_when_the_predicate_holds() {
        let venus = Venus::new();
        let (tx, mut rx) = channel(0);
        let done = venus.spawn_with_handle(async move { rx.wait_for(|n| *n >= 3).await });
        venus.run_until_stalled();

        tx.send(1).unwrap();
        venus.run_until_stalled();
        assert!(!done.is_finished());
        tx.send(5).unwrap();

        assert_eq!(venus.run_until(async move { done.await.unwrap() }), Ok(()));
    }

    #[test]
    fn wait_for_is_immediate_when_the_value_already_qualifies() {
        let venus = Venus::new();
        let (_tx, mut rx) = channel(10);

        assert_eq!(venus.run_until(async move { rx.wait_for(|n| *n > 5).await }), Ok(()));
    }

    #[test]
    fn send_fails_without_receivers_but_send_replace_does_not() {
        let (tx, rx) = channel(1);
        drop(rx);

        assert_eq!(tx.send(2), Err(SendError(2)));
        assert_eq!(tx.send_replace(3), 1);
        assert_eq!(*tx.borrow(), 3);
    }

    #[test]
    fn send_modify_changes_in_place() {
        let (tx, rx) = channel(vec![1]);

        tx.send_modify(|items| items.push(2));

        assert!(rx.has_changed());
        assert_eq!(*rx.borrow(), [1, 2]);
        assert_eq!(tx.receiver_count(), 1);
    }
}
