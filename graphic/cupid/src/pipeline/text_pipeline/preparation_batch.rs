use std::collections::HashSet;
use std::hash::Hash;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, mpsc};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;

/// One unique CPU preparation job with its stable collection order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IndexedJob<K, I> {
    pub(super) order: usize,
    pub(super) key: K,
    pub(super) input: I,
}

impl<K, I> IndexedJob<K, I> {
    pub(super) fn new(order: usize, key: K, input: I) -> Self {
        Self { order, key, input }
    }
}

/// An owned CPU preparation result associated with its source job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PreparedResult<K, O> {
    order: usize,
    key: K,
    output: O,
}

impl<K, O> PreparedResult<K, O> {
    pub(super) fn new(order: usize, key: K, output: O) -> Self {
        Self { order, key, output }
    }
}

/// Validation error returned before any prepared results are committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InvalidPreparedResults;

/// Failure returned when any CPU job fails before the batch can be committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BatchExecutionError;

/// Synchronous CPU batch executor with bounded native parallelism.
///
/// Native batches above the serial threshold run through a manually configured
/// worker set. Owned preparation batches use the persistent worker set; WASM,
/// one-worker environments, and small batches use direct iteration through the
/// same job/result contract. The borrowed scoped path is retained only for the
/// release comparison tests.
#[cfg(not(target_arch = "wasm32"))]
trait WorkerBatch: Send + Sync {
    fn run(&self, worker_index: usize);
    fn fail(&self, worker_index: usize);
}

