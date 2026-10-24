//! The one place work leaves the UI thread.

use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::thread::{self, JoinHandle};

use futures_util::task::AtomicWaker;

/// One unit of work queued for a worker thread.
type Job = Box<dyn FnOnce() + Send>;

/// The rendezvous between a worker thread and the awaiting UI-thread task.
struct Rendezvous<T> {
    completion: SyncSender<T>,
    waker: AtomicWaker,
}

impl<T> Rendezvous<T> {
    fn new() -> (Arc<Self>, Receiver<T>) {
        let (completion, result) = mpsc::sync_channel(1);
        (
            Arc::new(Self {
                completion,
                waker: AtomicWaker::new(),
            }),
            result,
        )
    }

    /// Publishes `value` and wakes the awaiting task.
    fn complete(&self, value: T) {
        let _ = self.completion.send(value);
        self.waker.wake();
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
    result: Receiver<T>,
    rendezvous: Arc<Rendezvous<T>>,
}

impl<T> Future for Offloaded<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Ok(value) = self.result.try_recv() {
            return Poll::Ready(value);
        }

        self.rendezvous.waker.register(cx.waker());
        if let Ok(value) = self.result.try_recv() {
            return Poll::Ready(value);
        }
        Poll::Pending
    }
}

/// The receiver shared by all workers.
///
/// Standard-library channels have one receiver, so workers take turns waiting
/// for the next job through this mutex. The lock is released before a job runs,
/// which lets every available worker pull from the queue independently.
type SharedJobs = Arc<Mutex<Receiver<Job>>>;

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
/// Every worker receives from one shared queue. A worker releases the receiver
/// lock before running a job, so a blocked job does not prevent another worker
/// from taking the next queued job.
///
/// # Panics
///
/// A closure that panics takes its worker thread with it, and the awaiting task
/// stays parked forever rather than observing a value that does not exist. Do
/// not panic in offloaded work; the release profile aborts the process on panic
/// in any case.
pub struct OffloadPool {
    /// The sender is taken during drop so workers observe channel disconnection
    /// after all accepted jobs have been received.
    sender: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl OffloadPool {
    /// Spawns a pool of `threads` workers, at least one.
    pub fn new(threads: usize) -> Self {
        let threads = threads.max(1);
        let (sender, receiver) = mpsc::channel();
        let receiver: SharedJobs = Arc::new(Mutex::new(receiver));

        let workers = (0..threads)
            .map(|index| {
                let receiver = Arc::clone(&receiver);
                thread::Builder::new()
                    .name(format!("aimer-venus-offload-{index}"))
                    .spawn(move || worker(receiver))
                    .expect("an offload worker thread")
            })
            .collect();

        Self {
            sender: Some(sender),
            workers,
        }
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
        let (rendezvous, result) = Rendezvous::new();
        let completion = Arc::clone(&rendezvous);

        self.submit(Box::new(move || completion.complete(work())));
        Offloaded { result, rendezvous }
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
    fn submit(&self, job: Job) {
        if let Some(sender) = self.sender.as_ref() {
            let _ = sender.send(job);
        }
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
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker(receiver: SharedJobs) {
    loop {
        let job = {
            let receiver = receiver
                .lock()
                .expect("the offload job receiver must not be poisoned");
            receiver.recv()
        };

        match job {
            Ok(job) => job(),
            Err(_) => return,
        }
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
