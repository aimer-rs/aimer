//! Turns raw counters into the numbers the overlay shows.
//!
//! The sampler owns no clock and reads no global: the caller says when it is
//! and supplies the readings, so the arithmetic (window rates, idle decay, the
//! present-frame accounting) is deterministic under test. The overlay is what
//! wires it to the real clock and the real counters.

use std::time::Duration;

use aimer_frame_stats::GpuFrameProbe;
use aimer_utils::AnimInstant;

use crate::process::{CpuMeter, ProcessSample};

/// How long a measurement window lasts, and so how often the numbers move.
pub(crate) const REFRESH_INTERVAL: Duration = Duration::from_millis(250);

/// Everything the sampler reads from outside, taken at one instant.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Readings {
    /// Frames presented since the process started.
    pub(crate) presented_frames: u64,
    /// GPU frame intervals reported since the previous reading.
    pub(crate) gpu: GpuFrameProbe,
    /// Process memory and CPU time.
    pub(crate) process: ProcessSample,
}

impl Readings {
    /// Reads the live counters. Takes (and so resets) the GPU probe.
    pub(crate) fn read() -> Self {
        Self {
            presented_frames: aimer_frame_stats::presented_frames(),
            gpu: aimer_frame_stats::take_gpu_frame_probe(),
            process: crate::process::sample(),
        }
    }
}

/// One closed measurement window, ready to display.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Metrics {
    /// Frames the application drew per second.
    pub(crate) draw_fps: f64,
    /// Frames that reached the screen per second, not counting the overlay's
    /// own refresh frames.
    pub(crate) present_fps: f64,
    /// Mean UI-thread time of one application frame, in milliseconds. `None`
    /// when the application drew nothing in the window.
    pub(crate) ui_ms: Option<f64>,
    /// Mean GPU time of one frame, in milliseconds. `None` when no frame was
    /// timed: the window was idle, or the backend does not report GPU time.
    pub(crate) gpu_ms: Option<f64>,
    /// Process CPU use in percent of one core.
    pub(crate) cpu_percent: Option<f64>,
    /// Process resident memory in bytes.
    pub(crate) resident_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: AnimInstant,
    presented_at_start: u64,
}

/// Accumulates frames between refreshes and closes a window into [`Metrics`].
#[derive(Debug, Default)]
pub(crate) struct Sampler {
    window: Option<Window>,
    app_frames: u64,
    ui_time: Duration,
    overlay_only_frames: u64,
    /// Overlay-only frames whose presentation has not been seen yet. A frame
    /// presented on the raster thread can be counted a window after it was
    /// drawn; this carries the debt across the boundary.
    owed_presents: u64,
    cpu: CpuMeter,
}

impl Sampler {
    /// When the open window began, or `None` before the first one.
    #[inline]
    pub(crate) fn window_started(&self) -> Option<AnimInstant> {
        self.window.map(|window| window.started)
    }

    /// Records one finished `draw`.
    ///
    /// `app_frame` is whether the application itself changed what is on screen.
    /// A frame that only exists because the overlay refreshed is not the
    /// application's frame: counting it would pin an idle application at the
    /// refresh rate.
    pub(crate) fn record_draw(&mut self, elapsed: Duration, app_frame: bool) {
        if app_frame {
            self.app_frames += 1;
            self.ui_time += elapsed;
        } else {
            self.overlay_only_frames += 1;
        }
    }

