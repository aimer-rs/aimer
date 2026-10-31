//! Where a frame's time actually goes.
//!
//! A frame is three distinct pieces of work. In threaded mode, recording stays
//! on the UI thread, encoding moves to the worker, and presentation follows the
//! platform's thread-affinity rules:
//!
//! | Phase                     | Work                                            |
//! |---------------------------|-------------------------------------------------|
//! | [`FramePhase::Build`]     | build + layout + paint — the widget walk        |
//! | [`FramePhase::Encode`]    | encode draw calls and submit GPU commands      |
//! | [`FramePhase::Present`]   | hand the swap chain image to the compositor     |
//!
//! On native Metal, `cpu_frame` spans the start of command recording through
//! command-buffer submission (and scheduling when offloaded), before drawable presentation. `gpu_frame` spans
//! the earliest GPU start through the latest GPU completion among the command
//! buffers submitted for that frame. GPU measurements arrive asynchronously
//! from Metal completion handlers. Their p95 values are upper bounds from
//! 0.5 ms histogram buckets.
//!
//! Native apps default to a bounded render worker. [`pipeline_timing`] separates
//! UI recording/submission, queue delay, and drawable acquisition; these are
//! independent windows because worker and GPU samples arrive asynchronously.
//! `cpu_frame` includes queue delay when offloaded, through backend submission
//! (and producer scheduling on Metal), before UI drawable presentation.
//!
//! Runtime instrumentation is inactive unless the `frame-stats` feature is
//! enabled; without it, [`PhaseTimer::start`] reads no clock and
//! [`PhaseTimer::finish`] does nothing.
//!
//! # Examples
//!
//! ```no_run
//! # fn read_it() {
//! let breakdown = aimer_frame_stats::frame_breakdown();
//! println!(
//!     "build {:?} / encode {:?} / present {:?}",
//!     breakdown.build.average(),
//!     breakdown.encode.average(),
//!     breakdown.present.average()
//! );
//! # }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
mod pipeline;
pub use pipeline::{PipelineTiming, pipeline_timing};

/// One of the three phases a frame passes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePhase {
    /// The widget walk: build, layout and paint into a [`DrawList`].
    ///
    /// [`DrawList`]: aimer_cupid::draw_cmd::DrawList
    Build,
    /// Encoding the recorded draw list and submitting GPU work.
    Encode,
    /// Presenting the encoded image.
    Present,
}

/// Accumulated timings for a single phase.
///
/// A phase is sampled once per frame that reached it, so `samples` differs
/// between phases when frames are dropped: a frame whose surface texture could
/// not be acquired is built, but never encoded or presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhaseSamples {
    /// How many frames contributed to this phase.
    pub samples: u64,
    /// The summed duration of every sample.
    pub total: Duration,
}

impl PhaseSamples {
    /// The mean time this phase took, or [`Duration::ZERO`] when never sampled.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_frame_stats::PhaseSamples;
    ///
    /// let samples = PhaseSamples {
    ///     samples: 4,
    ///     total: Duration::from_millis(8),
    /// };
    ///
    /// assert_eq!(samples.average(), Duration::from_millis(2));
    /// assert_eq!(PhaseSamples::default().average(), Duration::ZERO);
    /// ```
    #[inline]
    pub fn average(&self) -> Duration {
        if self.samples == 0 {
            return Duration::ZERO;
        }
        self.total / self.samples.min(u32::MAX as u64) as u32
    }
}

/// A snapshot of the frame breakdown, as of the moment it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameBreakdown {
    /// Build, layout and paint — the part that stays on the UI thread.
    pub build: PhaseSamples,
    /// Command encoding and submission; acquisition is in [`PipelineTiming`].
    pub encode: PhaseSamples,
    /// Presentation of the encoded image.
    pub present: PhaseSamples,
}

/// Timing summaries for the native Metal frame window.
#[cfg(feature = "frame-stats")]
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTimingStats {
    pub pipeline: PipelineTiming,
    pub cpu_frame: PhaseSamples,
    pub cpu_frame_p95: Duration,
    pub cpu_frame_p95_saturated: bool,
    pub gpu_frame: PhaseSamples,
    pub gpu_frame_p95: Duration,
    pub gpu_frame_p95_saturated: bool,
}