#[cfg(not(target_arch = "wasm32"))]
struct PersistentBatch<K, I, O, C, MakeContext, Prepare> {
    jobs: Arc<[IndexedJob<K, I>]>,
    active_workers: usize,
    context_type: std::marker::PhantomData<fn() -> C>,
    make_context: Arc<MakeContext>,
    prepare: Arc<Prepare>,
    next_job: AtomicUsize,
    failed: Arc<AtomicBool>,
    result_sender: mpsc::Sender<Vec<PreparedResult<K, O>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<K, I, O, C, MakeContext, Prepare> WorkerBatch
    for PersistentBatch<K, I, O, C, MakeContext, Prepare>
where
    K: Clone + Send + Sync + 'static,
    I: Clone + Send + Sync + 'static,
    O: Send + 'static,
    C: Send + 'static,
    MakeContext: Fn() -> C + Send + Sync + 'static,
    Prepare: Fn(&mut C, &IndexedJob<K, I>) -> Option<O> + Send + Sync + 'static,
{
    fn run(&self, worker_index: usize) {
        if worker_index >= self.active_workers {
            return;
        }

        let mut context = (self.make_context)();
        let mut worker_results = Vec::new();

        while !self.failed.load(Ordering::Relaxed) {
            let index = self.next_job.fetch_add(1, Ordering::Relaxed);
            let Some(job) = self.jobs.get(index) else {
                break;
            };
            let Some(output) = (self.prepare)(&mut context, job) else {
                self.failed.store(true, Ordering::Relaxed);
                break;
            };
            worker_results.push(PreparedResult::new(job.order, job.key.clone(), output));
        }

        let _ = self.result_sender.send(worker_results);
    }

    fn fail(&self, worker_index: usize) {
        self.failed.store(true, Ordering::Relaxed);
        if worker_index < self.active_workers {
            let _ = self.result_sender.send(Vec::new());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct WorkerPoolState {
    generation: u64,
    completed: usize,
    shutdown: bool,
    batch: Option<Arc<dyn WorkerBatch>>,
}

#[cfg(not(target_arch = "wasm32"))]
struct WorkerPoolShared {
    state: Mutex<WorkerPoolState>,
    wake: std::sync::Condvar,
    worker_count: usize,
}

#[cfg(not(target_arch = "wasm32"))]
struct WorkerPool {
    shared: Arc<WorkerPoolShared>,
    handles: Vec<JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl WorkerPool {
    fn new_with_worker_counter(
        worker_count: usize,
        worker_starts: Option<Arc<AtomicUsize>>,
    ) -> Self {
        debug_assert!(worker_count > 1);

        let shared = Arc::new(WorkerPoolShared {
            state: Mutex::new(WorkerPoolState {
                generation: 0,
                completed: 0,
                shutdown: false,
                batch: None,
            }),
            wake: std::sync::Condvar::new(),
            worker_count,
        });
        let mut handles = Vec::with_capacity(worker_count);

        for worker_index in 0..worker_count {
            if let Some(worker_starts) = worker_starts.as_ref() {
                worker_starts.fetch_add(1, Ordering::SeqCst);
            }
            let shared = Arc::clone(&shared);
            let handle = std::thread::Builder::new()
                .name(format!("aimer-text-worker-{worker_index}"))
                .spawn(move || {
                    let mut seen_generation = 0;
                    loop {
                        let batch = {
                            let mut state = match shared.state.lock() {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                            while !state.shutdown && state.generation == seen_generation {
                                state = match shared.wake.wait(state) {
                                    Ok(state) => state,
                                    Err(_) => return,
                                };
                            }
                            if state.shutdown {
                                return;
                            }
                            seen_generation = state.generation;
                            state.batch.as_ref().map(Arc::clone)
                        };

                        let Some(batch) = batch else {
                            continue;
                        };
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            batch.run(worker_index);
                        }))
                        .is_err()
                        {
                            batch.fail(worker_index);
                        }

                        let mut state = match shared.state.lock() {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                        state.completed += 1;
                        if state.completed == shared.worker_count {
                            state.batch = None;
                            shared.wake.notify_all();
                        }
                    }
                })
                .expect("Aimer text worker should be spawnable");
            handles.push(handle);
        }

        Self { shared, handles }
    }

    fn execute(&self, batch: Arc<dyn WorkerBatch>) {
        let mut state = self
            .shared
            .state
            .lock()
            .expect("Aimer text worker state should not be poisoned");
        while state.batch.is_some() {
            state = self
                .shared
                .wake
                .wait(state)
                .expect("Aimer text worker state should not be poisoned");
        }
        state.generation = state.generation.wrapping_add(1);
        state.completed = 0;
        state.batch = Some(batch);
        self.shared.wake.notify_all();
        while state.batch.is_some() {
            state = self
                .shared
                .wake
                .wait(state)
                .expect("Aimer text worker state should not be poisoned");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for WorkerPool {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.shutdown = true;
            self.shared.wake.notify_all();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

pub(super) struct BatchExecutor {
    #[cfg(not(target_arch = "wasm32"))]
    workers: usize,
    #[cfg(not(target_arch = "wasm32"))]
    serial_threshold: usize,
    #[cfg(not(target_arch = "wasm32"))]
    persistent_pool: Option<WorkerPool>,
}

impl BatchExecutor {
    const SERIAL_THRESHOLD: usize = 4;
    #[cfg(any(target_os = "android", target_os = "ios"))]
    const MAX_WORKERS: usize = 2;
    #[cfg(all(
        not(target_arch = "wasm32"),
        not(any(target_os = "android", target_os = "ios"))
    ))]
    const MAX_WORKERS: usize = 4;

    pub(super) fn new() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let workers = std::thread::available_parallelism()
            .map(|parallelism| parallelism.get().saturating_sub(1).max(1))
            .unwrap_or(1)
            .min(Self::MAX_WORKERS);
        #[cfg(target_arch = "wasm32")]
        let workers = 1;
        Self::with_configuration(workers, Self::SERIAL_THRESHOLD)
    }

    #[cfg(test)]
    fn for_test(workers: usize, serial_threshold: usize) -> Self {
        Self::with_configuration(workers, serial_threshold)
    }

    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn for_test_with_worker_counter(
        workers: usize,
        serial_threshold: usize,
        worker_starts: Arc<AtomicUsize>,
    ) -> Self {
        Self::with_configuration_and_worker_counter(workers, serial_threshold, Some(worker_starts))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_configuration(workers: usize, serial_threshold: usize) -> Self {
        Self::with_configuration_and_worker_counter(workers, serial_threshold, None)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn with_configuration_and_worker_counter(
        workers: usize,
        serial_threshold: usize,
        worker_starts: Option<Arc<AtomicUsize>>,
    ) -> Self {
        let workers = workers.max(1);
        Self {
            workers,
            serial_threshold,
            persistent_pool: (workers > 1)
                .then(|| WorkerPool::new_with_worker_counter(workers, worker_starts)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn with_configuration(_workers: usize, _serial_threshold: usize) -> Self {
        Self {}
    }

    #[cfg(test)]
    pub(super) fn execute_with_context<K, I, O, C, MakeContext, Prepare>(
        &self,
        jobs: &[IndexedJob<K, I>],
        make_context: MakeContext,
        prepare: Prepare,
    ) -> Result<Vec<PreparedResult<K, O>>, BatchExecutionError>
    where
        K: Clone + Send + Sync,
        I: Sync,
        O: Send,
        C: Send,
        MakeContext: Fn() -> C + Send + Sync,
        Prepare: Fn(&mut C, &IndexedJob<K, I>) -> Option<O> + Send + Sync,
    {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }

        #[cfg(not(target_arch = "wasm32"))]
        if jobs.len() >= self.serial_threshold && self.workers > 1 {
            return self.execute_parallel(jobs, &make_context, &prepare);
        }

        let mut context = make_context();
        jobs.iter()
            .map(|job| {
                prepare(&mut context, job)
                    .map(|output| PreparedResult::new(job.order, job.key.clone(), output))
                    .ok_or(BatchExecutionError)
            })
            .collect()
    }

    /// Executes an owned preparation batch on the persistent native workers.
    ///
    /// The owned bounds are intentional: a task may outlive this call on the
    /// worker queue, so it cannot borrow the caller's jobs or callback. The
    /// caller supplies one reference-counted slice, while each worker task
    /// creates one context and reuses it for all jobs it claims. The
    /// persistent threads keep thread-local scaler/font caches alive between
    /// batches.
    pub(super) fn execute_persistent_with_context<K, I, O, C, MakeContext, Prepare>(
        &self,
        jobs: Arc<[IndexedJob<K, I>]>,
        make_context: MakeContext,
        prepare: Prepare,
    ) -> Result<Vec<PreparedResult<K, O>>, BatchExecutionError>
    where
        K: Clone + Send + Sync + 'static,
        I: Clone + Send + Sync + 'static,
        O: Send + 'static,
        C: Send + 'static,
        MakeContext: Fn() -> C + Send + Sync + 'static,
        Prepare: Fn(&mut C, &IndexedJob<K, I>) -> Option<O> + Send + Sync + 'static,
    {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }

        #[cfg(not(target_arch = "wasm32"))]
        if jobs.len() >= self.serial_threshold && self.workers > 1 && self.persistent_pool.is_some()
        {
            return self.execute_persistent_parallel(jobs, make_context, prepare);
        }

        let mut context = make_context();
        jobs.iter()
            .map(|job| {
                prepare(&mut context, job)
                    .map(|output| PreparedResult::new(job.order, job.key.clone(), output))
                    .ok_or(BatchExecutionError)
            })
            .collect()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn execute_persistent_parallel<K, I, O, C, MakeContext, Prepare>(
        &self,
        jobs: Arc<[IndexedJob<K, I>]>,
        make_context: MakeContext,
        prepare: Prepare,
    ) -> Result<Vec<PreparedResult<K, O>>, BatchExecutionError>
    where
        K: Clone + Send + Sync + 'static,
        I: Clone + Send + Sync + 'static,
        O: Send + 'static,
        C: Send + 'static,
        MakeContext: Fn() -> C + Send + Sync + 'static,
        Prepare: Fn(&mut C, &IndexedJob<K, I>) -> Option<O> + Send + Sync + 'static,
    {
        let Some(pool) = self.persistent_pool.as_ref() else {
            return Err(BatchExecutionError);
        };

        let worker_count = self.workers.min(jobs.len()).max(1);
        let job_count = jobs.len();
        let make_context = Arc::new(make_context);
        let prepare = Arc::new(prepare);
        let failed = Arc::new(AtomicBool::new(false));
        let (result_sender, result_receiver) = mpsc::channel();

        let batch_state = Arc::new(PersistentBatch {
            jobs,
            active_workers: worker_count,
            context_type: std::marker::PhantomData,
            make_context,
            prepare,
            next_job: AtomicUsize::new(0),
            failed: Arc::clone(&failed),
            result_sender,
        });
        let worker_batch: Arc<dyn WorkerBatch> = batch_state;
        pool.execute(worker_batch);

        let mut results = Vec::with_capacity(job_count);
        for _ in 0..worker_count {
            let Ok(worker_results) = result_receiver.recv() else {
                return Err(BatchExecutionError);
            };
            results.extend(worker_results);
        }

        if failed.load(Ordering::Relaxed) || results.len() != job_count {
            return Err(BatchExecutionError);
        }
        Ok(results)
    }

    /// Runs one batch through a manually configured set of scoped workers.
    ///
    /// The workers share only an atomic next-job counter and a result channel.
    /// Each worker owns one context for the complete batch, so the executor
    /// keeps the same context-reuse contract as the previous pool while
    /// retaining borrowed jobs and callbacks without cloning their inputs.
    #[cfg(all(test, not(target_arch = "wasm32")))]
    fn execute_parallel<K, I, O, C, MakeContext, Prepare>(
        &self,
        jobs: &[IndexedJob<K, I>],
        make_context: &MakeContext,
        prepare: &Prepare,
    ) -> Result<Vec<PreparedResult<K, O>>, BatchExecutionError>
    where
        K: Clone + Send + Sync,
        I: Sync,
        O: Send,
        C: Send,
        MakeContext: Fn() -> C + Sync,
        Prepare: Fn(&mut C, &IndexedJob<K, I>) -> Option<O> + Sync,
    {
        let worker_count = self.workers.min(jobs.len()).max(1);
        let next_job = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let (result_sender, result_receiver) = mpsc::channel();
        let mut results = Vec::with_capacity(jobs.len());

        std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let result_sender = result_sender.clone();
                let next_job = &next_job;
                let failed = &failed;
                scope.spawn(move || {
                    let mut context = make_context();
                    let mut worker_results = Vec::new();

                    while !failed.load(Ordering::Relaxed) {
                        let index = next_job.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(index) else {
                            break;
                        };
                        let Some(output) = prepare(&mut context, job) else {
                            failed.store(true, Ordering::Relaxed);
                            break;
                        };
                        worker_results.push(PreparedResult::new(
                            job.order,
                            job.key.clone(),
                            output,
                        ));
                    }

                    let _ = result_sender.send(worker_results);
                });
            }
            drop(result_sender);
            for worker_results in result_receiver {
                results.extend(worker_results);
            }
        });

        if failed.load(Ordering::Relaxed) || results.len() != jobs.len() {
            return Err(BatchExecutionError);
        }
        Ok(results)
    }
}

/// Collects unique preparation jobs while retaining first-seen source order.
pub(super) struct PreparationBatch<K, I> {
    keys: HashSet<K>,
    jobs: Vec<IndexedJob<K, I>>,
}

impl<K, I> PreparationBatch<K, I>
where
    K: Clone + Eq + Hash,
{
    pub(super) fn new() -> Self {
        Self {
            keys: HashSet::new(),
            jobs: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, key: K, input: I) {
        if !self.keys.insert(key.clone()) {
            return;
        }

        self.jobs.push(IndexedJob::new(self.jobs.len(), key, input));
    }

    pub(super) fn jobs(&self) -> &[IndexedJob<K, I>] {
        &self.jobs
    }

    pub(super) fn into_shared_jobs(self) -> Arc<[IndexedJob<K, I>]> {
        self.jobs.into()
    }

    /// Validates and orders a complete result set before exposing any output.
    #[cfg(test)]
    pub(super) fn merge<O>(
        &self,
        results: Vec<PreparedResult<K, O>>,
    ) -> Result<Vec<(K, O)>, InvalidPreparedResults> {
        Self::merge_jobs(&self.jobs, results)
    }

    /// Validates and orders results after the batch has transferred ownership
    /// of its job slice to a persistent worker executor.
    pub(super) fn merge_jobs<O>(
        jobs: &[IndexedJob<K, I>],
        results: Vec<PreparedResult<K, O>>,
    ) -> Result<Vec<(K, O)>, InvalidPreparedResults> {
        if results.len() != jobs.len() {
            return Err(InvalidPreparedResults);
        }

        let mut ordered = std::iter::repeat_with(|| None)
            .take(jobs.len())
            .collect::<Vec<_>>();
        for result in results {
            let Some(job) = jobs.get(result.order) else {
                return Err(InvalidPreparedResults);
            };
            if result.key != job.key || ordered[result.order].is_some() {
                return Err(InvalidPreparedResults);
            }
            let order = result.order;
            ordered[order] = Some(result);
        }

        let mut by_order = Vec::with_capacity(ordered.len());
        for job in jobs {
            let Some(result) = ordered[job.order].take() else {
                return Err(InvalidPreparedResults);
            };
            by_order.push((result.key, result.output));
        }

        Ok(by_order)
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use aimer_time::{Duration, Instant};

    use super::{BatchExecutor, IndexedJob, PreparationBatch, PreparedResult};
    use crate::font::{FontFamily, FontStyle, FontWeight};
    use crate::text_pipeline::glyph_rasterizer::GlyphRasterizer;
    use crate::text_pipeline::text_layout::{
        ShapedText, layout_shaped_text_result, shape_text_styled,
    };

    #[derive(Clone)]
    struct LayoutProbe {
        shaped: Arc<ShapedText>,
        width: f32,
    }

    #[test]
    fn collection_preserves_first_seen_request_and_span_order() {
        let mut batch = PreparationBatch::new();

        batch.push("request-0/span-0", 10);
        batch.push("request-0/span-1", 11);
        batch.push("request-1/span-0", 20);

        let jobs = batch.jobs();
        assert_eq!(jobs.len(), 3);
        assert_eq!(jobs[0], IndexedJob::new(0, "request-0/span-0", 10));
        assert_eq!(jobs[1], IndexedJob::new(1, "request-0/span-1", 11));
        assert_eq!(jobs[2], IndexedJob::new(2, "request-1/span-0", 20));
    }

    #[test]
    fn collection_eliminates_duplicate_cache_keys() {
        let mut batch = PreparationBatch::new();

        batch.push("shared", 10);
        batch.push("unique", 20);
        batch.push("shared", 30);

        assert_eq!(
            batch.jobs(),
            &[
                IndexedJob::new(0, "shared", 10),
                IndexedJob::new(1, "unique", 20),
            ]
        );
    }

    #[test]
    fn empty_batch_merges_to_an_empty_commit() {
        let batch = PreparationBatch::<&str, i32>::new();

        assert_eq!(
            batch.merge(Vec::<PreparedResult<&str, i32>>::new()),
            Ok(vec![])
        );
    }

    #[test]
    fn merge_is_ordered_and_all_or_nothing() {
        let mut batch = PreparationBatch::new();
        batch.push("first", 10);
        batch.push("second", 20);

        let reversed = vec![
            PreparedResult::new(1, "second", 200),
            PreparedResult::new(0, "first", 100),
        ];
        assert_eq!(
            batch.merge(reversed),
            Ok(vec![("first", 100), ("second", 200)])
        );

        let incomplete = vec![PreparedResult::new(0, "first", 100)];
        assert!(batch.merge(incomplete).is_err());

        let duplicate = vec![
            PreparedResult::new(0, "first", 100),
            PreparedResult::new(0, "first", 101),
        ];
        assert!(batch.merge(duplicate).is_err());
    }

    #[test]
    fn executor_merges_out_of_order_work_in_source_order() {
        let executor = BatchExecutor::for_test(2, 1);
        let mut batch = PreparationBatch::new();
        batch.push("slow", 30_u64);
        batch.push("fast", 0_u64);

        let results = executor
            .execute_with_context(
                batch.jobs(),
                || (),
                |_, job| {
                    std::thread::sleep(Duration::from_millis(job.input));
                    Some(job.input)
                },
            )
            .unwrap();

        assert_eq!(batch.merge(results), Ok(vec![("slow", 30), ("fast", 0)]));
    }

    #[test]
    fn executor_uses_serial_path_below_threshold() {
        let executor = BatchExecutor::for_test(4, 4);
        let mut batch = PreparationBatch::new();
        batch.push("one", 1);
        batch.push("two", 2);
        batch.push("three", 3);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        executor
            .execute_with_context(batch.jobs(), || (), {
                let active = active.clone();
                let peak = peak.clone();
                move |_, _| {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(2));
                    active.fetch_sub(1, Ordering::SeqCst);
                    Some(())
                }
            })
            .unwrap();

        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn executor_failure_exposes_no_partial_result_set() {
        let executor = BatchExecutor::for_test(2, 1);
        let mut batch = PreparationBatch::new();
        batch.push("good", 1);
        batch.push("bad", 2);

        let result = executor.execute_with_context(
            batch.jobs(),
            || (),
            |_, job| (job.key != "bad").then_some(job.input),
        );

        assert!(result.is_err());
    }

    #[test]
    fn serial_and_parallel_execution_produce_identical_ordered_results() {
        let serial = BatchExecutor::for_test(1, 1);
        let parallel = BatchExecutor::for_test(4, 1);
        let mut batch = PreparationBatch::new();
        for value in 0..16 {
            batch.push(value, value * 2);
        }

        let execute = |executor: &BatchExecutor| {
            let results = executor
                .execute_with_context(
                    batch.jobs(),
                    || 10,
                    |context, job| Some(*context + job.input),
                )
                .unwrap();
            batch.merge(results).unwrap()
        };

        assert_eq!(execute(&parallel), execute(&serial));
    }

    #[test]
    fn empty_batch_does_not_construct_a_worker_context() {
        let executor = BatchExecutor::for_test(4, 1);
        let contexts = AtomicUsize::new(0);
        let jobs = Vec::<IndexedJob<i32, i32>>::new();

        let results = executor
            .execute_with_context(
                &jobs,
                || {
                    contexts.fetch_add(1, Ordering::SeqCst);
                },
                |_, _| Some(()),
            )
            .unwrap();

        assert!(results.is_empty());
        assert_eq!(contexts.load(Ordering::SeqCst), 0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_executor_reuses_worker_threads_across_batches() {
        let worker_starts = Arc::new(AtomicUsize::new(0));
        let executor = BatchExecutor::for_test_with_worker_counter(2, 1, worker_starts.clone());
        let mut batch = PreparationBatch::new();
        for value in 0..8 {
            batch.push(value, value);
        }
        let jobs: Arc<[IndexedJob<usize, usize>]> = batch.jobs().to_vec().into();

        let execute = || {
            executor
                .execute_persistent_with_context(jobs.clone(), || (), |_, job| Some(job.input))
                .unwrap()
                .len()
        };

        let first = execute();
        let starts_after_first = worker_starts.load(Ordering::SeqCst);
        let second = execute();

        assert_eq!(first, batch.jobs().len());
        assert_eq!(second, batch.jobs().len());
        assert_eq!(starts_after_first, 2);
        assert_eq!(worker_starts.load(Ordering::SeqCst), starts_after_first);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_executor_preserves_order_and_rejects_failed_batches() {
        let executor = BatchExecutor::for_test(2, 1);
        let mut batch = PreparationBatch::new();
        batch.push("slow", 3_u64);
        batch.push("fast", 0_u64);
        let jobs: Arc<[IndexedJob<&str, u64>]> = batch.jobs().to_vec().into();

        let results = executor
            .execute_persistent_with_context(
                jobs.clone(),
                || (),
                |_, job| {
                    std::thread::sleep(Duration::from_millis(job.input));
                    Some(job.input)
                },
            )
            .unwrap();
        assert_eq!(batch.merge(results), Ok(vec![("slow", 3), ("fast", 0)]));

        let mut failed_batch = PreparationBatch::new();
        failed_batch.push("good", 1_u64);
        failed_batch.push("bad", 2_u64);
        let failed_jobs: Arc<[IndexedJob<&str, u64>]> = failed_batch.jobs().to_vec().into();
        assert!(
            executor
                .execute_persistent_with_context(
                    failed_jobs,
                    || (),
                    |_, job| (job.key != "bad").then_some(job.input),
                )
                .is_err()
        );

        assert!(
            executor
                .execute_persistent_with_context(jobs, || (), |_, job| Some(job.input),)
                .is_ok()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[ignore = "run in release mode for a persistent-vs-scoped shaping/layout comparison"]
    fn persistent_executor_shaping_benchmark() {
        const ITERATIONS: usize = 100;
        const JOB_COUNT: usize = 16;
        const WORKERS: usize = 4;
        const SAMPLE: &str =
            "Aimer shapes styled text into glyph runs before wrapping and painting. ";

        let inputs = (0..JOB_COUNT)
            .map(|index| {
                Arc::<str>::from(format!("{SAMPLE} persistent-worker-batch-{index} {SAMPLE}"))
            })
            .collect::<Vec<_>>();
        let jobs: Arc<[IndexedJob<usize, Arc<str>>]> = inputs
            .iter()
            .enumerate()
            .map(|(order, input)| IndexedJob::new(order, order, input.clone()))
            .collect::<Vec<_>>()
            .into();
        let persistent = BatchExecutor::for_test(WORKERS, 1);
        let scoped = BatchExecutor::for_test(WORKERS, 1);

        let execute_persistent = || {
            persistent
                .execute_persistent_with_context(
                    jobs.clone(),
                    move || GlyphRasterizer::new(),
                    move |rasterizer, job| {
                        Some(shape_text_styled(
                            rasterizer,
                            job.input.as_ref(),
                            16.0,
                            FontFamily::SANS_SERIF,
                            FontWeight::Normal,
                            FontStyle::Normal,
                            None,
                        ))
                    },
                )
                .map(|results| black_box(results.len()))
                .expect("persistent shaping batch should complete")
        };
        let execute_scoped = || {
            scoped
                .execute_with_context(
                    &jobs,
                    || GlyphRasterizer::new(),
                    |rasterizer, job| {
                        Some(shape_text_styled(
                            rasterizer,
                            job.input.as_ref(),
                            16.0,
                            FontFamily::SANS_SERIF,
                            FontWeight::Normal,
                            FontStyle::Normal,
                            None,
                        ))
                    },
                )
                .map(|results| black_box(results.len()))
                .expect("scoped shaping batch should complete")
        };

        assert_eq!(execute_persistent(), JOB_COUNT);
        assert_eq!(execute_scoped(), JOB_COUNT);

        let persistent_start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(execute_persistent());
        }
        let persistent_elapsed = persistent_start.elapsed();

        let scoped_start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(execute_scoped());
        }
        let scoped_elapsed = scoped_start.elapsed();

        let shaped = Arc::new(shape_text_styled(
            &mut GlyphRasterizer::new(),
            SAMPLE,
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        ));
        let layout_jobs: Arc<[IndexedJob<usize, LayoutProbe>]> = (0..JOB_COUNT)
            .map(|order| {
                IndexedJob::new(
                    order,
                    order,
                    LayoutProbe {
                        shaped: shaped.clone(),
                        width: 240.0 + order as f32,
                    },
                )
            })
            .collect::<Vec<_>>()
            .into();
        let layout_persistent = BatchExecutor::for_test(WORKERS, 1);
        let layout_scoped = BatchExecutor::for_test(WORKERS, 1);
        let execute_layout_persistent = || {
            layout_persistent
                .execute_persistent_with_context(
                    layout_jobs.clone(),
                    || (),
                    |(), job| {
                        Some(layout_shaped_text_result(
                            &job.input.shaped,
                            0.0,
                            0.0,
                            job.input.width,
                        ))
                    },
                )
                .map(|results| black_box(results.len()))
                .expect("persistent layout batch should complete")
        };
        let execute_layout_scoped = || {
            layout_scoped
                .execute_with_context(
                    &layout_jobs,
                    || (),
                    |(), job| {
                        Some(layout_shaped_text_result(
                            &job.input.shaped,
                            0.0,
                            0.0,
                            job.input.width,
                        ))
                    },
                )
                .map(|results| black_box(results.len()))
                .expect("scoped layout batch should complete")
        };

        assert_eq!(execute_layout_persistent(), JOB_COUNT);
        assert_eq!(execute_layout_scoped(), JOB_COUNT);

        let layout_persistent_start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(execute_layout_persistent());
        }
        let layout_persistent_elapsed = layout_persistent_start.elapsed();

        let layout_scoped_start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(execute_layout_scoped());
        }
        let layout_scoped_elapsed = layout_scoped_start.elapsed();

        println!(
            "persistent executor shaping: persistent={persistent_elapsed:?}, scoped={scoped_elapsed:?}, iterations={ITERATIONS}, jobs={JOB_COUNT}"
        );
        println!(
            "persistent executor layout: persistent={layout_persistent_elapsed:?}, scoped={layout_scoped_elapsed:?}, iterations={ITERATIONS}, jobs={JOB_COUNT}"
        );
    }
}