    /// Opens the first window, or closes the current one once
    /// [`REFRESH_INTERVAL`] has passed and opens the next.
    ///
    /// Returns `None` while there is nothing to report: on the opening call,
    /// and before the interval has elapsed. `read` is only called when a window
    /// opens or closes.
    pub(crate) fn advance(
        &mut self,
        now: AnimInstant,
        read: impl FnOnce() -> Readings,
    ) -> Option<Metrics> {
        let Some(window) = self.window else {
            let readings = read();
            self.open(now, &readings);
            // The first reading has nothing to compare with; it is the baseline
            // the first window's CPU use is measured from.
            self.cpu.update(now, readings.process.cpu_time);
            return None;
        };
        let elapsed = now.duration_since(window.started);
        if elapsed < REFRESH_INTERVAL {
            return None;
        }
        let readings = read();
        let seconds = elapsed.as_secs_f64();

        // Frames that reached the screen, minus the ones the overlay asked for
        // itself. The debt is paid as presents arrive, so a present that lands
        // a window after its draw still cancels out.
        let presented = readings
            .presented_frames
            .saturating_sub(window.presented_at_start);
        self.owed_presents += self.overlay_only_frames;
        let paid = self.owed_presents.min(presented);
        self.owed_presents -= paid;

        let metrics = Metrics {
            draw_fps: self.app_frames as f64 / seconds,
            present_fps: (presented - paid) as f64 / seconds,
            ui_ms: (self.app_frames > 0)
                .then(|| self.ui_time.as_secs_f64() * 1000.0 / self.app_frames as f64),
            gpu_ms: readings
                .gpu
                .average()
                .map(|average| average.as_secs_f64() * 1000.0),
            cpu_percent: self.cpu.update(now, readings.process.cpu_time),
            resident_bytes: readings.process.resident_bytes,
        };
        self.open(now, &readings);
        Some(metrics)
    }

    /// Starts a window at `now`: zeroes the counters and takes the baselines
    /// the next window is measured against.
    fn open(&mut self, now: AnimInstant, readings: &Readings) {
        self.window = Some(Window {
            started: now,
            presented_at_start: readings.presented_frames,
        });
        self.app_frames = 0;
        self.ui_time = Duration::ZERO;
        self.overlay_only_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn readings(presented: u64) -> Readings {
        Readings {
            presented_frames: presented,
            ..Readings::default()
        }
    }

    /// Opens a window at `t0` with `presented` frames already on the counter.
    fn opened(t0: AnimInstant, presented: u64) -> Sampler {
        let mut sampler = Sampler::default();
        assert_eq!(sampler.advance(t0, || readings(presented)), None);
        sampler
    }

    #[test]
    fn the_first_call_opens_a_window_and_reports_nothing() {
        let t0 = AnimInstant::now();
        let mut sampler = Sampler::default();
        let mut reads = 0;
        assert_eq!(
            sampler.advance(t0, || {
                reads += 1;
                readings(0)
            }),
            None
        );
        assert_eq!(reads, 1);
    }

    #[test]
    fn nothing_is_reported_or_read_before_the_interval_elapses() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        sampler.record_draw(ms(2), true);
        let mut reads = 0;
        assert_eq!(
            sampler.advance(t0 + ms(249), || {
                reads += 1;
                readings(0)
            }),
            None
        );
        assert_eq!(reads, 0, "a window that is still open reads nothing");
        let metrics = sampler.advance(t0 + ms(250), || readings(0)).unwrap();
        assert!((metrics.draw_fps - 4.0).abs() < 1e-9, "the early frame survived");
    }

