//! Runs blocking calls, in order, on the offload pool.

use std::collections::VecDeque;
use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::completion::{Completion, completion};
use crate::venus::Venus;

type Job<S> = Box<dyn FnOnce(&S) + Send>;

/// State a [`Lane`] owns and shares with the jobs it runs.
///
/// The hooks run on the worker that drains the lane, never on the UI thread.
/// Both default to nothing, and `()` is a state with no hooks.
pub trait LaneState: Send + Sync + 'static {
    /// A job panicked. The job's [`Completion`] resolves with
    /// [`Interrupted`](crate::Interrupted); this is the place to drop caches
    /// the job may have left half-updated.
    fn job_panicked(&self) {}

    /// The queue ran empty. Work done here happens where no caller is waiting
    /// on it. A panic is contained, and jobs queued meanwhile run before the
    /// lane goes idle.
    fn idle(&self) {}
}

impl LaneState for () {}

struct Queue<S> {
    jobs: VecDeque<Job<S>>,
    /// Whether a drain job is queued or running. Set and cleared under the same
    /// lock that guards `jobs`, so a job is never left with nobody to run it.
    draining: bool,
}

struct Shared<S> {
    state: S,
    queue: Mutex<Queue<S>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every job runs under `catch_unwind`, so a poisoned lock holds a queue that
    // is still consistent.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An ordered queue of blocking jobs. Clones share the queue.
///
/// A lane owns no thread. Each job joins a FIFO queue; when the queue goes from
/// empty to busy, one drain job is handed to
/// [`Venus::spawn_blocking`](crate::Venus::spawn_blocking) and runs jobs until
/// the queue is empty again. Jobs therefore run one at a time in the order they
/// were issued, even though the pool has several workers, so a read issued after
/// a write sees it.
///
/// Each job's result comes back as a [`Completion`], and the awaiting task is
/// woken through the thread-safe waker Venus gives its futures, so the result is
/// applied in the frame phase the task belongs to.
///
/// A lane can carry state `S`, shared with every job as `&S` and reachable from
/// the handle through `Deref`. It lives as long as the last handle or running
/// drain job, so a job outlives the handle that issued it.
///
/// # Examples
///
/// ```
/// use aimer_venus::{Lane, Venus};
///
/// let lane = Lane::<()>::default();
/// // No runtime is installed on this thread, so there is nowhere to run.
/// assert!(Venus::current().is_none());
/// assert!(lane.submit(|_| 1 + 1).is_none());
/// ```
pub struct Lane<S = ()> {
    shared: Arc<Shared<S>>,
}

impl<S> Clone for Lane<S> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<S: Default + LaneState> Default for Lane<S> {
    fn default() -> Self {
        Self::new(S::default())
    }
}

impl<S> Deref for Lane<S> {
    type Target = S;

    fn deref(&self) -> &S {
        &self.shared.state
    }
}

impl<S: LaneState> Lane<S> {
    /// Creates a lane that shares `state` with its jobs.
    pub fn new(state: S) -> Self {
        Self {
            shared: Arc::new(Shared {
                state,
                queue: Mutex::new(Queue {
                    jobs: VecDeque::new(),
                    draining: false,
                }),
            }),
        }
    }

