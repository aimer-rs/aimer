//! Timers and background tasks on one thread.
//!
//! A daemon has no frame to hang timers on, so it owns a single worker thread
//! that sleeps until the earliest deadline (or forever when there is none) and
//! runs due jobs one after another. One thread, a binary heap and a condition
//! variable: nothing polls, so an idle daemon costs no CPU.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type Job = Box<dyn FnMut() + Send>;

/// A queued job and when it is due.
struct Entry {
    due: Instant,
    /// Breaks ties so jobs due at the same instant run in submission order.
    seq: u64,
    period: Option<Duration>,
    cancelled: Arc<AtomicBool>,
    job: Job,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.due == other.due && self.seq == other.seq
    }
}

impl Eq for Entry {}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Entry {
    /// Reversed, so the std max-heap pops the earliest deadline first.
    fn cmp(&self, other: &Self) -> Ordering {
        other.due.cmp(&self.due).then_with(|| other.seq.cmp(&self.seq))
    }
}

/// The time-ordered queue, free of threads and clocks so tests can drive it
/// with fixed instants.
#[derive(Default)]
struct Queue {
    heap: BinaryHeap<Entry>,
    next_seq: u64,
}

impl Queue {
    fn push(
        &mut self,
        due: Instant,
        period: Option<Duration>,
        cancelled: Arc<AtomicBool>,
        job: Job,
    ) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(Entry { due, seq, period, cancelled, job });
    }

    /// The deadline of the earliest job that has not been cancelled.
    fn next_due(&mut self) -> Option<Instant> {
        self.discard_cancelled();
        self.heap.peek().map(|entry| entry.due)
    }

    /// Removes and returns the earliest job if it is due at `now`.
    fn pop_due(&mut self, now: Instant) -> Option<Entry> {
        self.discard_cancelled();
        if self.heap.peek()?.due <= now { self.heap.pop() } else { None }
    }

    /// Queues a repeating job's next run. A tick missed because the thread was
    /// busy is skipped rather than replayed in a burst.
    fn reschedule(&mut self, mut entry: Entry, now: Instant) {
        let Some(period) = entry.period else { return };
        let mut next = entry.due + period;
        if next <= now {
            next = now + period;
        }
        entry.due = next;
        entry.seq = self.next_seq;
        self.next_seq += 1;
        self.heap.push(entry);
    }

    fn discard_cancelled(&mut self) {
        while self
            .heap
            .peek()
            .is_some_and(|entry| entry.cancelled.load(AtomicOrdering::Acquire))
        {
            self.heap.pop();
        }
    }
}

struct State {
    queue: Queue,
    stopping: bool,
}

struct Inner {
    state: Mutex<State>,
    wake: Condvar,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Jobs run outside the lock and panics are caught, so the lock cannot
        // be poisoned by user code; recover anyway rather than cascade.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Cancels a scheduled job.
///
/// Dropping the handle does **not** cancel the job; it keeps running detached.
#[derive(Clone, Debug)]
pub struct TaskHandle {
    cancelled: Arc<AtomicBool>,
}

impl TaskHandle {
    /// Stops the job from running again. A run already in progress finishes.
    #[inline]
    pub fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Release);
    }

    /// Whether [`TaskHandle::cancel`] was called, or the job panicked.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrdering::Acquire)
    }
}