    #[test]
    fn draw_fps_and_ui_time_count_only_application_frames() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        sampler.record_draw(ms(2), true);
        sampler.record_draw(ms(4), true);
        sampler.record_draw(ms(6), true);
        sampler.record_draw(ms(1), false);
        sampler.record_draw(ms(1), false);
        let metrics = sampler.advance(t0 + ms(250), || readings(0)).unwrap();
        assert!((metrics.draw_fps - 12.0).abs() < 1e-9, "{metrics:?}");
        assert!((metrics.ui_ms.unwrap() - 4.0).abs() < 1e-9, "{metrics:?}");
    }

    #[test]
    fn an_idle_window_decays_everything_to_zero_instead_of_holding_the_last_value() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        for _ in 0..10 {
            sampler.record_draw(ms(3), true);
        }
        let busy = sampler.advance(t0 + ms(250), || readings(10)).unwrap();
        assert!((busy.draw_fps - 40.0).abs() < 1e-9);
        assert!((busy.present_fps - 40.0).abs() < 1e-9);
        let idle = sampler.advance(t0 + ms(500), || readings(10)).unwrap();
        assert_eq!(idle.draw_fps, 0.0);
        assert_eq!(idle.present_fps, 0.0);
        assert_eq!(idle.ui_ms, None, "no frame, no mean");
    }

    #[test]
    fn a_late_refresh_divides_by_the_real_elapsed_time() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        for _ in 0..5 {
            sampler.record_draw(ms(1), true);
        }
        let metrics = sampler.advance(t0 + ms(1000), || readings(5)).unwrap();
        assert!((metrics.draw_fps - 5.0).abs() < 1e-9, "{metrics:?}");
        assert!((metrics.present_fps - 5.0).abs() < 1e-9, "{metrics:?}");
    }

    #[test]
    fn present_fps_excludes_the_overlays_own_refresh_frames() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 100);
        for _ in 0..3 {
            sampler.record_draw(ms(1), true);
        }
        sampler.record_draw(ms(1), false);
        // Four frames reached the screen: three of the application's, one the
        // overlay's.
        let metrics = sampler.advance(t0 + ms(250), || readings(104)).unwrap();
        assert!((metrics.present_fps - 12.0).abs() < 1e-9, "{metrics:?}");
    }

    #[test]
    fn a_refresh_frame_presented_a_window_late_is_still_not_counted() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        sampler.record_draw(ms(1), false);
        // Drawn in this window, but still on the raster thread at its close.
        let first = sampler.advance(t0 + ms(250), || readings(0)).unwrap();
        assert_eq!(first.present_fps, 0.0);
        let second = sampler.advance(t0 + ms(500), || readings(1)).unwrap();
        assert_eq!(second.present_fps, 0.0, "the late present pays the debt: {second:?}");
        sampler.record_draw(ms(1), true);
        let third = sampler.advance(t0 + ms(750), || readings(2)).unwrap();
        assert!((third.present_fps - 4.0).abs() < 1e-9, "{third:?}");
    }

    #[test]
    fn present_fps_never_goes_negative_when_more_frames_were_drawn_than_presented() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        for _ in 0..4 {
            sampler.record_draw(ms(1), false);
        }
        let metrics = sampler.advance(t0 + ms(250), || readings(1)).unwrap();
        assert_eq!(metrics.present_fps, 0.0);
    }

    #[test]
    fn gpu_time_is_the_mean_of_the_reported_frames_or_nothing() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        let with_gpu = Readings {
            gpu: GpuFrameProbe {
                frames: 4,
                total: ms(10),
            },
            ..Readings::default()
        };
        let metrics = sampler.advance(t0 + ms(250), || with_gpu).unwrap();
        assert!((metrics.gpu_ms.unwrap() - 2.5).abs() < 1e-9, "{metrics:?}");
        let metrics = sampler.advance(t0 + ms(500), Readings::default).unwrap();
        assert_eq!(metrics.gpu_ms, None);
    }

    #[test]
    fn cpu_percent_comes_from_the_change_in_process_cpu_time() {
        let t0 = AnimInstant::now();
        let mut sampler = Sampler::default();
        let at = |cpu_ms: u64| Readings {
            process: ProcessSample {
                resident_bytes: Some(10 << 20),
                cpu_time: Some(ms(cpu_ms)),
            },
            ..Readings::default()
        };
        assert_eq!(sampler.advance(t0, || at(1_000)), None);
        let metrics = sampler.advance(t0 + ms(250), || at(1_125)).unwrap();
        assert!((metrics.cpu_percent.unwrap() - 50.0).abs() < 1e-6, "{metrics:?}");
        assert_eq!(metrics.resident_bytes, Some(10 << 20));
    }

    #[test]
    fn a_platform_without_process_numbers_reports_none() {
        let t0 = AnimInstant::now();
        let mut sampler = opened(t0, 0);
        let metrics = sampler.advance(t0 + ms(250), Readings::default).unwrap();
        assert_eq!(metrics.cpu_percent, None);
        assert_eq!(metrics.resident_bytes, None);
    }
}