/// Counts the UI work recorded for a group of frames.
///
/// These values describe the build-side workload. They are deliberately kept
/// separate from [`FrameBreakdown`], whose encode and present samples may run
/// on a different thread when raster offloading is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameContentStats {
    /// Number of frames represented by the counters.
    pub frames: u64,
    /// Number of retained elements reached by drawing.
    pub drawn_nodes: u64,
    /// Number of commands recorded into the frame draw lists.
    pub draw_commands: u64,
    /// Number of compositor-style retained layers recorded into the frame.
    pub retained_layers: u64,
    /// Number of text and text-decoration commands recorded.
    pub text_commands: u64,
    /// Number of already-loaded texture draw commands recorded.
    pub image_draws: u64,
    /// Number of image-byte upload commands recorded.
    pub image_uploads: u64,
    /// Number of text metrics cache hits.
    pub text_cache_hits: u64,
    /// Number of text metrics cache misses.
    pub text_cache_misses: u64,
    /// Number of retained element boundaries entered by rebuilding.
    pub rebuild_visits: u64,
    /// Number of retained boundaries pruned by the dirty-path index.
    pub rebuild_pruned: u64,
    /// Number of stateful elements checked by the rebuild walk.
    pub stateful_rebuild_checks: u64,
    /// Number of stateless elements checked by the rebuild walk.
    pub stateless_rebuild_checks: u64,
    /// Number of stateful `build` callbacks invoked during the frame.
    pub stateful_builds: u64,
    /// Number of stateless `build` callbacks invoked during the frame.
    pub stateless_builds: u64,
    /// Number of retained-element layout calls observed during the frame.
    pub layout_calls: u64,
    /// Number of routed event visits used for hit testing and input dispatch.
    pub hit_test_visits: u64,
    /// Number of retained-element paint calls observed during the frame.
    pub paint_calls: u64,
    /// Number of root draw calls that entered the retained tree.
    pub root_draw_calls: u64,
    /// Number of scroll input events accepted by the application root.
    pub scroll_events: u64,
    /// Number of delivered scroll steps from input or smoothing.
    pub scroll_steps: u64,
    /// Number of active smoothing ticks.
    pub smoothing_steps: u64,
    /// Number of state updates requested through the framework state API.
    pub state_updates: u64,
    /// New element invalidation records queued during the interval.
    pub invalidations_queued: u64,
    /// Element invalidations merged into an already pending record.
    pub invalidations_coalesced: u64,
    /// Direct retained-tree lookups performed for invalidation records.
    pub element_index_lookups: u64,
    /// Invalidation IDs that were stale in the current tree generation.
    pub stale_element_ids: u64,
    /// Number of scroll-offset changes committed to retained scroll state.
    pub scroll_offset_updates: u64,
    /// Number of window redraw requests observed by the framework.
    pub redraw_requests: u64,
}

impl FrameContentStats {
    #[inline]
    fn average(value: u64, frames: u64) -> f64 {
        if frames == 0 {
            0.0
        } else {
            value as f64 / frames as f64
        }
    }

    /// Average number of drawn retained elements per frame.
    #[inline]
    pub fn average_drawn_nodes(&self) -> f64 {
        Self::average(self.drawn_nodes, self.frames)
    }

    /// Average number of recorded draw commands per frame.
    #[inline]
    pub fn average_draw_commands(&self) -> f64 {
        Self::average(self.draw_commands, self.frames)
    }

    /// Average number of retained compositor layers per frame.
    #[inline]
    pub fn average_retained_layers(&self) -> f64 {
        Self::average(self.retained_layers, self.frames)
    }

    /// Average number of text commands per frame.
    #[inline]
    pub fn average_text_commands(&self) -> f64 {
        Self::average(self.text_commands, self.frames)
    }

    /// Average number of image draws per frame.
    #[inline]
    pub fn average_image_draws(&self) -> f64 {
        Self::average(self.image_draws, self.frames)
    }

    /// Average number of image uploads per frame.
    #[inline]
    pub fn average_image_uploads(&self) -> f64 {
        Self::average(self.image_uploads, self.frames)
    }

    /// Average number of text metrics cache misses per frame.
    #[inline]
    pub fn average_text_cache_misses(&self) -> f64 {
        Self::average(self.text_cache_misses, self.frames)
    }
}

/// Counts frame-wake requests around the native display/event-loop tick.
///
/// `accepted` is a request that occupied the one pending wake slot;
/// `coalesced` is a request made while that slot was already occupied; and
/// `display_ticks` is a delivered `FrameReady` wake. The counters describe
/// scheduling pressure, not rendered-frame count — a platform may merge or
/// defer a redraw after the wake is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameRequestStats {
    /// Requests that successfully occupied the pending frame-wake slot.
    pub accepted: u64,
    /// Requests folded into an already pending frame wake.
    pub coalesced: u64,
    /// Native display/event-loop ticks that consumed a pending frame wake.
    pub display_ticks: u64,
}

#[derive(Debug, Default)]
struct FrameRequestAccumulator {
    accepted: AtomicU64,
    coalesced: AtomicU64,
    display_ticks: AtomicU64,
}