/// The scheduling thread and the queue it serves.
pub(crate) struct Scheduler {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Scheduler {
    pub(crate) fn start() -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State { queue: Queue::default(), stopping: false }),
            wake: Condvar::new(),
        });
        let worker = Arc::clone(&inner);
        let thread = std::thread::Builder::new()
            .name("aimer-daemon-scheduler".to_owned())
            .spawn(move || run(&worker))
            .expect("the operating system can spawn the scheduler thread");
        Self { inner, thread: Mutex::new(Some(thread)) }
    }

    /// Queues `job` for `delay` from now, repeating every `period` if given.
    ///
    /// After [`Scheduler::stop`] nothing runs, and the returned handle is
    /// already cancelled.
    pub(crate) fn schedule(
        &self,
        delay: Duration,
        period: Option<Duration>,
        job: Job,
    ) -> TaskHandle {
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = TaskHandle { cancelled: Arc::clone(&cancelled) };
        let mut state = self.inner.lock();
        if state.stopping {
            cancelled.store(true, AtomicOrdering::Release);
            return handle;
        }
        // A zero or huge delay must not overflow the clock; saturate to a
        // deadline far enough away to be "never".
        let now = Instant::now();
        let due = now.checked_add(delay).unwrap_or_else(|| far_future(now));
        state.queue.push(due, period, cancelled, job);
        drop(state);
        self.inner.wake.notify_one();
        handle
    }

    /// Drops queued jobs, lets a running one finish and joins the thread.
    ///
    /// Safe to call from a job (the join is skipped on the scheduler thread)
    /// and more than once.
    pub(crate) fn stop(&self) {
        let abandoned = {
            let mut state = self.inner.lock();
            state.stopping = true;
            std::mem::take(&mut state.queue)
        };
        self.inner.wake.notify_all();
        // Dropped outside the lock: a job may own a handle that locks again.
        drop(abandoned);
        let thread = self.thread.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(thread) = thread
            && thread.thread().id() != std::thread::current().id()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        self.stop();
    }
}

fn far_future(now: Instant) -> Instant {
    // About 30 years; `checked_add` on that cannot fail on the supported
    // platforms, but fall back to `now` instead of panicking if one does.
    now.checked_add(Duration::from_secs(60 * 60 * 24 * 365 * 30)).unwrap_or(now)
}

