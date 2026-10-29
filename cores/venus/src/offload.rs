//! The one place work leaves the UI thread.

use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::waker_slot::WakerSlot;

/// One unit of work queued for a worker thread.
type Job = Box<dyn FnOnce() + Send>;

/// The rendezvous between a worker thread and the awaiting UI-thread task.
///
/// One allocation carries the result, the "it is there" flag and the waker
/// slot. Exactly one worker publishes into it and exactly one future consumes
/// from it, so a single release/acquire flag is all the synchronization the
/// value needs.
struct Rendezvous<T> {
    value: UnsafeCell<Option<T>>,
    /// Set, with `Release`, by the worker once `value` is written; the future
    /// only touches `value` after observing it with `Acquire`.
    ready: AtomicBool,
    waker: WakerSlot,
}

// SAFETY: `value` is written once by the worker strictly before `ready` is
// released, and read once by the awaiting future strictly after `ready` is
// acquired, so the two accesses never overlap. Only `T: Send` moves between
// the threads. The remaining fields are `Sync` on their own.
unsafe impl<T: Send> Sync for Rendezvous<T> {}

impl<T> Rendezvous<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: UnsafeCell::new(None),
            ready: AtomicBool::new(false),
            waker: WakerSlot::default(),
        })
    }

    /// Publishes `value` and wakes the awaiting task.
    ///
    /// Called at most once per rendezvous, by the job that owns the worker end.
    fn complete(&self, value: T) {
        // SAFETY: the worker end is unique and `ready` is still false, so the
        // consumer is not reading `value` — see the `Sync` impl.
        unsafe { *self.value.get() = Some(value) };
        self.ready.store(true, Ordering::Release);
        self.waker.wake();
    }

    /// Takes the result if the worker has published it.
    #[inline]
    fn take(&self) -> Option<T> {
        if !self.ready.load(Ordering::Acquire) {
            return None;
        }
        // SAFETY: `ready` was acquired, so the worker's write is visible and
        // finished; the single consumer is the only reader.
        let value = unsafe { (*self.value.get()).take() };
        debug_assert!(value.is_some(), "an offloaded value was taken twice");
        value
    }
}

/// A value being computed on a worker thread.
///
/// Awaited by a task on the UI thread, which keeps its non-`Send` captures the
/// whole time: only the closure and its result cross the boundary. When the
/// value arrives, the awaiting task is woken, and the wake lands in the frame
/// phase the task belongs to — so the result is applied to state at a defined
/// point in the frame rather than "eventually".
///
/// See [`Venus::spawn_blocking`](crate::Venus::spawn_blocking).
#[must_use = "offloaded work is only observed by awaiting it"]
pub struct Offloaded<T> {
    rendezvous: Arc<Rendezvous<T>>,
}

impl<T> Future for Offloaded<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(value) = self.rendezvous.take() {
            return Poll::Ready(value);
        }

        self.rendezvous.waker.register(cx.waker());
        // A worker that finished between the check above and the registration
        // found no waker to call, so look once more.
        match self.rendezvous.take() {
            Some(value) => Poll::Ready(value),
            None => Poll::Pending,
        }
    }
}

/// How many `try_lock` attempts precede a blocking lock on the job queue.
const LOCK_SPINS: usize = 64;

/// How long the watcher lets a backlog sit without any job being popped before
/// it concludes every running job is blocked and starts working itself.
///
/// Offloaded work is by definition unsliceable, so a delay this small is
/// invisible next to the jobs that make it matter, while a backlog of tiny jobs
/// drains well inside one interval at a time.
const WATCH_INTERVAL: Duration = Duration::from_micros(50);

/// The state of the pool's single backlog watcher.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Watch {
    /// No worker is watching.
    Off,
    /// A worker has been notified to become the watcher but has not woken yet.
    Requested,
    /// A worker is waiting on [`JobQueue::stalled`] for the backlog to move.
    Active,
}

