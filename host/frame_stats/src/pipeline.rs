//! Timings that distinguish UI occupancy from worker backpressure.
use super::{AtomicPhase, Duration, PhaseSamples};

/// Independent phase windows; worker samples may arrive after their UI frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PipelineTiming {
    /// Time spent recording and submitting on the UI thread.
    pub ui: PhaseSamples,
    /// Histogram upper bound for the UI occupancy p95.
    pub ui_p95: Duration,
    /// Whether the UI p95 reached the final histogram bucket.
    pub ui_p95_saturated: bool,
    /// Time from submission until the worker starts the frame.
    pub queue: PhaseSamples,
    /// Time spent acquiring a drawable or swap-chain image.
    pub acquire: PhaseSamples,
}

static UI: AtomicPhase = AtomicPhase::new();
static QUEUE: AtomicPhase = AtomicPhase::new();
static ACQUIRE: AtomicPhase = AtomicPhase::new();
pub(super) enum Phase {
    Ui,
    Queue,
    Acquire,
}

#[cfg(feature = "frame-stats")]
pub(super) fn record(phase: Phase, elapsed: Duration) {
    match phase {
        Phase::Ui => &UI,
        Phase::Queue => &QUEUE,
        Phase::Acquire => &ACQUIRE,
    }
    .record(elapsed);
}

/// Snapshots UI, queue, and acquisition timing without resetting their counters.
pub fn pipeline_timing() -> PipelineTiming {
    #[cfg(feature = "frame-stats")]
    let (ui_p95, ui_p95_saturated) = UI.p95_bucket();
    #[cfg(not(feature = "frame-stats"))]
    let (ui_p95, ui_p95_saturated) = (Duration::ZERO, false);
    PipelineTiming {
        ui: UI.snapshot(),
        ui_p95,
        ui_p95_saturated,
        queue: QUEUE.snapshot(),
        acquire: ACQUIRE.snapshot(),
    }
}

#[cfg(feature = "frame-stats")]
pub(super) fn take() -> PipelineTiming {
    let (ui, (ui_p95, ui_p95_saturated)) = UI.take();
    PipelineTiming {
        ui,
        ui_p95,
        ui_p95_saturated,
        queue: QUEUE.take().0,
        acquire: ACQUIRE.take().0,
    }
}
