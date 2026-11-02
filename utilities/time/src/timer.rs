//! Poll-based timers.
//!
//! None of these spawn threads or allocate. Each operation has an `_at` form
//! taking the current [`Instant`] explicitly, which is what tests use with a
//! [`ManualClock`](crate::ManualClock), and a convenience form that reads the
//! real clock.

use std::time::Duration;

use crate::Instant;

/// Measures elapsed time across start/stop cycles.
#[derive(Debug, Clone, Default)]
pub struct Stopwatch {
    accumulated: Duration,
    running_since: Option<Instant>,
}

impl Stopwatch {
    /// Creates a stopped stopwatch at zero.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a stopwatch already running as of the real now.
    #[inline]
    pub fn started() -> Self {
        let mut watch = Self::new();
        watch.start();
        watch
    }

    /// Starts or resumes; does nothing if already running.
    #[inline]
    pub fn start(&mut self) {
        self.start_at(Instant::now());
    }

    /// Starts or resumes as of `now`; does nothing if already running.
    #[inline]
    pub fn start_at(&mut self, now: Instant) {
        if self.running_since.is_none() {
            self.running_since = Some(now);
        }
    }

    /// Pauses, keeping the elapsed time; does nothing if stopped.
    #[inline]
    pub fn stop(&mut self) {
        self.stop_at(Instant::now());
    }

    /// Pauses as of `now`, keeping the elapsed time; does nothing if stopped.
    #[inline]
    pub fn stop_at(&mut self, now: Instant) {
        if let Some(since) = self.running_since.take() {
            self.accumulated = self.accumulated.saturating_add(now.duration_since(since));
        }
    }

    /// Stops and clears the elapsed time.
    #[inline]
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Whether the stopwatch is currently running.
    #[inline]
    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Total running time as of the real now.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed_at(Instant::now())
    }

    /// Total running time as of `now`.
    #[inline]
    pub fn elapsed_at(&self, now: Instant) -> Duration {
        match self.running_since {
            Some(since) => self.accumulated.saturating_add(now.duration_since(since)),
            None => self.accumulated,
        }
    }
}

/// A one-shot deadline.
#[derive(Debug, Clone, Copy)]
pub struct Countdown {
    start: Instant,
    duration: Duration,
}

impl Countdown {
    /// Starts counting down `duration` from the real now.
    #[inline]
    pub fn new(duration: Duration) -> Self {
        Self::new_at(Instant::now(), duration)
    }

    /// Starts counting down `duration` from `now`.
    #[inline]
    pub fn new_at(now: Instant, duration: Duration) -> Self {
        Self { start: now, duration }
    }

    /// Restarts the full duration from `now`.
    #[inline]
    pub fn restart_at(&mut self, now: Instant) {
        self.start = now;
    }

    /// Time left as of `now`, zero once expired.
    #[inline]
    pub fn remaining_at(&self, now: Instant) -> Duration {
        self.duration.saturating_sub(now.duration_since(self.start))
    }

    /// Whether the deadline has passed as of `now`.
    #[inline]
    pub fn is_expired_at(&self, now: Instant) -> bool {
        now.duration_since(self.start) >= self.duration
    }

    /// Fraction elapsed in `0.0..=1.0`; a zero-length countdown is always `1.0`.
    #[inline]
    pub fn progress_at(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (now.duration_since(self.start).as_secs_f64() / self.duration.as_secs_f64()).min(1.0) as f32
    }

    /// Time left as of the real now.
    #[inline]
    pub fn remaining(&self) -> Duration {
        self.remaining_at(Instant::now())
    }

    /// Whether the deadline has passed as of the real now.
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.is_expired_at(Instant::now())
    }
}

/// Trailing-edge debounce: fires once, `delay` after the last trigger.
///
/// Typical use is a search box: [`trigger`](Debounce::trigger) on every
/// keystroke and run the query when [`poll`](Debounce::poll) returns `true`.
#[derive(Debug, Clone)]
pub struct Debounce {
    delay: Duration,
    last_trigger: Option<Instant>,
}

impl Debounce {
    /// Creates an idle debounce with the given quiet period.
    #[inline]
    pub fn new(delay: Duration) -> Self {
        Self { delay, last_trigger: None }
    }

    /// Records an event as of the real now, restarting the quiet period.
    #[inline]
    pub fn trigger(&mut self) {
        self.trigger_at(Instant::now());
    }

    /// Records an event as of `now`, restarting the quiet period.
    #[inline]
    pub fn trigger_at(&mut self, now: Instant) {
        self.last_trigger = Some(now);
    }

    /// Returns `true` exactly once after the quiet period has elapsed.
    #[inline]
    pub fn poll(&mut self) -> bool {
        self.poll_at(Instant::now())
    }

