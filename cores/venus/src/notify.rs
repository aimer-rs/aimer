//! Waking a task from another task on the same thread, without data.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

const WAITING: u8 = 0;
const NOTIFIED_ONE: u8 = 1;
const NOTIFIED_ALL: u8 = 2;

struct Waiter {
    state: Rc<Cell<u8>>,
    waker: Waker,
}

#[derive(Default)]
struct Inner {
    /// A `notify_one` that found nobody waiting, kept for the next `notified`.
    permit: Cell<bool>,
    waiters: RefCell<VecDeque<Waiter>>,
}

/// A signal one task raises and another waits for.
///
/// The everyday uses are "something changed, look again" and "the work is
/// ready" — an edge, not a value. `Notify` is a cheap clonable handle.
///
/// [`notify_one`](Self::notify_one) wakes the longest-waiting task, or, if none
/// is waiting, stores a single permit so the next [`notified`](Self::notified)
/// returns at once: a signal raised just before the wait is not lost.
/// [`notify_waiters`](Self::notify_waiters) wakes every task waiting *now* and
/// stores nothing.
///
/// A task only counts as waiting once its [`Notified`] future has been polled.
///
/// # Examples
///
/// ```
/// use aimer_venus::{Notify, Venus};
///
/// let venus = Venus::new();
/// let ready = Notify::new();
///
/// let signal = ready.clone();
/// let worker = venus.spawn_with_handle(async move {
///     signal.notified().await;
///     "go"
/// });
/// venus.run_until_stalled();
///
/// ready.notify_one();
/// assert_eq!(venus.run_until(async move { worker.await.unwrap() }), "go");
/// ```
#[derive(Clone, Default)]
pub struct Notify(Rc<Inner>);

impl Notify {
    /// Creates a signal with nothing pending.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Waits for a notification.
    #[inline]
    pub fn notified(&self) -> Notified {
        Notified {
            inner: Rc::clone(&self.0),
            state: None,
        }
    }

    /// Wakes the task that has waited longest, or stores one permit if none is
    /// waiting.
    pub fn notify_one(&self) {
        let waiter = self.0.waiters.borrow_mut().pop_front();
        match waiter {
            Some(waiter) => {
                waiter.state.set(NOTIFIED_ONE);
                waiter.waker.wake();
            }
            None => self.0.permit.set(true),
        }
    }

    /// Wakes every task waiting right now. Nothing is stored for later.
    pub fn notify_waiters(&self) {
        let waiters = std::mem::take(&mut *self.0.waiters.borrow_mut());
        for waiter in waiters {
            waiter.state.set(NOTIFIED_ALL);
            waiter.waker.wake();
        }
    }
}

/// The future returned by [`Notify::notified`].
#[must_use = "a Notified does nothing unless awaited"]
pub struct Notified {
    inner: Rc<Inner>,
    /// Shared with this waiter's queue entry once it has queued.
    state: Option<Rc<Cell<u8>>>,
}

impl Future for Notified {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = &mut *self;
        match &this.state {
            None => {
                if this.inner.permit.replace(false) {
                    return Poll::Ready(());
                }
                let state = Rc::new(Cell::new(WAITING));
                this.inner.waiters.borrow_mut().push_back(Waiter {
                    state: Rc::clone(&state),
                    waker: cx.waker().clone(),
                });
                this.state = Some(state);
                Poll::Pending
            }
            Some(state) if state.get() != WAITING => {
                // Consumed: nothing left for `drop` to pass on.
                state.set(WAITING);
                this.state = None;
                Poll::Ready(())
            }
            Some(state) => {
                let mut waiters = this.inner.waiters.borrow_mut();
                if let Some(entry) = waiters.iter_mut().find(|entry| Rc::ptr_eq(&entry.state, state))
                    && !entry.waker.will_wake(cx.waker())
                {
                    entry.waker = cx.waker().clone();
                }
                Poll::Pending
            }
        }
    }
}