    /// Queues `work` behind everything already issued to this lane.
    ///
    /// Returns `None` when no Venus runtime is installed on the calling thread,
    /// because there is no pool to run on. The returned [`Completion`] resolves
    /// with [`Interrupted`](crate::Interrupted) if `work` panics.
    pub fn submit<T, F>(&self, work: F) -> Option<Completion<T>>
    where
        T: Send + 'static,
        F: FnOnce(&S) -> T + Send + 'static,
    {
        let venus = Venus::current()?;
        let (completer, receive) = completion();
        let job: Job<S> = Box::new(move |state| completer.complete(work(state)));
        let start_drain = {
            let mut queue = lock(&self.shared.queue);
            queue.jobs.push_back(job);
            !std::mem::replace(&mut queue.draining, true)
        };
        if start_drain {
            let shared = Arc::clone(&self.shared);
            // The jobs report through their own completions; the drain's result is
            // not observed, and dropping the handle does not cancel the work.
            drop(venus.spawn_blocking(move || drain(&shared)));
        }
        Some(receive)
    }
}

fn drain<S: LaneState>(shared: &Shared<S>) {
    loop {
        let next = lock(&shared.queue).jobs.pop_front();
        match next {
            Some(job) => {
                // A panicking job must not strand the jobs behind it. Its completer
                // is dropped by the unwind, which resolves its future with an error.
                if catch_unwind(AssertUnwindSafe(|| job(&shared.state))).is_err() {
                    let _ = catch_unwind(AssertUnwindSafe(|| shared.state.job_panicked()));
                }
            }
            None => {
                let _ = catch_unwind(AssertUnwindSafe(|| shared.state.idle()));
                let mut queue = lock(&shared.queue);
                // Something may have been queued while the idle hook ran.
                if queue.jobs.is_empty() {
                    queue.draining = false;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Context, Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::completion::Interrupted;

    struct Parker(std::thread::Thread);

    impl Wake for Parker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    /// Blocks the test thread until `completion` resolves.
    fn wait<T>(mut completion: Completion<T>) -> Result<T, Interrupted> {
        let waker = Waker::from(Arc::new(Parker(std::thread::current())));
        let mut context = Context::from_waker(&waker);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Poll::Ready(result) = Pin::new(&mut completion).poll(&mut context) {
                return result;
            }
            assert!(Instant::now() < deadline, "the lane never delivered");
            std::thread::park_timeout(Duration::from_millis(50));
        }
    }

    fn with_runtime<R>(test: impl FnOnce() -> R) -> R {
        let venus = Venus::new();
        venus.install();
        let result = test();
        Venus::uninstall();
        result
    }

    #[test]
    fn without_a_runtime_there_is_nowhere_to_run() {
        let lane = Lane::<()>::default();

        assert!(lane.submit(|_| ()).is_none());
    }

    #[test]
    fn jobs_run_one_at_a_time_in_issue_order() {
        with_runtime(|| {
            let lane = Lane::<()>::default();
            let log = Arc::new(Mutex::new(Vec::new()));

            let tasks: Vec<_> = (0..32)
                .map(|index| {
                    let log = Arc::clone(&log);
                    lane.submit(move |_| {
                        // Yield so an unordered pool would interleave the jobs.
                        std::thread::yield_now();
                        lock(&log).push(index);
                        index
                    })
                    .unwrap()
                })
                .collect();
            let results: Vec<_> = tasks.into_iter().map(|task| wait(task).unwrap()).collect();

            assert_eq!(results, (0..32).collect::<Vec<_>>());
            assert_eq!(*lock(&log), (0..32).collect::<Vec<_>>());
        });
    }

    #[test]
    fn a_panicking_job_is_interrupted_and_the_jobs_behind_it_still_run() {
        struct Counting(AtomicUsize);

        impl LaneState for Counting {
            fn job_panicked(&self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        with_runtime(|| {
            let lane = Lane::new(Counting(AtomicUsize::new(0)));

            let bad = lane.submit(|_| -> u32 { panic!("a job that fails") }).unwrap();
            let good = lane.submit(|_| 5_u32).unwrap();

            assert_eq!(wait(bad), Err(Interrupted));
            assert_eq!(wait(good), Ok(5));
            assert_eq!(lane.0.load(Ordering::SeqCst), 1);
        });
    }

    #[test]
    fn the_idle_hook_runs_after_the_queue_empties() {
        struct Idling(AtomicUsize);

        impl LaneState for Idling {
            fn idle(&self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        with_runtime(|| {
            let lane = Lane::new(Idling(AtomicUsize::new(0)));

            wait(lane.submit(|_| ()).unwrap()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while lane.0.load(Ordering::SeqCst) == 0 {
                assert!(Instant::now() < deadline, "the idle hook never ran");
                std::thread::yield_now();
            }

            // A new job after idling starts a fresh drain.
            wait(lane.submit(|_| ()).unwrap()).unwrap();
            while lane.0.load(Ordering::SeqCst) < 2 {
                assert!(Instant::now() < deadline, "the second idle never ran");
                std::thread::yield_now();
            }
        });
    }

    #[test]
    fn jobs_can_share_state_through_the_lane() {
        with_runtime(|| {
            let lane = Lane::new(AtomicUsize::new(10));

            let task = lane.submit(|state| state.fetch_add(1, Ordering::SeqCst)).unwrap();

            assert_eq!(wait(task), Ok(10));
            assert_eq!(lane.load(Ordering::SeqCst), 11);
        });
    }

    impl LaneState for AtomicUsize {}
}
