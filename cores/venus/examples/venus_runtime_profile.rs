//! Release measurements for Venus's scheduler and wake paths.
//!
//! Run with:
//!
//! ```text
//! cargo run -p aimer_venus --example venus_runtime_profile --release
//! ```

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::hint::black_box;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread;
use aimer_time::{Duration, Instant};

use aimer_venus::{LocalScheduler, PollContext, Venus, yield_now};

const WARMUP_ROUNDS: usize = 3;
const MEASURED_ROUNDS: usize = 9;
const EMPTY_FRAMES_PER_ROUND: usize = 20_000;
const READY_TASKS: usize = 10_000;
const WAKE_COUNT: usize = 10_000;
const SCOPE_COUNT: usize = 20_000;
const ROUND_TRIPS: usize = 2_000;
const OFFLOAD_BURST: usize = 2_000;
// A lost wake would park the driver forever; the timeout turns it into a failure.
const PARK_LIMIT: Duration = Duration::from_secs(30);

struct ParkOnce {
    slot: Rc<RefCell<Option<Waker>>>,
    ready: bool,
}

impl Future for ParkOnce {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.ready {
            return Poll::Ready(());
        }

        this.ready = true;
        *this.slot.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

struct NoopPollContext;

impl PollContext for NoopPollContext {
    #[inline]
    fn enter(&self, poll: &mut dyn FnMut()) {
        poll();
    }
}

fn report(name: &str, samples: &mut [f64], unit: &str) {
    samples.sort_by(f64::total_cmp);
    let p50 = samples[((samples.len() - 1) as f64 * 0.50).round() as usize];
    let p95 = samples[((samples.len() - 1) as f64 * 0.95).round() as usize];
    println!(
        "{name:<43} p50 {:>9.2} {unit}/op  p95 {:>9.2} {unit}/op",
        p50, p95
    );
}

fn collect_samples(mut operation: impl FnMut() -> f64) -> Vec<f64> {
    for _ in 0..WARMUP_ROUNDS {
        black_box(operation());
    }

    (0..MEASURED_ROUNDS).map(|_| operation()).collect()
}

fn drive_empty_frame(venus: &Venus) {
    venus.begin_frame();
    black_box(venus.run_frame_tasks());
    black_box(venus.run_microtasks());
    let budget = venus.idle_budget();
    black_box(venus.run_idle(&budget));
    venus.end_frame();
    black_box(venus.has_ready_work());
}

fn measure_empty_frames() {
    let venus = Venus::new();
    let mut samples = Vec::with_capacity(MEASURED_ROUNDS);

    for _ in 0..WARMUP_ROUNDS {
        for _ in 0..EMPTY_FRAMES_PER_ROUND {
            drive_empty_frame(&venus);
        }
    }

    for _ in 0..MEASURED_ROUNDS {
        let start = Instant::now();
        for _ in 0..EMPTY_FRAMES_PER_ROUND {
            drive_empty_frame(&venus);
        }
        samples.push(start.elapsed().as_secs_f64() * 1e9 / EMPTY_FRAMES_PER_ROUND as f64);
    }

    report("empty frame scheduler", &mut samples, "ns");
}

fn spawn_batch() -> f64 {
    let scheduler = LocalScheduler::new();
    let start = Instant::now();

    for _ in 0..READY_TASKS {
        scheduler.spawn(async {});
    }

    black_box(scheduler.task_count());
    start.elapsed().as_secs_f64() * 1e9 / READY_TASKS as f64
}

fn ready_drain_batch(with_context: bool) -> f64 {
    let scheduler = LocalScheduler::new();
    if with_context {
        scheduler.set_poll_context(NoopPollContext);
    }

    let calls = Rc::new(Cell::new(0));
    for _ in 0..READY_TASKS {
        let counted = calls.clone();
        scheduler.spawn(async move {
            counted.set(counted.get() + 1);
        });
    }

    let start = Instant::now();
    let polled = scheduler.run_microtasks();
    let elapsed = start.elapsed();

    assert_eq!(polled, READY_TASKS);
    assert_eq!(calls.get(), READY_TASKS);
    elapsed.as_secs_f64() * 1e9 / READY_TASKS as f64
}

fn same_thread_wake_batch() -> f64 {
    let scheduler = LocalScheduler::new();
    let steps = Rc::new(Cell::new(0));
    let counted = steps.clone();
    scheduler.spawn(async move {
        for _ in 0..WAKE_COUNT {
            counted.set(counted.get() + 1);
            yield_now().await;
        }
    });

    let start = Instant::now();
    let polled = scheduler.run_microtasks();
    let elapsed = start.elapsed();

    assert_eq!(polled, WAKE_COUNT + 1);
    assert_eq!(steps.get(), WAKE_COUNT);
    elapsed.as_secs_f64() * 1e9 / polled as f64
}

fn cross_thread_wake_batch() -> f64 {
    let scheduler = LocalScheduler::new();
    let pings = Arc::new(AtomicUsize::new(0));
    let counted = pings.clone();
    scheduler.set_notifier(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });

    let slot = Rc::new(RefCell::new(None));
    scheduler.spawn(ParkOnce {
        slot: slot.clone(),
        ready: false,
    });
    assert_eq!(scheduler.run_microtasks(), 1);

    let waker = slot
        .borrow()
        .clone()
        .expect("the parked task to publish a waker");
    let start = Instant::now();
    thread::spawn(move || {
        for _ in 0..WAKE_COUNT {
            waker.wake_by_ref();
        }
    })
    .join()
    .expect("the worker to finish");
    let polled = scheduler.run_microtasks();
    let elapsed = start.elapsed();

    assert_eq!(polled, 1);
    assert_eq!(pings.load(Ordering::SeqCst), 1);
    elapsed.as_secs_f64() * 1e9 / WAKE_COUNT as f64
}

fn scope_drop_batch() -> f64 {
    let scheduler = LocalScheduler::new();
    for _ in 0..READY_TASKS {
        scheduler.spawn(async {});
    }
    scheduler.run_microtasks();

    let start = Instant::now();
    for _ in 0..SCOPE_COUNT {
        drop(scheduler.scope());
    }
    start.elapsed().as_secs_f64() * 1e9 / SCOPE_COUNT as f64
}

/// A runtime whose wake notifier unparks the calling (UI) thread.
fn offload_runtime() -> Rc<Venus> {
    let venus = Venus::new();
    let ui = thread::current();
    venus.set_notifier(move || ui.unpark());
    venus
}

/// Runs microtasks until every task finishes, parking while workers are busy.
fn drive_until_idle(venus: &Venus) {
    while venus.task_count() > 0 {
        venus.run_microtasks();
        if venus.task_count() > 0 && !venus.has_ready_work() {
            let parked = Instant::now();
            thread::park_timeout(PARK_LIMIT);
            assert!(parked.elapsed() < PARK_LIMIT, "an offload wake was lost");
        }
    }
}

/// One offload at a time: spawn, hop to a worker, wake, resume.
fn offload_round_trip_batch(venus: &Rc<Venus>) -> f64 {
    let total = Rc::new(Cell::new(0usize));
    let sum = total.clone();
    let runtime = venus.clone();
    venus.spawn(async move {
        for value in 0..ROUND_TRIPS {
            sum.set(sum.get() + runtime.spawn_blocking(move || value).await);
        }
    });

    let start = Instant::now();
    drive_until_idle(venus);
    let elapsed = start.elapsed();

    assert_eq!(total.get(), ROUND_TRIPS * (ROUND_TRIPS - 1) / 2);
    elapsed.as_secs_f64() * 1e9 / ROUND_TRIPS as f64
}

/// Many tasks each awaiting an offload, so workers and wakes overlap.
fn offload_burst_batch(venus: &Rc<Venus>) -> f64 {
    let total = Rc::new(Cell::new(0usize));
    for value in 0..OFFLOAD_BURST {
        let sum = total.clone();
        let runtime = venus.clone();
        venus.spawn(async move {
            // Await first: `sum.get()` read before the await would be stale once
            // other overlapping tasks have added their values.
            let value = runtime.spawn_blocking(move || value).await;
            sum.set(sum.get() + value);
        });
    }

    let start = Instant::now();
    drive_until_idle(venus);
    let elapsed = start.elapsed();

    assert_eq!(total.get(), OFFLOAD_BURST * (OFFLOAD_BURST - 1) / 2);
    elapsed.as_secs_f64() * 1e9 / OFFLOAD_BURST as f64
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("Venus runtime profile ({profile}, {MEASURED_ROUNDS} measured rounds)");
    println!("scenario                                      p50        p95");

    measure_empty_frames();

    let mut samples = collect_samples(spawn_batch);
    report("task spawn", &mut samples, "ns");

    let mut samples = collect_samples(|| ready_drain_batch(false));
    report("ready microtask drain", &mut samples, "ns");

    let mut samples = collect_samples(|| ready_drain_batch(true));
    report("ready microtask drain (context)", &mut samples, "ns");

    let mut samples = collect_samples(same_thread_wake_batch);
    report("same-thread wake", &mut samples, "ns");

    let mut samples = collect_samples(cross_thread_wake_batch);
    report("cross-thread wake burst", &mut samples, "ns");

    let mut samples = collect_samples(scope_drop_batch);
    report("empty scope drop", &mut samples, "ns");

    // The runtime, and so its worker pool, is built once outside the timings.
    let venus = offload_runtime();
    let mut samples = collect_samples(|| offload_round_trip_batch(&venus));
    report("offload round trip (sequential)", &mut samples, "ns");

    let mut samples = collect_samples(|| offload_burst_batch(&venus));
    report("offload burst (overlapping)", &mut samples, "ns");
}
