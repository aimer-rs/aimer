//! A lock-free slot for the one waker a worker thread has to fire.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Waker;

/// No thread is touching the waker cell.
const WAITING: usize = 0;
/// A thread inside [`WakerSlot::register`] owns the waker cell.
const REGISTERING: usize = 0b01;
/// A thread inside [`WakerSlot::take`] owns the waker cell.
const WAKING: usize = 0b10;

/// The slot where one awaiting task parks its waker for another thread to fire.
///
/// One side calls [`register`](Self::register) and the other calls
/// [`wake`](Self::wake), with no lock and no allocation on either path. The
/// state word doubles as a try-lock on the waker cell, and nobody ever waits
/// for it: a loser of the race hands its work to the winner instead.
///
/// # Protocol
///
/// - `register` that finds the slot busy waking wakes the new waker itself,
///   since the task is about to be polled again anyway.
/// - `wake` that finds a `register` in flight only records that it was here.
///   `register` notices on its way out, takes the waker it just stored and
///   fires it, so a wake is never lost.
///
/// Together this gives the usual guarantee: a `wake` that starts after
/// `register` returned wakes the registered task, and a `wake` that overlaps
/// `register` still wakes it. The awaiting side must re-check its condition
/// after registering, exactly as with any waker slot.
///
/// There is a single registering side; concurrent `register` calls are safe
/// but only one of them is guaranteed to be kept.
#[derive(Default)]
pub(crate) struct WakerSlot {
    state: AtomicUsize,
    waker: UnsafeCell<Option<Waker>>,
}

// SAFETY: `waker` is only accessed by the thread that moved `state` out of
// `WAITING` into `REGISTERING` or `WAKING`, and that thread restores `WAITING`
// with a release operation before anyone else can acquire the cell. `Waker` is
// itself `Send + Sync`.
unsafe impl Send for WakerSlot {}
unsafe impl Sync for WakerSlot {}

impl WakerSlot {
    /// Stores `waker`, skipping the clone when it already wakes the same task.
    pub(crate) fn register(&self, waker: &Waker) {
        match self
            .state
            .compare_exchange(WAITING, REGISTERING, Ordering::Acquire, Ordering::Acquire)
            .unwrap_or_else(|actual| actual)
        {
            WAITING => {
                // SAFETY: the successful exchange gave this thread exclusive
                // ownership of the cell until `state` returns to `WAITING`.
                unsafe {
                    match &*self.waker.get() {
                        Some(current) if current.will_wake(waker) => {}
                        _ => *self.waker.get() = Some(waker.clone()),
                    }
                }

                let released = self.state.compare_exchange(
                    REGISTERING,
                    WAITING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                if let Err(actual) = released {
                    // Only a concurrent `take` sets `WAKING` on top of
                    // `REGISTERING`; it left the waker for this thread to fire.
                    debug_assert_eq!(actual, REGISTERING | WAKING);
                    // SAFETY: the cell is still owned by this thread, because
                    // `take` backed off without touching it.
                    let waker = unsafe { (*self.waker.get()).take() };
                    self.state.store(WAITING, Ordering::Release);
                    if let Some(waker) = waker {
                        waker.wake();
                    }
                }
            }
            WAKING => {
                // The previous waker is being fired right now. The task will
                // run again, so wake the new one rather than wait for the cell.
                waker.wake_by_ref();
                std::hint::spin_loop();
            }
            state => {
                // Another `register` is in flight; this call loses the race.
                debug_assert!(state == REGISTERING || state == REGISTERING | WAKING);
            }
        }
    }

    /// Removes the registered waker, if any, without waking it.
    pub(crate) fn take(&self) -> Option<Waker> {
        match self.state.fetch_or(WAKING, Ordering::AcqRel) {
            WAITING => {
                // SAFETY: the previous state was `WAITING`, so this thread now
                // owns the cell until it clears `WAKING` below.
                let waker = unsafe { (*self.waker.get()).take() };
                self.state.fetch_and(!WAKING, Ordering::Release);
                waker
            }
            state => {
                // A `register` is in flight and will fire the waker it stores,
                // or another `take` is already running.
                debug_assert!(
                    state == REGISTERING || state == REGISTERING | WAKING || state == WAKING
                );
                None
            }
        }
    }

    /// Wakes the registered task, if any, and clears the slot.
    #[inline]
    pub(crate) fn wake(&self) {
        if let Some(waker) = self.take() {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::Wake;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    struct Flag(AtomicUsize);

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn flag() -> (Arc<Flag>, Waker) {
        let flag = Arc::new(Flag(AtomicUsize::new(0)));
        let waker = Waker::from(flag.clone());
        (flag, waker)
    }

    #[test]
    fn an_empty_slot_wakes_nothing() {
        let slot = WakerSlot::default();

        slot.wake();

        assert!(slot.take().is_none());
    }

    #[test]
    fn a_slot_wakes_once_and_clears() {
        let (flag, waker) = flag();
        let slot = WakerSlot::default();

        slot.register(&waker);
        slot.register(&waker);
        slot.wake();
        slot.wake();

        assert_eq!(flag.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn registering_again_replaces_a_waker_for_another_task() {
        let (first, first_waker) = flag();
        let (second, second_waker) = flag();
        let slot = WakerSlot::default();

        slot.register(&first_waker);
        slot.register(&second_waker);
        slot.wake();

        assert_eq!(first.0.load(Ordering::SeqCst), 0);
        assert_eq!(second.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn take_hands_back_the_waker_without_waking_it() {
        let (flag, waker) = flag();
        let slot = WakerSlot::default();
        slot.register(&waker);

        let taken = slot.take().expect("a registered waker");

        assert_eq!(flag.0.load(Ordering::SeqCst), 0);
        // `will_wake` is only best-effort (Miri gives each vtable reference its
        // own address), so check the contract: the returned waker is the one
        // that was registered.
        taken.wake();
        assert_eq!(flag.0.load(Ordering::SeqCst), 1);
        assert!(slot.take().is_none());
    }

    #[test]
    fn a_slot_can_be_reused_after_waking() {
        let (flag, waker) = flag();
        let slot = WakerSlot::default();

        for round in 1..=3 {
            slot.register(&waker);
            slot.wake();
            assert_eq!(flag.0.load(Ordering::SeqCst), round);
        }
    }

    // The awaiting side's protocol: publish-check, register, check again. A
    // lost wake would leave it parked forever, so the wait is bounded.
    #[test]
    fn a_wake_racing_a_registration_is_never_lost() {
        for _ in 0..5_000 {
            let (flag, waker) = flag();
            let slot = WakerSlot::default();
            let done = AtomicBool::new(false);

            thread::scope(|scope| {
                scope.spawn(|| {
                    done.store(true, Ordering::SeqCst);
                    slot.wake();
                });

                if done.load(Ordering::SeqCst) {
                    return;
                }
                slot.register(&waker);
                if done.load(Ordering::SeqCst) {
                    return;
                }
                let deadline = Instant::now() + Duration::from_secs(60);
                while flag.0.load(Ordering::SeqCst) == 0 {
                    assert!(Instant::now() < deadline, "the wake was lost");
                    thread::yield_now();
                }
            });
        }
    }
}