impl Drop for Notified {
    fn drop(&mut self) {
        let Some(state) = self.state.take() else {
            return;
        };
        match state.get() {
            // Still queued: leave the line.
            WAITING => self
                .inner
                .waiters
                .borrow_mut()
                .retain(|entry| !Rc::ptr_eq(&entry.state, &state)),
            // Picked by `notify_one` but never consumed: pass the wake on, or
            // the signal dies with this future.
            NOTIFIED_ONE => Notify(Rc::clone(&self.inner)).notify_one(),
            _ => {}
        }
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::Venus;

    fn waiter(venus: &Rc<Venus>, notify: &Notify, name: &'static str, log: &Rc<RefCell<Vec<&'static str>>>) {
        let (notify, log) = (notify.clone(), log.clone());
        venus.spawn(async move {
            notify.notified().await;
            log.borrow_mut().push(name);
        });
    }

    #[test]
    fn notify_one_wakes_a_waiting_task() {
        let venus = Venus::new();
        let notify = Notify::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        waiter(&venus, &notify, "a", &log);
        venus.run_until_stalled();
        assert!(log.borrow().is_empty());

        notify.notify_one();
        venus.run_until_stalled();

        assert_eq!(*log.borrow(), ["a"]);
    }

    #[test]
    fn a_notification_before_the_wait_is_not_lost() {
        let venus = Venus::new();
        let notify = Notify::new();
        notify.notify_one();
        notify.notify_one();

        venus.run_until(notify.notified());

        // Permits do not stack: a second wait still parks.
        let second = venus.run_until_for(notify.notified(), std::time::Duration::from_millis(5));
        assert!(second.is_none());
    }

    #[test]
    fn notify_one_wakes_the_longest_waiter_first() {
        let venus = Venus::new();
        let notify = Notify::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        waiter(&venus, &notify, "first", &log);
        venus.run_until_stalled();
        waiter(&venus, &notify, "second", &log);
        venus.run_until_stalled();

        notify.notify_one();
        venus.run_until_stalled();
        assert_eq!(*log.borrow(), ["first"]);

        notify.notify_one();
        venus.run_until_stalled();
        assert_eq!(*log.borrow(), ["first", "second"]);
    }

    #[test]
    fn notify_waiters_wakes_everyone_and_stores_nothing() {
        let venus = Venus::new();
        let notify = Notify::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        waiter(&venus, &notify, "a", &log);
        waiter(&venus, &notify, "b", &log);
        venus.run_until_stalled();

        notify.notify_waiters();
        venus.run_until_stalled();
        assert_eq!(log.borrow().len(), 2);

        let late = venus.run_until_for(notify.notified(), std::time::Duration::from_millis(5));
        assert!(late.is_none(), "no permit was left behind");
    }

    #[test]
    fn a_dropped_waiter_does_not_swallow_the_notification() {
        let venus = Venus::new();
        let notify = Notify::new();
        let log = Rc::new(RefCell::new(Vec::new()));
        let scope = venus.scope();
        let (cancelled, record) = (notify.clone(), log.clone());
        venus.spawn_in(scope.id(), async move {
            cancelled.notified().await;
            record.borrow_mut().push("cancelled");
        });
        venus.run_until_stalled();
        waiter(&venus, &notify, "survivor", &log);
        venus.run_until_stalled();

        // The first waiter is picked, then dropped before it can run.
        notify.notify_one();
        drop(scope);
        venus.run_until_stalled();

        assert_eq!(*log.borrow(), ["survivor"]);
    }

    #[test]
    fn a_waiter_that_leaves_the_queue_is_not_woken_later() {
        let venus = Venus::new();
        let notify = Notify::new();
        let ran = Rc::new(Cell::new(false));
        let scope = venus.scope();
        let (signal, flag) = (notify.clone(), ran.clone());
        venus.spawn_in(scope.id(), async move {
            signal.notified().await;
            flag.set(true);
        });
        venus.run_until_stalled();
        drop(scope);

        notify.notify_one();
        venus.run_until_stalled();

        assert!(!ran.get());
        // The signal was stored for the next waiter instead.
        venus.run_until(notify.notified());
    }
}
