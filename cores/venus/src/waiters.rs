//! The waiter list every single-threaded primitive in this crate parks tasks on.

use std::cell::RefCell;
use std::task::Waker;

/// Tasks waiting for a state change, woken all at once.
///
/// Waking everyone is deliberate: a waiter whose future was dropped leaves a
/// stale waker behind, and handing a single wake to it would lose the signal a
/// live waiter needed. A spurious wake costs one poll; a lost one hangs a task.
#[derive(Default)]
pub(crate) struct Waiters(RefCell<Vec<Waker>>);

impl Waiters {
    /// Parks `waker`, once per distinct task.
    pub(crate) fn register(&self, waker: &Waker) {
        let mut waiters = self.0.borrow_mut();
        if !waiters.iter().any(|known| known.will_wake(waker)) {
            waiters.push(waker.clone());
        }
    }

    /// Wakes every parked task. The list is released first, so a woken task
    /// that runs inline may register again.
    pub(crate) fn wake_all(&self) {
        let woken = std::mem::take(&mut *self.0.borrow_mut());
        for waker in woken {
            waker.wake();
        }
    }
}
