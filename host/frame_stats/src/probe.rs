//! Always-on counters for on-screen diagnostics.
//!
//! The phase timings in the rest of this crate are compiled out unless the
//! `frame-stats` feature is on, because they read a clock on every frame. The
//! overlay needs two numbers in every debug build, so they live here with a
//! cost that is near zero until something asks:
//!
//! - presented frames are one relaxed atomic increment at the point a frame
//!   reaches the screen;
//! - GPU frame time is only *collected* while at least one [`GpuProbeGuard`] is
//!   alive. Collecting it costs a completion handler per command buffer, so a
//!   hidden overlay must not pay for it.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// GPU frame intervals reported since the last [`take_gpu_frame_probe`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GpuFrameProbe {
    /// Frames whose GPU interval was reported.
    pub frames: u64,
    /// Sum of those intervals.
    pub total: Duration,
}

impl GpuFrameProbe {
    /// The mean GPU interval, or `None` when no frame was reported.
    #[inline]
    pub fn average(&self) -> Option<Duration> {
        let frames = u32::try_from(self.frames).ok().filter(|frames| *frames > 0);
        match frames {
            Some(frames) => Some(self.total / frames),
            None if self.frames == 0 => None,
            None => Some(Duration::from_nanos(
                (self.total.as_nanos() / u128::from(self.frames)) as u64,
            )),
        }
    }
}

/// The accumulator behind the global probe, separate so it can be tested
/// without sharing state between tests.
#[derive(Debug, Default)]
pub(crate) struct Probe {
    presented: AtomicU64,
    gpu_frames: AtomicU64,
    gpu_nanos: AtomicU64,
    watchers: AtomicUsize,
}

impl Probe {
    pub(crate) const fn new() -> Self {
        Self {
            presented: AtomicU64::new(0),
            gpu_frames: AtomicU64::new(0),
            gpu_nanos: AtomicU64::new(0),
            watchers: AtomicUsize::new(0),
        }
    }

    #[inline]
    pub(crate) fn record_presented(&self) {
        self.presented.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn presented(&self) -> u64 {
        self.presented.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn record_gpu(&self, elapsed: Duration) {
        // Saturates instead of wrapping: a corrupt timestamp must not turn into
        // a tiny average.
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        self.gpu_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.gpu_frames.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn take_gpu(&self) -> GpuFrameProbe {
        // `frames` is published last (Release) and read first (Acquire), so a
        // sample is never counted without its time. The pair can still straddle
        // a concurrent report, which only shifts that frame to the next take.
        let frames = self.gpu_frames.swap(0, Ordering::Acquire);
        let nanos = self.gpu_nanos.swap(0, Ordering::Relaxed);
        GpuFrameProbe {
            frames,
            total: Duration::from_nanos(nanos),
        }
    }

    #[inline]
    pub(crate) fn gpu_enabled(&self) -> bool {
        self.watchers.load(Ordering::Relaxed) > 0
    }
}

static PROBE: Probe = Probe::new();

/// Keeps GPU frame timing switched on while it is alive.
///
/// Guards count: timing stays on until the last one is dropped, so two windows
/// with a visible overlay each hold their own.
#[derive(Debug)]
#[must_use = "GPU timing is collected only while the guard is alive"]
pub struct GpuProbeGuard(());

/// Switches GPU frame timing on until the returned guard is dropped.
pub fn acquire_gpu_probe() -> GpuProbeGuard {
    PROBE.watchers.fetch_add(1, Ordering::Relaxed);
    GpuProbeGuard(())
}

impl Drop for GpuProbeGuard {
    fn drop(&mut self) {
        PROBE.watchers.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Whether anything currently wants GPU frame times.
///
/// The render path checks this once per frame before it starts a capture.
#[inline]
pub fn gpu_probe_enabled() -> bool {
    PROBE.gpu_enabled()
}

/// Counts one frame that reached the screen.
///
/// Called by the renderer at the moment of presentation, on whichever thread
/// presents.
#[doc(hidden)]
#[inline]
pub fn record_presented_frame() {
    PROBE.record_presented();
}

/// The number of frames presented since the process started.
///
/// Take two readings and divide the difference by the time between them for a
/// presented frame rate.
#[inline]
pub fn presented_frames() -> u64 {
    PROBE.presented()
}

/// Reports one completed GPU frame interval to the probe.
#[doc(hidden)]
#[inline]
pub(crate) fn record_gpu_probe(elapsed: Duration) {
    PROBE.record_gpu(elapsed);
}

/// Takes and resets the GPU frame intervals reported since the last call.
pub fn take_gpu_frame_probe() -> GpuFrameProbe {
    PROBE.take_gpu()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presented_frames_count_every_record() {
        let probe = Probe::new();
        assert_eq!(probe.presented(), 0);
        for _ in 0..5 {
            probe.record_presented();
        }
        assert_eq!(probe.presented(), 5);
    }

    #[test]
    fn gpu_probe_averages_and_resets_on_take() {
        let probe = Probe::new();
        assert_eq!(probe.take_gpu().average(), None, "no frame, no average");
        probe.record_gpu(Duration::from_micros(2_000));
        probe.record_gpu(Duration::from_micros(4_000));
        let taken = probe.take_gpu();
        assert_eq!(taken.frames, 2);
        assert_eq!(taken.average(), Some(Duration::from_micros(3_000)));
        assert_eq!(probe.take_gpu(), GpuFrameProbe::default());
    }

    #[test]
    fn gpu_probe_saturates_an_absurd_interval_instead_of_wrapping() {
        let probe = Probe::new();
        probe.record_gpu(Duration::MAX);
        let taken = probe.take_gpu();
        assert_eq!(taken.frames, 1);
        assert!(taken.average().unwrap() > Duration::from_secs(1));
    }

    #[test]
    fn average_of_more_frames_than_u32_does_not_divide_by_zero() {
        let probe = GpuFrameProbe {
            frames: u64::from(u32::MAX) + 1,
            total: Duration::from_secs(u64::from(u32::MAX) + 1),
        };
        assert_eq!(probe.average(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn gpu_timing_stays_on_until_the_last_guard_drops() {
        let probe = Probe::new();
        assert!(!probe.gpu_enabled());
        probe.watchers.fetch_add(2, Ordering::Relaxed);
        assert!(probe.gpu_enabled());
        probe.watchers.fetch_sub(1, Ordering::Relaxed);
        assert!(probe.gpu_enabled());
        probe.watchers.fetch_sub(1, Ordering::Relaxed);
        assert!(!probe.gpu_enabled());
    }

    #[test]
    fn guard_toggles_the_global_probe() {
        let before = gpu_probe_enabled();
        let guard = acquire_gpu_probe();
        assert!(gpu_probe_enabled());
        drop(guard);
        assert_eq!(gpu_probe_enabled(), before);
    }
}