    /// Returns `true` exactly once after the quiet period has elapsed as of `now`.
    #[inline]
    pub fn poll_at(&mut self, now: Instant) -> bool {
        match self.last_trigger {
            Some(last) if now.duration_since(last) >= self.delay => {
                self.last_trigger = None;
                true
            }
            _ => false,
        }
    }

    /// Whether an event is waiting for its quiet period to end.
    #[inline]
    pub fn is_pending(&self) -> bool {
        self.last_trigger.is_some()
    }

    /// Drops a pending event.
    #[inline]
    pub fn cancel(&mut self) {
        self.last_trigger = None;
    }
}

/// Leading-edge throttle: lets one event through per `interval`.
#[derive(Debug, Clone)]
pub struct Throttle {
    interval: Duration,
    last_fire: Option<Instant>,
}

impl Throttle {
    /// Creates a throttle whose first event always passes.
    #[inline]
    pub fn new(interval: Duration) -> Self {
        Self { interval, last_fire: None }
    }

    /// Returns `true` if an event may run as of the real now.
    #[inline]
    pub fn try_fire(&mut self) -> bool {
        self.try_fire_at(Instant::now())
    }

    /// Returns `true`, and starts a new interval, if an event may run as of `now`.
    #[inline]
    pub fn try_fire_at(&mut self, now: Instant) -> bool {
        match self.last_fire {
            Some(last) if now.duration_since(last) < self.interval => false,
            _ => {
                self.last_fire = Some(now);
                true
            }
        }
    }

    /// Lets the next event through regardless of the interval.
    #[inline]
    pub fn reset(&mut self) {
        self.last_fire = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Clock, ManualClock};

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn stopwatch_accumulates_across_pauses() {
        let clock = ManualClock::new();
        let mut watch = Stopwatch::new();
        assert!(!watch.is_running());
        watch.start_at(clock.now());
        clock.advance(MS(100));
        watch.stop_at(clock.now());
        clock.advance(MS(500));
        assert_eq!(watch.elapsed_at(clock.now()), MS(100));
        watch.start_at(clock.now());
        watch.start_at(clock.now() + MS(9999));
        clock.advance(MS(50));
        assert_eq!(watch.elapsed_at(clock.now()), MS(150));
        watch.reset();
        assert_eq!(watch.elapsed_at(clock.now()), Duration::ZERO);
    }

    #[test]
    fn countdown_expires_and_reports_progress() {
        let clock = ManualClock::new();
        let mut countdown = Countdown::new_at(clock.now(), MS(200));
        assert!(!countdown.is_expired_at(clock.now()));
        clock.advance(MS(50));
        assert_eq!(countdown.remaining_at(clock.now()), MS(150));
        assert!((countdown.progress_at(clock.now()) - 0.25).abs() < 1e-6);
        clock.advance(MS(150));
        assert!(countdown.is_expired_at(clock.now()));
        assert_eq!(countdown.progress_at(clock.now()), 1.0);
        clock.advance(MS(1000));
        assert_eq!(countdown.remaining_at(clock.now()), Duration::ZERO);
        assert_eq!(countdown.progress_at(clock.now()), 1.0);
        countdown.restart_at(clock.now());
        assert!(!countdown.is_expired_at(clock.now()));
        assert_eq!(Countdown::new_at(clock.now(), Duration::ZERO).progress_at(clock.now()), 1.0);
    }

    #[test]
    fn debounce_fires_once_after_quiet_period() {
        let clock = ManualClock::new();
        let mut debounce = Debounce::new(MS(300));
        assert!(!debounce.poll_at(clock.now()));
        debounce.trigger_at(clock.now());
        assert!(debounce.is_pending());
        clock.advance(MS(200));
        debounce.trigger_at(clock.now());
        clock.advance(MS(200));
        assert!(!debounce.poll_at(clock.now()), "re-trigger restarts the period");
        clock.advance(MS(100));
        assert!(debounce.poll_at(clock.now()));
        assert!(!debounce.poll_at(clock.now()), "fires only once");
        assert!(!debounce.is_pending());
    }

    #[test]
    fn debounce_cancel_drops_pending_event() {
        let clock = ManualClock::new();
        let mut debounce = Debounce::new(MS(10));
        debounce.trigger_at(clock.now());
        debounce.cancel();
        clock.advance(MS(100));
        assert!(!debounce.poll_at(clock.now()));
    }

    #[test]
    fn throttle_passes_one_event_per_interval() {
        let clock = ManualClock::new();
        let mut throttle = Throttle::new(MS(100));
        assert!(throttle.try_fire_at(clock.now()));
        clock.advance(MS(99));
        assert!(!throttle.try_fire_at(clock.now()));
        clock.advance(MS(1));
        assert!(throttle.try_fire_at(clock.now()));
        throttle.reset();
        assert!(throttle.try_fire_at(clock.now()));
    }
}
