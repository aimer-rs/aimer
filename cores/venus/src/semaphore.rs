//! Limiting how many tasks do something at once.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

struct Waiter {
    id: u64,
    waker: Waker,
}

struct Inner {
    permits: Cell<usize>,
    /// First come, first served: only the front waiter may take permits.
    queue: RefCell<VecDeque<Waiter>>,
    next_id: Cell<u64>,
}

impl Inner {
    /// Wakes the waiter at the front, which is the only one that can proceed.
    fn wake_front(&self) {
        let waker = self.queue.borrow().front().map(|waiter| waiter.waker.clone());
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn release(&self, count: usize) {
        self.permits.set(self.permits.get() + count);
        self.wake_front();
    }
}

/// A counter of permits that tasks wait on, in the order they asked.
///
/// The standard use is a concurrency cap: at most N image decodes, N requests in
/// flight. Acquire a permit before the work and hold it until the work is done;
/// dropping the [`Permit`] gives it back. `Semaphore` is a cheap clonable
/// handle, and a permit owns its share of it, so a permit can move into a
/// spawned task.
///
/// Waiters are served first-come, first-served, so a task asking for many
/// permits is not starved by a stream asking for one.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Semaphore, Venus, yield_now};
///
/// let venus = Venus::new();
/// let gate = Semaphore::new(2);
/// let running = Rc::new(Cell::new(0));
/// let peak = Rc::new(Cell::new(0));
///
/// for _ in 0..6 {
///     let (gate, running, peak) = (gate.clone(), running.clone(), peak.clone());
///     venus.spawn(async move {
///         let _permit = gate.acquire().await;
///         running.set(running.get() + 1);
///         peak.set(peak.get().max(running.get()));
///         yield_now().await; // the work
///         running.set(running.get() - 1);
///     });
/// }
/// venus.run_until_stalled();
///
/// assert_eq!(peak.get(), 2, "never more than two at once");
/// assert_eq!(running.get(), 0);
/// ```
#[derive(Clone)]
pub struct Semaphore(Rc<Inner>);

impl Semaphore {
    /// Creates a semaphore holding `permits`.
    pub fn new(permits: usize) -> Self {
        Self(Rc::new(Inner {
            permits: Cell::new(permits),
            queue: RefCell::new(VecDeque::new()),
            next_id: Cell::new(0),
        }))
    }

    /// How many permits are free right now.
    #[inline]
    pub fn available_permits(&self) -> usize {
        self.0.permits.get()
    }

    /// Adds `count` permits, waking waiters that can now proceed.
    pub fn add_permits(&self, count: usize) {
        self.0.release(count);
    }

    /// Waits for one permit.
    #[inline]
    pub fn acquire(&self) -> Acquire {
        self.acquire_many(1)
    }

    /// Waits for `count` permits at once, to be held together.
    ///
    /// A request larger than every permit the semaphore will ever have waits
    /// forever, and holds up everyone behind it.
    pub fn acquire_many(&self, count: usize) -> Acquire {
        Acquire {
            inner: Rc::clone(&self.0),
            need: count,
            queued: None,
        }
    }

    /// Takes one permit if it is free and nobody is already waiting.
    #[inline]
    pub fn try_acquire(&self) -> Option<Permit> {
        self.try_acquire_many(1)
    }

