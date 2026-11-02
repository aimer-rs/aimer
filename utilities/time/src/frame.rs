use std::time::Duration;

use crate::Instant;

/// Weight of the newest frame in the smoothed FPS estimate.
const FPS_SMOOTHING: f64 = 0.1;

/// Per-frame timing: delta, total elapsed time, frame count and smoothed FPS.
///
/// Call [`tick`](FrameClock::tick) once per frame. The first tick reports a
/// zero delta, since there is no previous frame to measure against. A
/// [`max_delta`](FrameClock::max_delta) clamp keeps animations from jumping
/// after a stall such as a backgrounded window or a debugger pause.
#[derive(Debug, Clone)]
pub struct FrameClock {
    last: Option<Instant>,
    delta: Duration,
    elapsed: Duration,
    frames: u64,
    fps: f64,
    max_delta: Duration,
}

impl FrameClock {
    /// Creates a clock that has not ticked yet and does not clamp deltas.
    #[inline]
    pub fn new() -> Self {
        Self {
            last: None,
            delta: Duration::ZERO,
            elapsed: Duration::ZERO,
            frames: 0,
            fps: 0.0,
            max_delta: Duration::MAX,
        }
    }

    /// Clamps every reported delta to at most `max_delta`.
    #[inline]
    pub fn max_delta(mut self, max_delta: Duration) -> Self {
        self.max_delta = max_delta;
        self
    }

    /// Advances one frame using the real clock and returns the frame delta.
    #[inline]
    pub fn tick(&mut self) -> Duration {
        self.tick_at(Instant::now())
    }

    /// Advances one frame as of `now` and returns the frame delta.
    pub fn tick_at(&mut self, now: Instant) -> Duration {
        let raw = self.last.map_or(Duration::ZERO, |last| now.duration_since(last));
        let delta = raw.min(self.max_delta);
        self.last = Some(now);
        self.delta = delta;
        self.elapsed = self.elapsed.saturating_add(delta);
        self.frames += 1;
        if !delta.is_zero() {
            let instant_fps = 1.0 / delta.as_secs_f64();
            self.fps = if self.fps == 0.0 {
                instant_fps
            } else {
                self.fps + (instant_fps - self.fps) * FPS_SMOOTHING
            };
        }
        delta
    }

    /// The delta reported by the latest tick.
    #[inline]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// The latest delta in seconds, convenient for animation maths.
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    /// The sum of all (clamped) deltas since the first tick.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// The number of ticks so far.
    #[inline]
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// An exponentially smoothed frames-per-second estimate, `0.0` before the
    /// second tick.
    #[inline]
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// Forgets all history; the next tick reports a zero delta again.
    #[inline]
    pub fn reset(&mut self) {
        *self = Self {
            max_delta: self.max_delta,
            ..Self::new()
        };
    }
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Clock, ManualClock};

    #[test]
    fn first_tick_has_zero_delta() {
        let clock = ManualClock::new();
        let mut frames = FrameClock::new();
        assert_eq!(frames.tick_at(clock.now()), Duration::ZERO);
        assert_eq!(frames.frame_count(), 1);
        assert_eq!(frames.fps(), 0.0);
    }

    #[test]
    fn tracks_delta_elapsed_and_fps() {
        let clock = ManualClock::new();
        let mut frames = FrameClock::new();
        frames.tick_at(clock.now());
        for _ in 0..100 {
            clock.advance(Duration::from_millis(10));
            assert_eq!(frames.tick_at(clock.now()), Duration::from_millis(10));
        }
        assert_eq!(frames.elapsed(), Duration::from_secs(1));
        assert!((frames.fps() - 100.0).abs() < 1e-6);
        assert!((frames.delta_secs() - 0.01).abs() < 1e-6);
    }

    #[test]
    fn clamps_stalls() {
        let clock = ManualClock::new();
        let mut frames = FrameClock::new().max_delta(Duration::from_millis(50));
        frames.tick_at(clock.now());
        clock.advance(Duration::from_secs(5));
        assert_eq!(frames.tick_at(clock.now()), Duration::from_millis(50));
        assert_eq!(frames.elapsed(), Duration::from_millis(50));
    }

    #[test]
    fn reset_keeps_clamp_and_clears_history() {
        let clock = ManualClock::new();
        let mut frames = FrameClock::new().max_delta(Duration::from_millis(5));
        frames.tick_at(clock.now());
        clock.advance(Duration::from_secs(1));
        frames.tick_at(clock.now());
        frames.reset();
        assert_eq!(frames.frame_count(), 0);
        assert_eq!(frames.tick_at(clock.now()), Duration::ZERO);
        clock.advance(Duration::from_secs(1));
        assert_eq!(frames.tick_at(clock.now()), Duration::from_millis(5));
    }
}
