//! Experimental minimum frame interval, keyed off the display refresh period.
//!
//! A frame is requested again as soon as the previous one completes, so a
//! window with continuing motion (scroll physics, an animation) renders as fast
//! as the swap chain hands out drawables. With `presentsWithTransaction` that
//! is faster than the display refreshes, and every frame past the refresh rate
//! costs a build, an encode and GPU work for an image nobody sees.
//!
//! Frames are admitted on a fixed grid of slots, one `period * percent / 100`
//! apart. A frame that arrives before its slot is refused, and the pacer
//! reports when to try again. A slot is counted from the previous *slot*, not
//! from when the previous frame happened to begin, so a timer that wakes the loop
//! late is made up by the next frame instead of slowing every frame after it.
//! The pacer is off unless `AIMER_FRAME_PACING_PERCENT` names a percentage, so
//! the same binary measures both behaviours; 100 is the display's own rate.

use std::time::{Duration, Instant};

/// How long a cached monitor refresh period stays valid. Reading it asks the
/// windowing system, which is not worth doing every frame.
const PERIOD_CACHE: Duration = Duration::from_secs(1);

/// Decides whether a frame may begin now or has to wait for the display.
#[derive(Debug, Default)]
pub(crate) struct FramePacer {
    /// Share of the refresh period that must pass between two frame starts.
    /// `None` disables pacing.
    percent: Option<u32>,
    /// The earliest instant the next frame may begin; `None` until one has.
    next_slot: Option<Instant>,
    #[cfg(feature = "frame-stats")]
    last_start: Option<Instant>,
    /// When the refused frame may be tried again; `None` when nothing waits.
    deadline: Option<Instant>,
    period: Option<(Instant, Option<Duration>)>,
    #[cfg(feature = "frame-stats")]
    intervals: IntervalStats,
}

impl FramePacer {
    /// Reads `AIMER_FRAME_PACING_PERCENT` once. Unset, empty or unparsable
    /// leaves pacing off.
    pub(crate) fn from_env() -> Self {
        let percent = std::env::var("AIMER_FRAME_PACING_PERCENT")
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|percent| *percent > 0);
        Self::with_percent(percent)
    }

    pub(crate) fn with_percent(percent: Option<u32>) -> Self {
        Self {
            percent,
            ..Self::default()
        }
    }

    /// The display refresh period, asking `query` (millihertz) at most once per
    /// [`PERIOD_CACHE`].
    pub(crate) fn period(
        &mut self,
        now: Instant,
        query: impl FnOnce() -> Option<u32>,
    ) -> Option<Duration> {
        if let Some((read, period)) = self.period
            && now.saturating_duration_since(read) < PERIOD_CACHE
        {
            return period;
        }
        let period = query()
            .filter(|millihertz| *millihertz > 0)
            .map(|millihertz| Duration::from_secs_f64(1000.0 / f64::from(millihertz)));
        self.period = Some((now, period));
        period
    }

    /// Returns `true` when a frame may begin at `now`, recording it as begun.
    ///
    /// Returns `false` when it started too soon after the previous one; the
    /// instant to try again is then [`Self::deadline`]. `bypass` admits the frame
    /// regardless, for work whose latency matters more than its rate, such as a
    /// live resize. A `None` period admits everything: without a refresh rate
    /// there is nothing to pace against.
    pub(crate) fn admit(&mut self, now: Instant, period: Option<Duration>, bypass: bool) -> bool {
        let interval = match (self.percent, period) {
            (Some(percent), Some(period)) => Some(period * percent / 100),
            _ => None,
        };
        if let (Some(slot), false) = (self.next_slot, bypass)
            && interval.is_some()
            && now < slot
        {
            self.deadline = Some(slot);
            return false;
        }
        #[cfg(feature = "frame-stats")]
        {
            if let Some(last) = self.last_start {
                self.intervals.record(now.duration_since(last), period);
            }
            self.last_start = Some(now);
        }
        self.next_slot = interval.map(|interval| match self.next_slot {
            // On the grid: one interval after the slot this frame took. A frame
            // that is more than an interval late — the window sat idle, or the
            // frame was bypassed — restarts the grid from now rather than
            // queueing a burst of catch-up frames.
            Some(slot) if !bypass && slot + interval > now => slot + interval,
            _ => now + interval,
        });
        self.deadline = None;
        true
    }

    /// When the frame refused by [`Self::admit`] may be tried again.
    #[inline]
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Returns `true`, and stops waiting, once the deadline has been reached.
    pub(crate) fn take_due(&mut self, now: Instant) -> bool {
        match self.deadline {
            Some(deadline) if now >= deadline => {
                self.deadline = None;
                true
            }
            _ => false,
        }
    }
}

/// Start-to-start frame intervals, summarised every [`IntervalStats::REPORT`]
/// frames to show whether pacing smooths or merely slows the loop.
#[cfg(feature = "frame-stats")]
#[derive(Debug, Default)]
struct IntervalStats {
    count: u32,
    total: Duration,
    min: Option<Duration>,
    max: Duration,
    /// Intervals under half a period: frames that cannot all reach the screen.
    bursts: u32,
    /// Intervals over one and a half periods: at least one refresh was missed.
    stalls: u32,
}

#[cfg(feature = "frame-stats")]
impl IntervalStats {
    const REPORT: u32 = 120;