/// What the queue lock protects.
struct QueueState {
    jobs: VecDeque<Job>,
    /// Workers blocked on [`JobQueue::available`]. Exact: a worker adds itself
    /// before it waits and removes itself, under the lock, as soon as it wakes
    /// for any reason.
    idle: usize,
    /// A worker has been notified and has not yet reacquired the lock.
    ///
    /// While set, neither `push` nor a busy worker notifies another: the woken
    /// worker is already on its way to the queue, and it wakes the next one
    /// itself if work is still left. Without this, a burst of tiny jobs rouses
    /// every sleeping worker at once, and they contend for the lock and for the
    /// completion path instead of getting any work done. It is cleared by the
    /// first worker to wake, so it cannot stay set with every worker asleep.
    waking: bool,
    /// Workers that have popped a job and not yet come back for the next one.
    /// Maintained under the lock: a worker is counted out by its next `pop`.
    busy: usize,
    /// Jobs popped so far; the watcher compares it across intervals to tell a
    /// draining backlog from a stalled one.
    progress: u64,
    watch: Watch,
    /// Set by drop; workers exit once `jobs` is empty.
    closed: bool,
}

impl QueueState {
    /// Whether the caller should notify a sleeping worker, marking one as on
    /// its way.
    ///
    /// With nobody running a job, the notified worker takes the job directly.
    /// With a worker already running, waking a second one to compete for a
    /// backlog of tiny jobs only adds contention, so the notified worker
    /// becomes the watcher instead: it takes work only if the backlog stops
    /// moving, which is how a blocked job is told apart from a busy one.
    #[inline]
    fn request_help(&mut self) -> bool {
        if self.idle == 0 || self.waking {
            return false;
        }
        if self.busy > 0 {
            if self.watch != Watch::Off {
                return false;
            }
            self.watch = Watch::Requested;
        }
        self.waking = true;
        true
    }
}

/// The queue shared by all workers.
///
/// A worker holds the lock only to pop a job or to go to sleep, never while
/// waiting for one: `Condvar::wait` releases it. Workers therefore contend for
/// a few instructions of queue manipulation instead of queueing up behind a
/// single worker that is blocked waiting for the next job.
struct JobQueue {
    state: Mutex<QueueState>,
    available: Condvar,
    /// Where the watcher waits between progress checks; separate from
    /// `available` so a push never wakes the watcher by accident.
    stalled: Condvar,
}