fn run(inner: &Inner) {
    let mut state = inner.lock();
    loop {
        if state.stopping {
            return;
        }
        let now = Instant::now();
        if let Some(mut entry) = state.queue.pop_due(now) {
            // The job runs without the lock so it may schedule more work.
            drop(state);
            let survived =
                panic::catch_unwind(AssertUnwindSafe(|| (entry.job)())).is_ok();
            state = inner.lock();
            if survived {
                state.queue.reschedule(entry, Instant::now());
            } else {
                // A job that panics is not run again.
                entry.cancelled.store(true, AtomicOrdering::Release);
            }
            continue;
        }
        state = match state.queue.next_due() {
            Some(due) => {
                let timeout = due.saturating_duration_since(now);
                inner
                    .wake
                    .wait_timeout(state, timeout)
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .0
            }
            None => inner.wake.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner()),
        };
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn noop() -> Job {
        Box::new(|| {})
    }

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn queue_pops_earliest_first_and_ties_in_submission_order() {
        let base = Instant::now();
        let mut queue = Queue::default();
        queue.push(base + Duration::from_secs(2), None, flag(), noop());
        queue.push(base + Duration::from_secs(1), None, flag(), noop());
        queue.push(base + Duration::from_secs(1), None, flag(), noop());
        let late = base + Duration::from_secs(10);
        let a = queue.pop_due(late).unwrap();
        let b = queue.pop_due(late).unwrap();
        let c = queue.pop_due(late).unwrap();
        assert_eq!(a.due, base + Duration::from_secs(1));
        assert_eq!(b.due, base + Duration::from_secs(1));
        assert!(a.seq < b.seq);
        assert_eq!(c.due, base + Duration::from_secs(2));
        assert!(queue.pop_due(late).is_none());
    }

    #[test]
    fn queue_holds_back_jobs_that_are_not_due() {
        let base = Instant::now();
        let mut queue = Queue::default();
        queue.push(base + Duration::from_secs(5), None, flag(), noop());
        assert!(queue.pop_due(base + Duration::from_secs(4)).is_none());
        assert_eq!(queue.next_due(), Some(base + Duration::from_secs(5)));
        assert!(queue.pop_due(base + Duration::from_secs(5)).is_some());
    }

    #[test]
    fn queue_skips_cancelled_jobs() {
        let base = Instant::now();
        let mut queue = Queue::default();
        let cancelled = flag();
        queue.push(base, None, Arc::clone(&cancelled), noop());
        queue.push(base + Duration::from_secs(1), None, flag(), noop());
        cancelled.store(true, AtomicOrdering::Release);
        assert_eq!(queue.next_due(), Some(base + Duration::from_secs(1)));
        assert!(queue.pop_due(base).is_none());
    }

    #[test]
    fn reschedule_keeps_the_cadence_and_ignores_one_shots() {
        let base = Instant::now();
        let period = Duration::from_secs(2);
        let mut queue = Queue::default();
        queue.push(base, Some(period), flag(), noop());
        queue.push(base, None, flag(), noop());
        let repeating = queue.pop_due(base).unwrap();
        let one_shot = queue.pop_due(base).unwrap();
        queue.reschedule(one_shot, base);
        assert!(queue.next_due().is_none());
        queue.reschedule(repeating, base + Duration::from_millis(100));
        assert_eq!(queue.next_due(), Some(base + period));
    }

    #[test]
    fn reschedule_skips_missed_ticks_instead_of_bursting() {
        let base = Instant::now();
        let period = Duration::from_secs(1);
        let mut queue = Queue::default();
        queue.push(base, Some(period), flag(), noop());
        let entry = queue.pop_due(base).unwrap();
        let now = base + Duration::from_secs(10);
        queue.reschedule(entry, now);
        assert_eq!(queue.next_due(), Some(now + period));
    }

    #[test]
    fn runs_a_one_shot_job_after_its_delay() {
        let scheduler = Scheduler::start();
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        scheduler.schedule(
            Duration::from_millis(30),
            None,
            Box::new(move || tx.send(Instant::now()).unwrap()),
        );
        let ran_at = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(ran_at.duration_since(started) >= Duration::from_millis(30));
        scheduler.stop();
    }

    #[test]
    fn repeating_job_runs_until_cancelled() {
        let scheduler = Scheduler::start();
        let (tx, rx) = mpsc::channel();
        let handle = scheduler.schedule(
            Duration::ZERO,
            Some(Duration::from_millis(5)),
            Box::new(move || {
                let _ = tx.send(());
            }),
        );
        for _ in 0..3 {
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        handle.cancel();
        assert!(handle.is_cancelled());
        scheduler.stop();
        // The sender dropped with the job, so draining ends.
        while rx.try_recv().is_ok() {}
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn a_panicking_job_does_not_kill_the_thread() {
        let scheduler = Scheduler::start();
        let bad = scheduler.schedule(Duration::ZERO, None, Box::new(|| panic!("boom")));
        let (tx, rx) = mpsc::channel();
        scheduler.schedule(
            Duration::from_millis(10),
            None,
            Box::new(move || tx.send(()).unwrap()),
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(bad.is_cancelled());
        scheduler.stop();
    }

    #[test]
    fn jobs_may_schedule_more_jobs() {
        let scheduler = Arc::new(Scheduler::start());
        let (tx, rx) = mpsc::channel();
        let inner = Arc::clone(&scheduler);
        scheduler.schedule(
            Duration::ZERO,
            None,
            Box::new({
                let mut tx = Some(tx);
                move || {
                    let tx = tx.take().unwrap();
                    inner.schedule(Duration::ZERO, None, Box::new({
                        let mut tx = Some(tx);
                        move || tx.take().unwrap().send(()).unwrap()
                    }));
                }
            }),
        );
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        scheduler.stop();
    }

    #[test]
    fn stop_is_idempotent_and_refuses_new_work() {
        let scheduler = Scheduler::start();
        scheduler.stop();
        scheduler.stop();
        let handle = scheduler.schedule(Duration::ZERO, None, noop());
        assert!(handle.is_cancelled());
    }

    #[test]
    fn an_enormous_delay_does_not_panic() {
        let scheduler = Scheduler::start();
        let handle = scheduler.schedule(Duration::MAX, None, noop());
        assert!(!handle.is_cancelled());
        scheduler.stop();
    }
}