    fn record(&mut self, interval: Duration, period: Option<Duration>) {
        self.count += 1;
        self.total += interval;
        self.min = Some(self.min.map_or(interval, |min| min.min(interval)));
        self.max = self.max.max(interval);
        if let Some(period) = period {
            self.bursts += u32::from(interval < period / 2);
            self.stalls += u32::from(interval > period * 3 / 2);
        }
        if self.count == Self::REPORT {
            let ms = |duration: Duration| duration.as_secs_f64() * 1_000.0;
            aimer_utils::debug!(
                "[frame-pacing] frames={} avg={:.2}ms ({:.0} fps) min={:.2}ms max={:.2}ms under-half-period={} over-1.5-periods={}",
                self.count,
                ms(self.total) / f64::from(self.count),
                f64::from(self.count) / self.total.as_secs_f64(),
                ms(self.min.unwrap_or_default()),
                ms(self.max),
                self.bursts,
                self.stalls,
            );
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ_120: Duration = Duration::from_nanos(8_333_333);

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn disabled_pacer_admits_every_frame() {
        let mut pacer = FramePacer::with_percent(None);
        let start = Instant::now();
        assert!(pacer.admit(start, Some(HZ_120), false));
        assert!(pacer.admit(start + ms(1), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), None);
    }

    #[test]
    fn first_frame_is_admitted() {
        let mut pacer = FramePacer::with_percent(Some(100));
        assert!(pacer.admit(Instant::now(), Some(HZ_120), false));
    }

    #[test]
    fn a_frame_before_the_refresh_period_waits_for_it() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        assert!(pacer.admit(start, Some(HZ_120), false));
        assert!(!pacer.admit(start + ms(2), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(start + HZ_120));
    }

    #[test]
    fn a_frame_at_the_deadline_is_admitted_and_clears_it() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        pacer.admit(start, Some(HZ_120), false);
        pacer.admit(start + ms(2), Some(HZ_120), false);
        assert!(pacer.admit(start + HZ_120, Some(HZ_120), false));
        assert_eq!(pacer.deadline(), None);
        // The next interval counts from the frame that began, not from the first.
        assert!(!pacer.admit(start + HZ_120 + ms(2), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(start + HZ_120 * 2));
    }

    #[test]
    fn a_late_wake_is_made_up_by_the_next_frame() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        assert!(pacer.admit(start, Some(HZ_120), false));
        // The timer fires 700 us after the slot it was set for.
        let late = start + HZ_120 + Duration::from_micros(700);
        assert!(pacer.admit(late, Some(HZ_120), false));
        // The next slot stays on the grid, so this frame has less than a full
        // period to wait and the average rate is the refresh rate, not slower.
        assert!(!pacer.admit(late + ms(1), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(start + HZ_120 * 2));
    }

    #[test]
    fn the_grid_restarts_after_the_window_has_been_idle() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        assert!(pacer.admit(start, Some(HZ_120), false));
        let later = start + Duration::from_secs(1);
        assert!(pacer.admit(later, Some(HZ_120), false));
        // Not a burst of catch-up frames: the next slot is one period from now.
        assert!(!pacer.admit(later + ms(1), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(later + HZ_120));
    }

    #[test]
    fn a_percentage_shortens_the_interval() {
        let mut pacer = FramePacer::with_percent(Some(50));
        let start = Instant::now();
        pacer.admit(start, Some(HZ_120), false);
        assert!(!pacer.admit(start + ms(2), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(start + HZ_120 / 2));
        assert!(pacer.admit(start + ms(5), Some(HZ_120), false));
    }

    #[test]
    fn a_bypassed_frame_is_admitted_and_restarts_the_interval() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        pacer.admit(start, Some(HZ_120), false);
        assert!(pacer.admit(start + ms(1), Some(HZ_120), true));
        assert!(!pacer.admit(start + ms(2), Some(HZ_120), false));
        assert_eq!(pacer.deadline(), Some(start + ms(1) + HZ_120));
    }

    #[test]
    fn an_unknown_refresh_rate_admits_every_frame() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        assert!(pacer.admit(start, None, false));
        assert!(pacer.admit(start + ms(1), None, false));
    }

    #[test]
    fn take_due_waits_for_the_deadline_and_fires_once() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        pacer.admit(start, Some(HZ_120), false);
        pacer.admit(start + ms(2), Some(HZ_120), false);
        assert!(!pacer.take_due(start + ms(3)));
        assert!(pacer.take_due(start + HZ_120));
        assert!(!pacer.take_due(start + HZ_120 + ms(1)));
        assert_eq!(pacer.deadline(), None);
    }

    #[test]
    fn the_refresh_period_is_read_once_per_cache_window() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        let reads = std::cell::Cell::new(0);
        let read = |millihertz| {
            reads.set(reads.get() + 1);
            Some(millihertz)
        };
        assert_eq!(pacer.period(start, || read(120_000)), Some(HZ_120));
        // A different answer inside the window is not asked for.
        assert_eq!(pacer.period(start + ms(500), || read(60_000)), Some(HZ_120));
        assert_eq!(reads.get(), 1);
        let sixty = pacer.period(start + ms(1_500), || read(60_000)).unwrap();
        assert!((sixty.as_secs_f64() - 1.0 / 60.0).abs() < 1e-6);
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn a_zero_or_missing_refresh_rate_is_unknown() {
        let mut pacer = FramePacer::with_percent(Some(100));
        let start = Instant::now();
        assert_eq!(pacer.period(start, || Some(0)), None);
        assert_eq!(pacer.period(start + ms(2_000), || None), None);
    }
}