impl JobQueue {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                jobs: VecDeque::new(),
                idle: 0,
                waking: false,
                busy: 0,
                progress: 0,
                watch: Watch::Off,
                closed: false,
            }),
            available: Condvar::new(),
            stalled: Condvar::new(),
        }
    }

    /// Jobs never run under this lock, so a panicking job cannot poison it.
    #[inline]
    fn lock(&self) -> MutexGuard<'_, QueueState> {
        // Critical sections are a few instructions, so a short spin usually
        // beats parking on a contended lock, which costs a kernel round trip
        // for both threads. The spin is bounded to stay cheap on a loaded or
        // low-power core.
        for _ in 0..LOCK_SPINS {
            match self.state.try_lock() {
                Ok(state) => return state,
                Err(TryLockError::WouldBlock) => std::hint::spin_loop(),
                Err(TryLockError::Poisoned(_)) => break,
            }
        }
        self.state
            .lock()
            .expect("the offload job queue must not be poisoned")
    }

    /// Queues `job`, waking a worker only if one is asleep.
    fn push(&self, job: Job) {
        let wake_worker = {
            let mut state = self.lock();
            if state.closed {
                return;
            }
            state.jobs.push_back(job);
            state.request_help()
        };
        // Notifying after the unlock keeps the woken worker from immediately
        // blocking on a lock this thread still holds. A worker that went idle
        // in between re-checks `jobs` before it sleeps, so no job is missed.
        if wake_worker {
            self.available.notify_one();
        }
    }

    /// Blocks for the next job; `None` once the queue is closed and drained.
    ///
    /// `finished` says the caller is coming back from running a job, which is
    /// how `busy` is kept exact without a second lock acquisition per job.
    fn pop(&self, finished: bool) -> Option<Job> {
        let mut state = self.lock();
        if finished {
            state.busy -= 1;
        }
        loop {
            if let Some(job) = state.jobs.pop_front() {
                state.busy += 1;
                state.progress = state.progress.wrapping_add(1);
                // Work is left over: ask for a watcher, which joins in only if
                // this job turns out to be slow.
                let help = !state.jobs.is_empty() && state.request_help();
                drop(state);
                if help {
                    self.available.notify_one();
                }
                return Some(job);
            }
            if state.closed {
                return None;
            }
            state.idle += 1;
            state = self
                .available
                .wait(state)
                .expect("the offload job queue must not be poisoned");
            state.idle -= 1;
            state.waking = false;
            if state.watch == Watch::Requested {
                state.watch = Watch::Active;
                state = self.watch(state);
            }
        }
    }

    /// Waits while the backlog keeps shrinking, and returns once it does not.
    ///
    /// Returns with the lock held and the watcher stood down. The caller's loop
    /// then either pops a job (the backlog stalled, so every running job must
    /// be slow or blocked) or goes back to sleep (the backlog drained, or the
    /// queue closed with nothing left). Because the watcher is not counted as
    /// idle while it waits, a push cannot hand it a job; the interval is its
    /// only trigger.
    fn watch<'a>(
        &'a self,
        mut state: MutexGuard<'a, QueueState>,
    ) -> MutexGuard<'a, QueueState> {
        let mut seen = state.progress;
        loop {
            state = self
                .stalled
                .wait_timeout(state, WATCH_INTERVAL)
                .expect("the offload job queue must not be poisoned")
                .0;
            if state.jobs.is_empty() || state.closed || state.progress == seen {
                break;
            }
            seen = state.progress;
        }
        state.watch = Watch::Off;
        state
    }

    /// Stops accepting jobs and wakes every worker so it can drain and exit.
    fn close(&self) {
        self.lock().closed = true;
        self.available.notify_all();
        self.stalled.notify_all();
    }
}

/// A small pool of threads for work that cannot be sliced.
///
/// A frame budget makes this non-optional rather than a nicety: a forty
/// millisecond parse or a PNG decode has no loop to yield from, so no amount of
/// cooperative scheduling saves the frame — the work has to leave the thread
/// entirely and come back as a wake.
///
/// This is also the *only* place Venus requires `Send`, which is the point.
/// Requiring it at the callback layer, as a general-purpose runtime does, taxes
/// every handler in the framework for the sake of the few that do I/O.
///
/// # Dispatch
///
/// Every worker pops from one shared queue. A worker releases the queue lock
/// before running a job, so a blocked job does not prevent another worker from
/// taking the next queued job.
///
/// # Panics
///
/// A closure that panics takes its worker thread with it, and the awaiting task
/// stays parked forever rather than observing a value that does not exist. Do
/// not panic in offloaded work; the release profile aborts the process on panic
/// in any case.
pub struct OffloadPool {
    queue: Arc<JobQueue>,
    workers: Vec<JoinHandle<()>>,
}

impl OffloadPool {
    /// Spawns a pool of `threads` workers, at least one.
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        let queue = Arc::new(JobQueue::new());

        let workers = (0..threads)
            .map(|index| {
                let queue = Arc::clone(&queue);
                thread::Builder::new()
                    .name(format!("aimer-venus-offload-{index}"))
                    .spawn(move || worker(&queue))
                    .expect("an offload worker thread")
            })
            .collect();

        Self { queue, workers }
    }

    /// Spawns a pool sized for the machine, leaving the UI thread a core.
    ///
    /// Capped at four: offloaded work in a GUI is bursty, and threads parked on
    /// an empty queue are pure footprint.
    pub fn with_default_threads() -> Self {
        let available = thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(2);
        Self::new(available.saturating_sub(1).clamp(1, 4))
    }

    /// Runs `work` on a worker thread, resolving on the UI thread.
    pub fn spawn_blocking<T, F>(&self, work: F) -> Offloaded<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let rendezvous = Rendezvous::new();
        let completion = Arc::clone(&rendezvous);

        self.submit(Box::new(move || completion.complete(work())));
        Offloaded { rendezvous }
    }

    /// Compatibility alias for [`Self::spawn_blocking`].
    #[inline]
    pub fn offload<T, F>(&self, work: F) -> Offloaded<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_blocking(work)
    }

    /// Queues `job` and makes sure a worker will get to it.
    #[inline]
    fn submit(&self, job: Job) {
        self.queue.push(job);
    }

    /// How many worker threads the pool owns.
    #[inline]
    pub fn thread_count(&self) -> usize {
        self.workers.len()
    }
}