#[cfg_attr(not(feature = "frame-stats"), allow(dead_code))]
impl FrameRequestAccumulator {
    #[inline]
    fn accepted(&self) {
        self.accepted.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn coalesced(&self) {
        self.coalesced.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn display_tick(&self) {
        self.display_ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> FrameRequestStats {
        FrameRequestStats {
            accepted: self.accepted.load(Ordering::Relaxed),
            coalesced: self.coalesced.load(Ordering::Relaxed),
            display_ticks: self.display_ticks.load(Ordering::Relaxed),
        }
    }

    fn reset(&self) {
        self.accepted.store(0, Ordering::Relaxed);
        self.coalesced.store(0, Ordering::Relaxed);
        self.display_ticks.store(0, Ordering::Relaxed);
    }
}

impl FrameBreakdown {
    /// The mean cost of a whole frame across all three phases.
    #[inline]
    pub fn average_frame(&self) -> Duration {
        self.build.average() + self.encode.average() + self.present.average()
    }

    /// The share of an average frame spent downstream of the widget walk.
    ///
    /// This covers the three recorded phases; separate acquisition and queue
    /// timings are not included. Returns `0.0` before anything has been sampled.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_frame_stats::{FrameBreakdown, PhaseSamples};
    ///
    /// let breakdown = FrameBreakdown {
    ///     build: PhaseSamples {
    ///         samples: 1,
    ///         total: Duration::from_millis(3),
    ///     },
    ///     encode: PhaseSamples {
    ///         samples: 1,
    ///         total: Duration::from_millis(1),
    ///     },
    ///     present: PhaseSamples::default(),
    /// };
    ///
    /// assert_eq!(breakdown.offloadable_share(), 0.25);
    /// ```
    #[inline]
    pub fn offloadable_share(&self) -> f64 {
        let frame = self.average_frame().as_secs_f64();
        if frame <= 0.0 {
            return 0.0;
        }
        (self.encode.average() + self.present.average()).as_secs_f64() / frame
    }
}

/// The lock-free accumulator behind the global statistics.
///
/// Kept separate from the global so the arithmetic is testable, and atomic
/// because the encode and present phases run on the raster thread while the
/// build phase runs on the UI thread.
#[derive(Debug, Default)]
struct FrameAccumulator {
    build: AtomicPhase,
    encode: AtomicPhase,
    present: AtomicPhase,
    #[cfg(feature = "frame-stats")]
    cpu_frame: AtomicPhase,
    #[cfg(feature = "frame-stats")]
    gpu_frame: AtomicPhase,
}

// `record` only has a caller in the render path when instrumentation is
// compiled in; the accumulator itself is always built, and always tested.
#[cfg_attr(not(feature = "frame-stats"), allow(dead_code))]
impl FrameAccumulator {
    #[inline]
    fn record(&self, phase: FramePhase, elapsed: Duration) {
        self.phase(phase).record(elapsed);
    }

    #[inline]
    fn phase(&self, phase: FramePhase) -> &AtomicPhase {
        match phase {
            FramePhase::Build => &self.build,
            FramePhase::Encode => &self.encode,
            FramePhase::Present => &self.present,
        }
    }

    fn snapshot(&self) -> FrameBreakdown {
        FrameBreakdown {
            build: self.build.snapshot(),
            encode: self.encode.snapshot(),
            present: self.present.snapshot(),
        }
    }

    #[cfg(feature = "frame-stats")]
    fn timing_snapshot(&self) -> FrameTimingStats {
        let (cpu_frame_p95, cpu_frame_p95_saturated) = self.cpu_frame.p95_bucket();
        let (gpu_frame_p95, gpu_frame_p95_saturated) = self.gpu_frame.p95_bucket();
        FrameTimingStats {
            pipeline: pipeline_timing(),
            cpu_frame: self.cpu_frame.snapshot(),
            cpu_frame_p95,
            cpu_frame_p95_saturated,
            gpu_frame: self.gpu_frame.snapshot(),
            gpu_frame_p95,
            gpu_frame_p95_saturated,
        }
    }

    #[cfg(feature = "frame-stats")]
    fn take(&self) -> (FrameBreakdown, FrameTimingStats) {
        let (cpu_frame, (cpu_frame_p95, cpu_frame_p95_saturated)) = self.cpu_frame.take();
        let (gpu_frame, (gpu_frame_p95, gpu_frame_p95_saturated)) = self.gpu_frame.take();
        (
            FrameBreakdown {
                build: self.build.take().0,
                encode: self.encode.take().0,
                present: self.present.take().0,
            },
            FrameTimingStats {
                pipeline: pipeline::take(),
                cpu_frame,
                cpu_frame_p95,
                cpu_frame_p95_saturated,
                gpu_frame,
                gpu_frame_p95,
                gpu_frame_p95_saturated,
            },
        )
    }

    fn reset(&self) {
        self.build.reset();
        self.encode.reset();
        self.present.reset();
        #[cfg(feature = "frame-stats")]
        self.cpu_frame.reset();
        #[cfg(feature = "frame-stats")]
        self.gpu_frame.reset();
    }
}

#[cfg(feature = "frame-stats")]
const PERCENTILE_BUCKET_WIDTH_NANOS: u64 = 500_000;
#[cfg(feature = "frame-stats")]
const PERCENTILE_BUCKET_COUNT: usize = 512;

#[derive(Debug)]
struct AtomicPhase {
    samples: AtomicU64,
    nanos: AtomicU64,
    #[cfg(feature = "frame-stats")]
    percentile_buckets: [AtomicU64; PERCENTILE_BUCKET_COUNT],
}

impl Default for AtomicPhase {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(not(feature = "frame-stats"), allow(dead_code))]
impl AtomicPhase {
    const fn new() -> Self {
        Self {
            samples: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
            #[cfg(feature = "frame-stats")]
            percentile_buckets: [const { AtomicU64::new(0) }; PERCENTILE_BUCKET_COUNT],
        }
    }

    #[inline]
    fn record(&self, elapsed: Duration) {
        let nanos = elapsed.as_nanos().min(u64::MAX as u128) as u64;
        #[cfg(feature = "frame-stats")]
        let bucket = (nanos / PERCENTILE_BUCKET_WIDTH_NANOS)
            .min((PERCENTILE_BUCKET_COUNT - 1) as u64) as usize;
        #[cfg(feature = "frame-stats")]
        self.percentile_buckets[bucket].fetch_add(1, Ordering::Relaxed);
        // Relaxed is enough: the counters are statistics, not a happens-before
        // edge for any other data, and a snapshot that catches a phase mid-update
        // is off by at most one frame.
        self.samples.fetch_add(1, Ordering::Relaxed);
        self.nanos.fetch_add(nanos, Ordering::Relaxed);
    }

    fn snapshot(&self) -> PhaseSamples {
        PhaseSamples {
            samples: self.samples.load(Ordering::Relaxed),
            total: Duration::from_nanos(self.nanos.load(Ordering::Relaxed)),
        }
    }

    #[cfg(feature = "frame-stats")]
    fn p95_bucket(&self) -> (Duration, bool) {
        let sample_count = self.samples.load(Ordering::Relaxed);
        if sample_count == 0 {
            return (Duration::ZERO, false);
        }
        let rank = sample_count.saturating_mul(95).saturating_add(99) / 100;
        let mut cumulative = 0_u64;
        for (index, bucket) in self.percentile_buckets.iter().enumerate() {
            cumulative = cumulative.saturating_add(bucket.load(Ordering::Relaxed));
            if cumulative >= rank {
                let upper_bound = (index as u64 + 1) * PERCENTILE_BUCKET_WIDTH_NANOS;
                return (
                    Duration::from_nanos(upper_bound),
                    index + 1 == PERCENTILE_BUCKET_COUNT,
                );
            }
        }
        (
            Duration::from_nanos(PERCENTILE_BUCKET_COUNT as u64 * PERCENTILE_BUCKET_WIDTH_NANOS),
            true,
        )
    }

    fn take(&self) -> (PhaseSamples, (Duration, bool)) {
        // Each counter is drained with a swap, so a concurrent worker sample
        // belongs to this or the following window rather than being erased.
        let samples = PhaseSamples {
            samples: self.samples.swap(0, Ordering::Relaxed),
            total: Duration::from_nanos(self.nanos.swap(0, Ordering::Relaxed)),
        };
        #[cfg(feature = "frame-stats")]
        let percentile = {
            let buckets = std::array::from_fn::<_, PERCENTILE_BUCKET_COUNT, _>(|i| {
                self.percentile_buckets[i].swap(0, Ordering::Relaxed)
            });
            let count: u64 = buckets.iter().sum();
            let rank = count.saturating_mul(95).saturating_add(99) / 100;
            let mut cumulative = 0;
            let index = buckets.iter().position(|count| {
                cumulative += count;
                cumulative >= rank
            });
            if count == 0 {
                (Duration::ZERO, false)
            } else {
                let index = index.unwrap_or(PERCENTILE_BUCKET_COUNT - 1);
                (
                    Duration::from_nanos((index as u64 + 1) * PERCENTILE_BUCKET_WIDTH_NANOS),
                    index + 1 == PERCENTILE_BUCKET_COUNT,
                )
            }
        };
        #[cfg(not(feature = "frame-stats"))]
        let percentile = (Duration::ZERO, false);
        (samples, percentile)
    }

    fn reset(&self) {
        self.samples.store(0, Ordering::Relaxed);
        self.nanos.store(0, Ordering::Relaxed);
        #[cfg(feature = "frame-stats")]
        for bucket in &self.percentile_buckets {
            bucket.store(0, Ordering::Relaxed);
        }
    }
}

/// Lock-free accumulation for build-side frame content counters.
#[derive(Debug, Default)]
struct FrameContentAccumulator {
    frames: AtomicU64,
    drawn_nodes: AtomicU64,
    draw_commands: AtomicU64,
    retained_layers: AtomicU64,
    text_commands: AtomicU64,
    image_draws: AtomicU64,
    image_uploads: AtomicU64,
    text_cache_hits: AtomicU64,
    text_cache_misses: AtomicU64,
    rebuild_visits: AtomicU64,
    rebuild_pruned: AtomicU64,
    stateful_rebuild_checks: AtomicU64,
    stateless_rebuild_checks: AtomicU64,
    stateful_builds: AtomicU64,
    stateless_builds: AtomicU64,
    layout_calls: AtomicU64,
    hit_test_visits: AtomicU64,
    paint_calls: AtomicU64,
    root_draw_calls: AtomicU64,
    scroll_events: AtomicU64,
    scroll_steps: AtomicU64,
    smoothing_steps: AtomicU64,
    state_updates: AtomicU64,
    invalidations_queued: AtomicU64,
    invalidations_coalesced: AtomicU64,
    element_index_lookups: AtomicU64,
    stale_element_ids: AtomicU64,
    scroll_offset_updates: AtomicU64,
    redraw_requests: AtomicU64,
}

#[cfg_attr(not(feature = "frame-stats"), allow(dead_code))]
impl FrameContentAccumulator {
    #[inline]
    fn record(
        &self,
        drawn_nodes: u64,
        draw_list: aimer_cupid::draw_cmd::DrawListStats,
        text_cache_hits: u64,
        text_cache_misses: u64,
        rebuild: aimer_widget::RebuildStats,
        work: aimer_widget::FrameWorkStats,
    ) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.drawn_nodes.fetch_add(drawn_nodes, Ordering::Relaxed);
        self.draw_commands
            .fetch_add(draw_list.commands as u64, Ordering::Relaxed);
        self.retained_layers
            .fetch_add(draw_list.retained_layers as u64, Ordering::Relaxed);
        self.text_commands
            .fetch_add(draw_list.text_commands as u64, Ordering::Relaxed);
        self.image_draws
            .fetch_add(draw_list.image_draws as u64, Ordering::Relaxed);
        self.image_uploads
            .fetch_add(draw_list.image_uploads as u64, Ordering::Relaxed);
        self.text_cache_hits
            .fetch_add(text_cache_hits, Ordering::Relaxed);
        self.text_cache_misses
            .fetch_add(text_cache_misses, Ordering::Relaxed);
        self.rebuild_visits
            .fetch_add(rebuild.visits, Ordering::Relaxed);
        self.rebuild_pruned
            .fetch_add(rebuild.pruned, Ordering::Relaxed);
        self.stateful_rebuild_checks
            .fetch_add(rebuild.stateful_checks, Ordering::Relaxed);
        self.stateless_rebuild_checks
            .fetch_add(rebuild.stateless_checks, Ordering::Relaxed);
        self.stateful_builds
            .fetch_add(rebuild.stateful_builds, Ordering::Relaxed);
        self.stateless_builds
            .fetch_add(rebuild.stateless_builds, Ordering::Relaxed);
        self.layout_calls
            .fetch_add(work.layout_calls, Ordering::Relaxed);
        self.hit_test_visits
            .fetch_add(work.hit_test_visits, Ordering::Relaxed);
        self.paint_calls
            .fetch_add(work.paint_calls, Ordering::Relaxed);
        self.root_draw_calls
            .fetch_add(work.root_draw_calls, Ordering::Relaxed);
        self.scroll_events
            .fetch_add(work.scroll_events, Ordering::Relaxed);
        self.scroll_steps
            .fetch_add(work.scroll_steps, Ordering::Relaxed);
        self.smoothing_steps
            .fetch_add(work.smoothing_steps, Ordering::Relaxed);
        self.state_updates
            .fetch_add(work.state_updates, Ordering::Relaxed);
        self.invalidations_queued
            .fetch_add(work.invalidations_queued, Ordering::Relaxed);
        self.invalidations_coalesced
            .fetch_add(work.invalidations_coalesced, Ordering::Relaxed);
        self.element_index_lookups
            .fetch_add(work.element_index_lookups, Ordering::Relaxed);
        self.stale_element_ids
            .fetch_add(work.stale_element_ids, Ordering::Relaxed);
        self.scroll_offset_updates
            .fetch_add(work.scroll_offset_updates, Ordering::Relaxed);
        self.redraw_requests
            .fetch_add(work.redraw_requests, Ordering::Relaxed);
    }

    fn snapshot(&self) -> FrameContentStats {
        FrameContentStats {
            frames: self.frames.load(Ordering::Relaxed),
            drawn_nodes: self.drawn_nodes.load(Ordering::Relaxed),
            draw_commands: self.draw_commands.load(Ordering::Relaxed),
            retained_layers: self.retained_layers.load(Ordering::Relaxed),
            text_commands: self.text_commands.load(Ordering::Relaxed),
            image_draws: self.image_draws.load(Ordering::Relaxed),
            image_uploads: self.image_uploads.load(Ordering::Relaxed),
            text_cache_hits: self.text_cache_hits.load(Ordering::Relaxed),
            text_cache_misses: self.text_cache_misses.load(Ordering::Relaxed),
            rebuild_visits: self.rebuild_visits.load(Ordering::Relaxed),
            rebuild_pruned: self.rebuild_pruned.load(Ordering::Relaxed),
            stateful_rebuild_checks: self.stateful_rebuild_checks.load(Ordering::Relaxed),
            stateless_rebuild_checks: self.stateless_rebuild_checks.load(Ordering::Relaxed),
            stateful_builds: self.stateful_builds.load(Ordering::Relaxed),
            stateless_builds: self.stateless_builds.load(Ordering::Relaxed),
            layout_calls: self.layout_calls.load(Ordering::Relaxed),
            hit_test_visits: self.hit_test_visits.load(Ordering::Relaxed),
            paint_calls: self.paint_calls.load(Ordering::Relaxed),
            root_draw_calls: self.root_draw_calls.load(Ordering::Relaxed),
            scroll_events: self.scroll_events.load(Ordering::Relaxed),
            scroll_steps: self.scroll_steps.load(Ordering::Relaxed),
            smoothing_steps: self.smoothing_steps.load(Ordering::Relaxed),
            state_updates: self.state_updates.load(Ordering::Relaxed),
            invalidations_queued: self.invalidations_queued.load(Ordering::Relaxed),
            invalidations_coalesced: self.invalidations_coalesced.load(Ordering::Relaxed),
            element_index_lookups: self.element_index_lookups.load(Ordering::Relaxed),
            stale_element_ids: self.stale_element_ids.load(Ordering::Relaxed),
            scroll_offset_updates: self.scroll_offset_updates.load(Ordering::Relaxed),
            redraw_requests: self.redraw_requests.load(Ordering::Relaxed),
        }
    }

    fn reset(&self) {
        self.frames.store(0, Ordering::Relaxed);
        self.drawn_nodes.store(0, Ordering::Relaxed);
        self.draw_commands.store(0, Ordering::Relaxed);
        self.retained_layers.store(0, Ordering::Relaxed);
        self.text_commands.store(0, Ordering::Relaxed);
        self.image_draws.store(0, Ordering::Relaxed);
        self.image_uploads.store(0, Ordering::Relaxed);
        self.text_cache_hits.store(0, Ordering::Relaxed);
        self.text_cache_misses.store(0, Ordering::Relaxed);
        self.rebuild_visits.store(0, Ordering::Relaxed);
        self.rebuild_pruned.store(0, Ordering::Relaxed);
        self.stateful_rebuild_checks.store(0, Ordering::Relaxed);
        self.stateless_rebuild_checks.store(0, Ordering::Relaxed);
        self.stateful_builds.store(0, Ordering::Relaxed);
        self.stateless_builds.store(0, Ordering::Relaxed);
        self.layout_calls.store(0, Ordering::Relaxed);
        self.hit_test_visits.store(0, Ordering::Relaxed);
        self.paint_calls.store(0, Ordering::Relaxed);
        self.root_draw_calls.store(0, Ordering::Relaxed);
        self.scroll_events.store(0, Ordering::Relaxed);
        self.scroll_steps.store(0, Ordering::Relaxed);
        self.smoothing_steps.store(0, Ordering::Relaxed);
        self.state_updates.store(0, Ordering::Relaxed);
        self.invalidations_queued.store(0, Ordering::Relaxed);
        self.invalidations_coalesced.store(0, Ordering::Relaxed);
        self.element_index_lookups.store(0, Ordering::Relaxed);
        self.stale_element_ids.store(0, Ordering::Relaxed);
        self.scroll_offset_updates.store(0, Ordering::Relaxed);
        self.redraw_requests.store(0, Ordering::Relaxed);
    }
}

static FRAME_STATS: FrameAccumulator = FrameAccumulator {
    build: AtomicPhase::new(),
    encode: AtomicPhase::new(),
    present: AtomicPhase::new(),
    #[cfg(feature = "frame-stats")]
    cpu_frame: AtomicPhase::new(),
    #[cfg(feature = "frame-stats")]
    gpu_frame: AtomicPhase::new(),
};

static FRAME_CONTENT_STATS: FrameContentAccumulator = FrameContentAccumulator {
    frames: AtomicU64::new(0),
    drawn_nodes: AtomicU64::new(0),
    draw_commands: AtomicU64::new(0),
    retained_layers: AtomicU64::new(0),
    text_commands: AtomicU64::new(0),
    image_draws: AtomicU64::new(0),
    image_uploads: AtomicU64::new(0),
    text_cache_hits: AtomicU64::new(0),
    text_cache_misses: AtomicU64::new(0),
    rebuild_visits: AtomicU64::new(0),
    rebuild_pruned: AtomicU64::new(0),
    stateful_rebuild_checks: AtomicU64::new(0),
    stateless_rebuild_checks: AtomicU64::new(0),
    stateful_builds: AtomicU64::new(0),
    stateless_builds: AtomicU64::new(0),
    layout_calls: AtomicU64::new(0),
    hit_test_visits: AtomicU64::new(0),
    paint_calls: AtomicU64::new(0),
    root_draw_calls: AtomicU64::new(0),
    scroll_events: AtomicU64::new(0),
    scroll_steps: AtomicU64::new(0),
    smoothing_steps: AtomicU64::new(0),
    state_updates: AtomicU64::new(0),
    invalidations_queued: AtomicU64::new(0),
    invalidations_coalesced: AtomicU64::new(0),
    element_index_lookups: AtomicU64::new(0),
    stale_element_ids: AtomicU64::new(0),
    scroll_offset_updates: AtomicU64::new(0),
    redraw_requests: AtomicU64::new(0),
};

static FRAME_REQUEST_STATS: FrameRequestAccumulator = FrameRequestAccumulator {
    accepted: AtomicU64::new(0),
    coalesced: AtomicU64::new(0),
    display_ticks: AtomicU64::new(0),
};

#[cfg(feature = "frame-stats")]
const DEBUG_REPORT_INTERVAL: u64 = 30;

/// The frame breakdown accumulated so far.
///
/// Every phase reads zero unless the crate was built with the `frame-stats`
/// feature.
#[inline]
pub fn frame_breakdown() -> FrameBreakdown {
    FRAME_STATS.snapshot()
}

/// Drop every sample collected so far.
///
/// Useful to exclude startup — the first frames pay for pipeline creation and
/// glyph atlas population, and are not representative of the steady state.
#[inline]
pub fn reset_frame_breakdown() {
    FRAME_STATS.reset();
}

/// The build-side content counters accumulated so far.
#[inline]
pub fn frame_content_stats() -> FrameContentStats {
    FRAME_CONTENT_STATS.snapshot()
}

/// Drop every build-side content sample collected so far.
#[inline]
pub fn reset_frame_content_stats() {
    FRAME_CONTENT_STATS.reset();
}

/// The native frame-wake counters accumulated so far.
#[inline]
pub fn frame_request_stats() -> FrameRequestStats {
    FRAME_REQUEST_STATS.snapshot()
}

/// Record one completed Metal GPU frame interval.
#[cfg(feature = "frame-stats")]
#[inline]
#[doc(hidden)]
pub fn record_gpu_frame_time(elapsed: Duration) {
    FRAME_STATS.gpu_frame.record(elapsed);
}

/// Drop every native frame-wake counter collected so far.
#[inline]
pub fn reset_frame_request_stats() {
    FRAME_REQUEST_STATS.reset();
}

/// Records a request that occupied the native pending frame-wake slot.
#[doc(hidden)]
#[inline]
pub fn record_frame_request_accepted() {
    #[cfg(feature = "frame-stats")]
    FRAME_REQUEST_STATS.accepted();
}

/// Records a request folded into an already pending native frame wake.
#[doc(hidden)]
#[inline]
pub fn record_frame_request_coalesced() {
    #[cfg(feature = "frame-stats")]
    FRAME_REQUEST_STATS.coalesced();
}

/// Records a delivered native display/event-loop tick.
#[doc(hidden)]
#[inline]
pub fn record_display_tick() {
    #[cfg(feature = "frame-stats")]
    FRAME_REQUEST_STATS.display_tick();
}

/// Records one frame's draw traversal and command-stream workload.
#[doc(hidden)]
#[inline]
pub fn record_frame_content(
    drawn_nodes: u64,
    draw_list: aimer_cupid::draw_cmd::DrawListStats,
    text_cache_hits: u64,
    text_cache_misses: u64,
    rebuild: aimer_widget::RebuildStats,
    work: aimer_widget::FrameWorkStats,
) {
    #[cfg(feature = "frame-stats")]
    FRAME_CONTENT_STATS.record(
        drawn_nodes,
        draw_list,
        text_cache_hits,
        text_cache_misses,
        rebuild,
        work,
    );
    #[cfg(not(feature = "frame-stats"))]
    let _ = (
        drawn_nodes,
        draw_list,
        text_cache_hits,
        text_cache_misses,
        rebuild,
        work,
    );
}

/// Takes a periodic debug report and resets the two frame accumulators.
///
/// The windowed handler calls this after a completed frame. Returning a report
/// every thirty build frames keeps the terminal useful during a long scroll
/// without adding a log call to the hot path itself.
#[doc(hidden)]
#[cfg(feature = "frame-stats")]
#[doc(hidden)]
pub fn take_debug_report() -> Option<(FrameBreakdown, FrameContentStats, FrameTimingStats)> {
    let content = FRAME_CONTENT_STATS.snapshot();
    if content.frames < DEBUG_REPORT_INTERVAL {
        return None;
    }
    FRAME_CONTENT_STATS.reset();
    let (breakdown, timing) = FRAME_STATS.take();
    Some((breakdown, content, timing))
}

/// A running measurement of one [`FramePhase`].
///
/// Zero-sized and inert unless the `frame-stats` feature is enabled, so the
/// render path can time itself unconditionally.
///
/// # Examples
///
/// ```
/// use aimer_frame_stats::{FramePhase, PhaseTimer};
///
/// let timer = PhaseTimer::start();
/// // ... build the frame ...
/// timer.finish(FramePhase::Build);
/// ```
#[derive(Debug)]
pub struct PhaseTimer {
    // `AnimInstant` rather than `aimer_time::Instant`: the web backend times the
    // same phases, and `std`'s clock is unsupported there.
    #[cfg(feature = "frame-stats")]
    started: aimer_utils::AnimInstant,
}

impl PhaseTimer {
    /// Begin timing a phase.
    #[inline]
    pub fn start() -> Self {
        Self {
            #[cfg(feature = "frame-stats")]
            started: aimer_utils::AnimInstant::now(),
        }
    }

    /// Attribute the elapsed time to `phase`.
    #[inline]
    pub fn finish(self, phase: FramePhase) {
        #[cfg(feature = "frame-stats")]
        FRAME_STATS.record(phase, self.started.elapsed());
        #[cfg(not(feature = "frame-stats"))]
        let _ = phase;
    }

    /// Records UI occupancy through recording and nonblocking submission.
    #[inline]
    #[doc(hidden)]
    pub fn finish_ui_frame(self) {
        self.finish_pipeline(pipeline::Phase::Ui);
    }
    /// Records time waiting for a worker to start the submitted frame.
    #[inline]
    #[doc(hidden)]
    pub fn finish_queue(self) {
        self.finish_pipeline(pipeline::Phase::Queue);
    }
    /// Records drawable acquisition separately from command encoding.
    #[inline]
    #[doc(hidden)]
    pub fn finish_acquire(self) {
        self.finish_pipeline(pipeline::Phase::Acquire);
    }
    fn finish_pipeline(self, phase: pipeline::Phase) {
        #[cfg(feature = "frame-stats")]
        pipeline::record(phase, self.started.elapsed());
        #[cfg(not(feature = "frame-stats"))]
        let _ = phase;
    }

    /// Record the elapsed CPU interval through native Metal command submission.
    #[inline]
    #[doc(hidden)]
    pub fn finish_cpu_frame(self) {
        #[cfg(feature = "frame-stats")]
        FRAME_STATS.cpu_frame.record(self.started.elapsed());
        #[cfg(not(feature = "frame-stats"))]
        let _ = self;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unsampled_phase_averages_to_zero() {
        assert_eq!(PhaseSamples::default().average(), Duration::ZERO);
    }

    #[test]
    fn a_phase_averages_over_its_samples() {
        let accumulator = FrameAccumulator::default();

        accumulator.record(FramePhase::Build, Duration::from_millis(4));
        accumulator.record(FramePhase::Build, Duration::from_millis(2));

        let build = accumulator.snapshot().build;
        assert_eq!(build.samples, 2);
        assert_eq!(build.total, Duration::from_millis(6));
        assert_eq!(build.average(), Duration::from_millis(3));
    }

    #[test]
    fn phases_are_accounted_separately() {
        let accumulator = FrameAccumulator::default();

        accumulator.record(FramePhase::Build, Duration::from_millis(6));
        accumulator.record(FramePhase::Encode, Duration::from_millis(1));
        accumulator.record(FramePhase::Present, Duration::from_millis(1));

        let breakdown = accumulator.snapshot();
        assert_eq!(breakdown.build.average(), Duration::from_millis(6));
        assert_eq!(breakdown.encode.average(), Duration::from_millis(1));
        assert_eq!(breakdown.present.average(), Duration::from_millis(1));
        assert_eq!(breakdown.average_frame(), Duration::from_millis(8));
    }

    #[test]
    fn a_dropped_frame_leaves_the_present_phase_unsampled() {
        let accumulator = FrameAccumulator::default();

        accumulator.record(FramePhase::Build, Duration::from_millis(5));
        accumulator.record(FramePhase::Encode, Duration::from_millis(1));

        let breakdown = accumulator.snapshot();
        assert_eq!(breakdown.build.samples, 1);
        assert_eq!(breakdown.present.samples, 0);
    }

    #[test]
    fn the_offloadable_share_is_everything_after_the_widget_walk() {
        let accumulator = FrameAccumulator::default();

        accumulator.record(FramePhase::Build, Duration::from_millis(6));
        accumulator.record(FramePhase::Encode, Duration::from_millis(1));
        accumulator.record(FramePhase::Present, Duration::from_millis(1));

        assert_eq!(accumulator.snapshot().offloadable_share(), 0.25);
    }

    #[test]
    fn frame_request_stats_distinguish_coalesced_wakes_from_display_ticks() {
        let accumulator = FrameRequestAccumulator::default();

        accumulator.accepted();
        accumulator.coalesced();
        accumulator.coalesced();
        accumulator.display_tick();

        assert_eq!(
            accumulator.snapshot(),
            FrameRequestStats {
                accepted: 1,
                coalesced: 2,
                display_ticks: 1,
            }
        );
    }

    #[test]
    fn an_empty_breakdown_has_nothing_to_offload() {
        assert_eq!(FrameBreakdown::default().offloadable_share(), 0.0);
    }

    #[test]
    fn resetting_drops_every_sample() {
        let accumulator = FrameAccumulator::default();
        accumulator.record(FramePhase::Build, Duration::from_millis(1));

        accumulator.reset();

        assert_eq!(accumulator.snapshot(), FrameBreakdown::default());
    }

    #[test]
    fn samples_from_several_threads_are_all_counted() {
        let shared = FrameAccumulator::default();

        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        shared.record(FramePhase::Encode, Duration::from_nanos(10));
                    }
                });
            }
        });

        let encode = shared.snapshot().encode;
        assert_eq!(encode.samples, 400);
        assert_eq!(encode.total, Duration::from_nanos(4000));
    }

    #[test]
    fn frame_content_accumulates_and_averages_work() {
        let accumulator = FrameContentAccumulator::default();

        accumulator.record(
            10,
            aimer_cupid::draw_cmd::DrawListStats {
                commands: 20,
                retained_layers: 3,
                text_commands: 8,
                image_draws: 2,
                image_uploads: 1,
            },
            4,
            1,
            aimer_widget::RebuildStats {
                visits: 4,
                pruned: 1,
                stateful_checks: 3,
                stateless_checks: 2,
                stateful_builds: 1,
                stateless_builds: 0,
            },
            aimer_widget::FrameWorkStats {
                layout_calls: 3,
                hit_test_visits: 4,
                paint_calls: 5,
                root_draw_calls: 1,
                scroll_events: 2,
                scroll_steps: 3,
                smoothing_steps: 1,
                state_updates: 2,
                scroll_offset_updates: 3,
                redraw_requests: 4,
                ..aimer_widget::FrameWorkStats::default()
            },
        );
        accumulator.record(
            6,
            aimer_cupid::draw_cmd::DrawListStats {
                commands: 10,
                retained_layers: 0,
                text_commands: 2,
                image_draws: 0,
                image_uploads: 3,
            },
            2,
            3,
            aimer_widget::RebuildStats {
                visits: 2,
                pruned: 0,
                stateful_checks: 1,
                stateless_checks: 1,
                stateful_builds: 0,
                stateless_builds: 1,
            },
            aimer_widget::FrameWorkStats {
                layout_calls: 1,
                hit_test_visits: 2,
                paint_calls: 3,
                root_draw_calls: 1,
                scroll_events: 1,
                scroll_steps: 2,
                smoothing_steps: 1,
                state_updates: 1,
                scroll_offset_updates: 2,
                redraw_requests: 3,
                ..aimer_widget::FrameWorkStats::default()
            },
        );
        let stats = accumulator.snapshot();
        assert_eq!(stats.frames, 2);
        assert_eq!(stats.drawn_nodes, 16);
        assert_eq!(stats.draw_commands, 30);
        assert_eq!(stats.retained_layers, 3);
        assert_eq!(stats.text_commands, 10);
        assert_eq!(stats.image_draws, 2);
        assert_eq!(stats.image_uploads, 4);
        assert_eq!(stats.text_cache_hits, 6);
        assert_eq!(stats.text_cache_misses, 4);
        assert_eq!(stats.rebuild_visits, 6);
        assert_eq!(stats.rebuild_pruned, 1);
        assert_eq!(stats.stateful_rebuild_checks, 4);
        assert_eq!(stats.stateless_rebuild_checks, 3);
        assert_eq!(stats.stateful_builds, 1);
        assert_eq!(stats.stateless_builds, 1);
        assert_eq!(stats.layout_calls, 4);
        assert_eq!(stats.hit_test_visits, 6);
        assert_eq!(stats.paint_calls, 8);
        assert_eq!(stats.root_draw_calls, 2);
        assert_eq!(stats.scroll_events, 3);
        assert_eq!(stats.scroll_steps, 5);
        assert_eq!(stats.smoothing_steps, 2);
        assert_eq!(stats.state_updates, 3);
        assert_eq!(stats.scroll_offset_updates, 5);
        assert_eq!(stats.redraw_requests, 7);
        assert_eq!(stats.average_drawn_nodes(), 8.0);
        assert_eq!(stats.average_draw_commands(), 15.0);
        assert_eq!(stats.average_retained_layers(), 1.5);
    }
}

#[cfg(test)]
mod drain_tests;