    /// Takes `count` permits if they are free and nobody is already waiting.
    pub fn try_acquire_many(&self, count: usize) -> Option<Permit> {
        if !self.0.queue.borrow().is_empty() || self.0.permits.get() < count {
            return None;
        }
        self.0.permits.set(self.0.permits.get() - count);
        Some(Permit {
            inner: Rc::clone(&self.0),
            count,
        })
    }
}

/// The future returned by [`Semaphore::acquire`] and friends.
#[must_use = "a permit is only taken by awaiting"]
pub struct Acquire {
    inner: Rc<Inner>,
    need: usize,
    /// This waiter's id once it has joined the queue.
    queued: Option<u64>,
}

impl Future for Acquire {
    type Output = Permit;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Permit> {
        let this = &mut *self;
        match this.queued {
            None => {
                if this.inner.queue.borrow().is_empty() && this.inner.permits.get() >= this.need {
                    this.inner.permits.set(this.inner.permits.get() - this.need);
                    return Poll::Ready(Permit {
                        inner: Rc::clone(&this.inner),
                        count: this.need,
                    });
                }
                let id = this.inner.next_id.get();
                this.inner.next_id.set(id + 1);
                this.inner.queue.borrow_mut().push_back(Waiter {
                    id,
                    waker: cx.waker().clone(),
                });
                this.queued = Some(id);
                Poll::Pending
            }
            Some(id) => {
                let mut queue = this.inner.queue.borrow_mut();
                let at_front = queue.front().is_some_and(|waiter| waiter.id == id);
                if at_front && this.inner.permits.get() >= this.need {
                    queue.pop_front();
                    drop(queue);
                    this.inner.permits.set(this.inner.permits.get() - this.need);
                    this.queued = None;
                    // Permits may be left over for whoever is next.
                    this.inner.wake_front();
                    return Poll::Ready(Permit {
                        inner: Rc::clone(&this.inner),
                        count: this.need,
                    });
                }
                if let Some(waiter) = queue.iter_mut().find(|waiter| waiter.id == id)
                    && !waiter.waker.will_wake(cx.waker())
                {
                    waiter.waker = cx.waker().clone();
                }
                Poll::Pending
            }
        }
    }
}

impl Drop for Acquire {
    fn drop(&mut self) {
        let Some(id) = self.queued else {
            return;
        };
        let was_front = {
            let mut queue = self.inner.queue.borrow_mut();
            let was_front = queue.front().is_some_and(|waiter| waiter.id == id);
            queue.retain(|waiter| waiter.id != id);
            was_front
        };
        // The one behind may fit in what the leaver was owed.
        if was_front {
            self.inner.wake_front();
        }
    }
}

/// Permits held by a task; dropping it gives them back.
#[must_use = "dropping a permit releases it at once"]
pub struct Permit {
    inner: Rc<Inner>,
    count: usize,
}

impl Permit {
    /// How many permits this holds.
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    /// Keeps the permits taken for good, shrinking the semaphore.
    pub fn forget(mut self) {
        self.count = 0;
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if self.count > 0 {
            self.inner.release(self.count);
        }
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use crate::{Venus, yield_now};

    #[test]
    fn permits_are_counted_and_returned_on_drop() {
        let gate = Semaphore::new(2);
        let first = gate.try_acquire().unwrap();
        let second = gate.try_acquire().unwrap();
        assert_eq!(gate.available_permits(), 0);
        assert!(gate.try_acquire().is_none());

        drop(first);
        assert_eq!(gate.available_permits(), 1);
        drop(second);
        assert_eq!(gate.available_permits(), 2);
    }

    #[test]
    fn at_most_the_permit_count_run_at_once() {
        let venus = Venus::new();
        let gate = Semaphore::new(3);
        let running = Rc::new(Cell::new(0));
        let peak = Rc::new(Cell::new(0));
        let finished = Rc::new(Cell::new(0));
        for _ in 0..10 {
            let (gate, running, peak, finished) =
                (gate.clone(), running.clone(), peak.clone(), finished.clone());
            venus.spawn(async move {
                let _permit = gate.acquire().await;
                running.set(running.get() + 1);
                peak.set(peak.get().max(running.get()));
                yield_now().await;
                yield_now().await;
                running.set(running.get() - 1);
                finished.set(finished.get() + 1);
            });
        }

        venus.run_until_stalled();

        assert_eq!(peak.get(), 3);
        assert_eq!(finished.get(), 10);
        assert_eq!(gate.available_permits(), 3);
    }

    #[test]
    fn waiters_are_served_in_the_order_they_asked() {
        let venus = Venus::new();
        let gate = Semaphore::new(1);
        let held = gate.try_acquire().unwrap();
        let order = Rc::new(RefCell::new(Vec::new()));
        for name in 0..4 {
            let (gate, order) = (gate.clone(), order.clone());
            venus.spawn(async move {
                let _permit = gate.acquire().await;
                order.borrow_mut().push(name);
            });
            venus.run_until_stalled();
        }

        drop(held);
        venus.run_until_stalled();

        assert_eq!(*order.borrow(), [0, 1, 2, 3]);
    }

    #[test]
    fn a_large_request_is_not_starved_by_small_ones_behind_it() {
        let venus = Venus::new();
        let gate = Semaphore::new(2);
        let one = gate.try_acquire().unwrap();
        let order = Rc::new(RefCell::new(Vec::new()));

        let (big_gate, big_log) = (gate.clone(), order.clone());
        venus.spawn(async move {
            let _both = big_gate.acquire_many(2).await;
            big_log.borrow_mut().push("big");
        });
        venus.run_until_stalled();
        let (small_gate, small_log) = (gate.clone(), order.clone());
        venus.spawn(async move {
            let _one = small_gate.acquire().await;
            small_log.borrow_mut().push("small");
        });
        venus.run_until_stalled();
        assert!(order.borrow().is_empty(), "the small one queues behind the big one");

        drop(one);
        venus.run_until_stalled();

        assert_eq!(*order.borrow(), ["big", "small"]);
    }

    #[test]
    fn a_waiter_that_gives_up_does_not_block_the_line() {
        let venus = Venus::new();
        let gate = Semaphore::new(1);
        let held = gate.try_acquire().unwrap();
        let scope = venus.scope();
        let leaver = gate.clone();
        venus.spawn_in(scope.id(), async move {
            let _never = leaver.acquire().await;
        });
        venus.run_until_stalled();
        let reached = Rc::new(Cell::new(false));
        let (stayer, flag) = (gate.clone(), reached.clone());
        venus.spawn(async move {
            let _permit = stayer.acquire().await;
            flag.set(true);
        });
        venus.run_until_stalled();

        drop(scope);
        drop(held);
        venus.run_until_stalled();

        assert!(reached.get());
    }

    #[test]
    fn add_permits_wakes_waiters() {
        let venus = Venus::new();
        let gate = Semaphore::new(0);
        let (waiter, got) = (gate.clone(), Rc::new(Cell::new(false)));
        let flag = got.clone();
        venus.spawn(async move {
            let permit = waiter.acquire().await;
            permit.forget();
            flag.set(true);
        });
        venus.run_until_stalled();
        assert!(!got.get());

        gate.add_permits(1);
        venus.run_until_stalled();

        assert!(got.get());
        assert_eq!(gate.available_permits(), 0, "a forgotten permit stays taken");
    }

    #[test]
    fn try_acquire_does_not_jump_the_queue() {
        let venus = Venus::new();
        let gate = Semaphore::new(1);
        let held = gate.try_acquire().unwrap();
        let waiter = gate.clone();
        venus.spawn(async move {
            let _permit = waiter.acquire().await;
            std::future::pending::<()>().await;
        });
        venus.run_until_stalled();

        drop(held);

        assert!(gate.try_acquire().is_none(), "the queued task has first claim");
        venus.run_until_stalled();
    }

    #[test]
    fn acquire_can_be_timed_out() {
        let venus = Venus::new();
        let gate = Semaphore::new(0);

        let outcome = venus.run_until_for(gate.acquire(), Duration::from_millis(5));

        assert!(outcome.is_none());
        assert_eq!(gate.available_permits(), 0);
    }
}