impl Default for OffloadPool {
    #[inline]
    fn default() -> Self {
        Self::with_default_threads()
    }
}

impl Drop for OffloadPool {
    /// Closes the pool and joins the workers.
    ///
    /// Workers only exit once the shared queue is empty, so a job the pool
    /// accepted still runs; and joining means an application shutting down does
    /// not race a worker that is halfway through writing into a rendezvous.
    fn drop(&mut self) {
        self.queue.close();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(queue: &JobQueue) {
    let mut next = queue.pop(false);
    while let Some(job) = next {
        job();
        next = queue.pop(true);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::Venus;

    /// Spins until `condition` holds, panicking with `what` if it never does.
    ///
    /// The deadline is generous because CI machines stall; a passing run never
    /// comes near it.
    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(120);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::yield_now();
        }
    }

    // Workers sleep between bursts and the pool wakes them one at a time, so a
    // lost wake-up would strand a burst. Every round starts with all workers
    // asleep, which is the state that needs the first notification.
    #[test]
    fn bursts_after_idle_gaps_are_never_stranded() {
        let pool = OffloadPool::new(4);
        let done = Arc::new(AtomicUsize::new(0));
        let mut expected = 0;

        for round in 0..300 {
            let burst = 1 + round % 9;
            for _ in 0..burst {
                let counted = done.clone();
                drop(pool.offload(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                }));
            }
            expected += burst;
            wait_until("a burst to finish", || done.load(Ordering::SeqCst) == expected);
            // Let the workers go back to sleep.
            thread::yield_now();
        }
    }

    #[test]
    fn a_pool_always_has_at_least_one_worker() {
        assert_eq!(OffloadPool::new(0).thread_count(), 1);
        assert!(OffloadPool::with_default_threads().thread_count() >= 1);
    }

    // The dispatch property the pool must never lose: work queued while a
    // worker is stuck belongs to the pool, so a free worker can still reach
    // every queued job.
    #[test]
    fn a_blocked_worker_does_not_strand_the_jobs_queued_behind_it() {
        let pool = OffloadPool::new(2);
        let occupied = Arc::new(AtomicUsize::new(0));

        let gates: Vec<mpsc::Sender<()>> = (0..pool.thread_count())
            .map(|_| {
                let (open, gate) = mpsc::channel::<()>();
                let counted = occupied.clone();
                // The result is observed through the counter, not awaited.
                drop(pool.offload(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let _ = gate.recv();
                }));
                open
            })
            .collect();
        wait_until("every worker to pick up its blocker", || {
            occupied.load(Ordering::SeqCst) == 2
        });

        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..8 {
            let counted = done.clone();
            drop(pool.offload(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }));
        }

        // One worker comes back; the other stays wedged the whole time.
        gates[0].send(()).expect("the blocked worker to be alive");
        wait_until("the free worker to finish every queued job", || {
            done.load(Ordering::SeqCst) == 8
        });

        gates[1].send(()).expect("the blocked worker to be alive");
    }

    // Only one worker runs a backlog at first; the rest join through the
    // watcher, which must keep handing the backlog on while workers stay
    // wedged, down to the last free thread.
    #[test]
    fn a_backlog_still_drains_while_most_workers_are_blocked() {
        let pool = OffloadPool::new(4);
        let occupied = Arc::new(AtomicUsize::new(0));

        let gates: Vec<mpsc::Sender<()>> = (0..3)
            .map(|_| {
                let (open, gate) = mpsc::channel::<()>();
                let counted = occupied.clone();
                drop(pool.offload(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let _ = gate.recv();
                }));
                open
            })
            .collect();
        wait_until("three workers to pick up their blockers", || {
            occupied.load(Ordering::SeqCst) == 3
        });

        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..200 {
            let counted = done.clone();
            drop(pool.offload(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }));
        }
        wait_until("the last free worker to drain the backlog", || {
            done.load(Ordering::SeqCst) == 200
        });

        for gate in gates {
            gate.send(()).expect("the blocked worker to be alive");
        }
    }

    // A watcher must hand work on when the only worker running is wedged, even
    // though no further push arrives to nudge it.
    #[test]
    fn a_watcher_rescues_jobs_queued_behind_a_blocked_worker() {
        let pool = OffloadPool::new(4);
        let (open, gate) = mpsc::channel::<()>();
        let started = Arc::new(AtomicUsize::new(0));
        let counted = started.clone();
        drop(pool.offload(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = gate.recv();
        }));
        wait_until("the blocker to start", || started.load(Ordering::SeqCst) == 1);

        let done = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let counted = done.clone();
            drop(pool.offload(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }));
        }
        wait_until("the watcher to take the queued jobs", || {
            done.load(Ordering::SeqCst) == 3
        });

        open.send(()).expect("the blocked worker to be alive");
    }

    // Dropping the pool joins the workers, and joining means draining: a job
    // the pool accepted is a job that runs, even when the drop arrives while
    // the queue is still full.
    #[test]
    fn jobs_accepted_before_the_pool_drops_still_run() {
        let pool = OffloadPool::new(1);
        let ran = Arc::new(AtomicUsize::new(0));

        let (open, gate) = mpsc::channel::<()>();
        drop(pool.offload(move || {
            let _ = gate.recv();
        }));
        for _ in 0..16 {
            let counted = ran.clone();
            drop(pool.offload(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }));
        }

        open.send(()).expect("the blocked worker to be alive");
        drop(pool);
        assert_eq!(ran.load(Ordering::SeqCst), 16);
    }

    // Many small jobs from one submitter — the shape the dispatch rework is
    // for. Every job must land exactly once, none lost to a wake race.
    #[test]
    fn a_storm_of_small_jobs_all_lands() {
        let jobs = 10_000;
        let done = Arc::new(AtomicUsize::new(0));

        let pool = OffloadPool::new(4);
        for _ in 0..jobs {
            let counted = done.clone();
            drop(pool.offload(move || {
                counted.fetch_add(1, Ordering::SeqCst);
            }));
        }
        drop(pool);

        assert_eq!(done.load(Ordering::SeqCst), jobs);
    }

    // The property that makes `offload` worth having: the awaiting task keeps
    // its `Rc` across the boundary, because only the closure and the result
    // cross it.
    #[test]
    fn an_awaiting_task_keeps_its_non_send_state_across_the_boundary() {
        let venus = Venus::new();
        let state = Rc::new(Cell::new(0));

        let runtime = venus.clone();
        let mutated = state.clone();
        venus.spawn(async move {
            let value = runtime
                .offload(|| {
                    thread::sleep(Duration::from_millis(5));
                    41
                })
                .await;
            mutated.set(value + 1);
        });

        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        assert_eq!(state.get(), 42);
    }

    // A result nobody awaited is still owned by the rendezvous, which must
    // release it exactly once when the last reference goes away.
    #[test]
    fn an_unawaited_result_is_dropped_exactly_once() {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let pool = OffloadPool::new(1);
        let held = drops.clone();
        let offloaded = pool.offload(move || Counted(held));
        drop(pool);

        assert_eq!(drops.load(Ordering::SeqCst), 0, "the result is still held");
        drop(offloaded);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn several_offloads_all_come_back() {
        let venus = Venus::new();
        let total = Rc::new(Cell::new(0));

        for value in 1..=8 {
            let runtime = venus.clone();
            let summed = total.clone();
            venus.spawn(async move {
                let value = runtime.offload(move || value).await;
                summed.set(summed.get() + value);
            });
        }

        while venus.task_count() > 0 {
            venus.run_microtasks();
        }

        assert_eq!(total.get(), 36);
    }
}
