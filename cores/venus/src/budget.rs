//! Frame deadlines, and the governor that decides how much of one Venus may
//! spend.

use std::cell::Cell;
use std::time::Duration;

#[cfg(aimer_portable_guest)]
use std::ops::{Add, Sub};

#[cfg(aimer_portable_guest)]
aimer_tls::thread_local! {
    static PORTABLE_FRAME_TIME_NANOS: Cell<u64> = const { Cell::new(1_000_000_000) };
}

#[cfg(not(aimer_portable_guest))]
use web_time::Instant;

#[cfg(aimer_portable_guest)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);

#[cfg(aimer_portable_guest)]
impl Instant {
    fn now() -> Self {
        Self(PORTABLE_FRAME_TIME_NANOS.with(Cell::get))
    }

    fn elapsed(self) -> Duration {
        Self::now() - self
    }
}

#[cfg(aimer_portable_guest)]
impl Add<Duration> for Instant {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        Self(
            self.0
                .saturating_add(rhs.as_nanos().min(u64::MAX as u128) as u64),
        )
    }
}

#[cfg(aimer_portable_guest)]
impl Sub for Instant {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration::from_nanos(self.0.saturating_sub(rhs.0))
    }
}

pub(crate) fn set_portable_frame_time(frame: u64) {
    #[cfg(aimer_portable_guest)]
    {
        const CLOCK_ORIGIN_NANOS: u64 = 1_000_000_000;
        const FRAME_NANOS: u64 = 16_000_000;
        PORTABLE_FRAME_TIME_NANOS.with(|time| {
            time.set(CLOCK_ORIGIN_NANOS.saturating_add(frame.saturating_mul(FRAME_NANOS)));
        });
    }

    #[cfg(not(aimer_portable_guest))]
    let _ = frame;
}

/// The smallest slice of frame time worth handing to an idle task.
///
/// Below this there is no point starting: the poll itself, plus the clock read
/// that follows it, would be most of what is left. It doubles as the threshold
/// [`crate::yield_if_over_budget`] compares against, so a cooperating task and
/// the scheduler agree on when the frame is over.
pub const IDLE_SLICE_FLOOR: Duration = Duration::from_micros(250);

/// How long a single microtask may take before a `venus-debug` build complains.
///
/// A microtask is contractually allowed to mutate state, not to do work — it
/// runs inside the frame's critical path with no budget check in front of it.
/// One that takes longer than this is a stutter waiting to be reported by a
/// user instead of by the author.
pub const MICROTASK_BUDGET_WARNING: Duration = Duration::from_micros(1_000);

/// How much of the current frame is left to spend.
///
/// The number that matters is never the frame interval, it is what remains
/// after build, layout and paint. A budget is therefore a *deadline* captured
/// at a point in time, not a duration: everything asks it "how much is left
/// **now**".
///
/// `Copy`, sixteen bytes, and its own clock — reading it costs one
/// `Instant::now`, around twenty-five nanoseconds on a modern machine, which is
/// noise next to any task worth budgeting.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::FrameBudget;
///
/// let budget = FrameBudget::from_now(Duration::from_millis(4));
/// assert!(!budget.is_exhausted());
///
/// // A frame that has already overrun hands out nothing.
/// assert!(FrameBudget::exhausted().is_exhausted());
/// assert_eq!(FrameBudget::exhausted().time_remaining(), Duration::ZERO);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBudget {
    start: Instant,
    deadline: Instant,
}

impl FrameBudget {
    /// Time reserved for present, compositor handoff and OS scheduling jitter.
    ///
    /// None of that is Venus's to spend, and none of it is predictable, so a
    /// frame is treated as shorter than it is. Aiming at the full interval is
    /// how a scheduler that is technically within budget still drops frames.
    pub const DEFAULT_SAFETY_MARGIN: Duration = Duration::from_millis(1);

    /// A budget covering `frame_time` from `frame_start`, less `safety_margin`.
    #[inline]
    pub fn new(frame_start: Instant, frame_time: Duration, safety_margin: Duration) -> Self {
        Self {
            start: frame_start,
            deadline: frame_start + frame_time.saturating_sub(safety_margin),
        }
    }

    /// A budget of `available` starting now.
    ///
    /// The shape a caller wants after build and paint are already done: it
    /// already knows how much is left, not when the frame began.
    #[inline]
    pub fn from_now(available: Duration) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            deadline: now + available,
        }
    }

    /// A budget with nothing in it, which every gate refuses.
    #[inline]
    pub fn exhausted() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            deadline: now,
        }
    }

    /// How much of the budget is left, saturating at zero.
    #[inline]
    pub fn time_remaining(&self) -> Duration {
        let now = Instant::now();
        if now >= self.deadline {
            return Duration::ZERO;
        }
        self.deadline - now
    }

    /// Whether the deadline has passed.
    #[inline]
    pub fn is_exhausted(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Whether there is enough left to be worth starting anything.
    #[inline]
    pub fn has_room(&self) -> bool {
        self.time_remaining() > IDLE_SLICE_FLOOR
    }

    /// How long ago the budget's frame started.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        Instant::now() - self.start
    }

    /// When the budget runs out.
    #[inline]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
}

aimer_tls::thread_local! {
    /// The budget of the phase currently running on this thread.
    ///
    /// A running task cannot be handed a `&FrameBudget` — its future was
    /// written by a user, not by the scheduler — so the budget of the phase it
    /// runs in is published here instead. This is what makes
    /// [`crate::yield_if_over_budget`] callable from inside arbitrary async
    /// code, and it is per-thread because a budget belongs to one UI thread's
    /// frame.
    static ACTIVE_BUDGET: Cell<Option<FrameBudget>> = const { Cell::new(None) };
}

/// Publishes `budget` for the duration of `f`, restoring whatever was there.
pub(crate) fn with_active<R>(budget: &FrameBudget, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<FrameBudget>);

    impl Drop for Restore {
        fn drop(&mut self) {
            ACTIVE_BUDGET.with(|active| active.set(self.0));
        }
    }

    let _restore = Restore(ACTIVE_BUDGET.with(|active| active.replace(Some(*budget))));
    f()
}

/// How much of the current frame's phase is left, from inside a running task.
///
/// `None` outside a budgeted phase — a microtask, or code not running under
/// Venus at all — which callers read as "not budgeted", never as "no time".
///
/// # Examples
///
/// ```
/// use aimer_venus::time_remaining_in_frame;
///
/// // Called from ordinary code, there is no frame to report on.
/// assert!(time_remaining_in_frame().is_none());
/// ```
#[inline]
pub fn time_remaining_in_frame() -> Option<Duration> {
    ACTIVE_BUDGET.with(|active| active.get().map(|budget| budget.time_remaining()))
}

/// Decides how much idle time each frame may spend, based on the last one.
///
/// Predicting what a task will cost is guesswork; noticing that the previous
/// frame overran is a measurement. A frame following an overrun spends nothing
/// on idle work, which lets the pipeline recover instead of compounding — one
/// frame of history turns out to be a far more stable governor than any
/// estimate of task cost.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::FrameGovernor;
///
/// let mut governor = FrameGovernor::for_refresh_rate(120.0);
/// assert!(governor.frame_time() < Duration::from_micros(8_400));
///
/// governor.begin_frame();
/// assert!(!governor.previous_frame_overran());
/// ```
#[derive(Debug, Clone)]
pub struct FrameGovernor {
    frame_time: Duration,
    safety_margin: Duration,
    frame_start: Instant,
    previous_frame_overran: bool,
}

impl FrameGovernor {
    /// A governor for a display refreshing every `frame_time`.
    #[inline]
    pub fn new(frame_time: Duration) -> Self {
        Self {
            frame_time,
            safety_margin: FrameBudget::DEFAULT_SAFETY_MARGIN,
            frame_start: Instant::now(),
            previous_frame_overran: false,
        }
    }

    /// A governor for a display refreshing `hz` times per second.
    #[inline]
    pub fn for_refresh_rate(hz: f32) -> Self {
        debug_assert!(hz > 0.0, "a refresh rate must be positive");
        Self::new(Duration::from_secs_f32(1.0 / hz))
    }

    /// Overrides the time reserved for present and platform jitter.
    #[inline]
    pub fn safety_margin(mut self, safety_margin: Duration) -> Self {
        self.safety_margin = safety_margin;
        self
    }

    /// Retunes the governor for a display refreshing `hz` times per second.
    ///
    /// A runtime is built before anyone knows what it will be drawing on, so the
    /// rate it starts with is a guess. Leaving that guess in place is not
    /// harmless: a governor that believes a 120 Hz frame is 16.6 ms hands out
    /// twice the idle time the frame actually has, and does not count a frame as
    /// overrun until two of them have already been missed.
    ///
    /// A non-positive or non-finite rate is ignored, because a platform that
    /// cannot report its refresh rate should leave the current one alone rather
    /// than divide by it.
    #[inline]
    pub fn set_refresh_rate(&mut self, hz: f32) {
        if hz.is_finite() && hz > 0.0 {
            self.frame_time = Duration::from_secs_f32(1.0 / hz);
        }
    }

    /// Marks the start of a frame.
    #[inline]
    pub fn begin_frame(&mut self) {
        self.frame_start = Instant::now();
    }

    /// The budget for this frame's idle phase.
    ///
    /// Empty when the previous frame overran, so recovery takes precedence over
    /// background work.
    #[inline]
    pub fn idle_budget(&self) -> FrameBudget {
        if self.previous_frame_overran {
            return FrameBudget::exhausted();
        }
        FrameBudget::new(self.frame_start, self.frame_time, self.safety_margin)
    }

    /// Marks the end of a frame, recording whether it overran.
    #[inline]
    pub fn end_frame(&mut self) {
        self.previous_frame_overran = self.frame_start.elapsed() > self.frame_time;
    }

    /// Whether the frame before this one missed its deadline.
    #[inline]
    pub const fn previous_frame_overran(&self) -> bool {
        self.previous_frame_overran
    }

    /// The display's frame interval.
    #[inline]
    pub const fn frame_time(&self) -> Duration {
        self.frame_time
    }
}

impl Default for FrameGovernor {
    /// A governor for a 60 Hz display.
    #[inline]
    fn default() -> Self {
        Self::for_refresh_rate(60.0)
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[test]
    fn a_budget_reports_zero_once_its_deadline_passes() {
        let budget = FrameBudget::from_now(Duration::from_millis(1));
        assert!(budget.time_remaining() > Duration::ZERO);

        thread::sleep(Duration::from_millis(2));

        assert_eq!(budget.time_remaining(), Duration::ZERO);
        assert!(budget.is_exhausted());
        assert!(!budget.has_room());
    }

    #[test]
    fn the_safety_margin_comes_off_the_frame() {
        let start = Instant::now();
        let full = FrameBudget::new(start, Duration::from_millis(8), Duration::ZERO);
        let margined = FrameBudget::new(start, Duration::from_millis(8), Duration::from_millis(1));

        assert_eq!(
            full.deadline() - margined.deadline(),
            Duration::from_millis(1)
        );
    }

    // A margin larger than the frame must not underflow the deadline; it simply
    // means there is no room at all.
    #[test]
    fn an_oversized_safety_margin_leaves_an_empty_budget() {
        let budget = FrameBudget::new(
            Instant::now(),
            Duration::from_millis(2),
            Duration::from_millis(8),
        );

        assert!(budget.is_exhausted());
    }

    #[test]
    fn a_budget_is_only_visible_while_it_is_active() {
        assert!(time_remaining_in_frame().is_none());

        let budget = FrameBudget::from_now(Duration::from_millis(4));
        with_active(&budget, || {
            assert!(time_remaining_in_frame().is_some_and(|left| left > Duration::ZERO));
        });

        assert!(time_remaining_in_frame().is_none());
    }

    // A governor built for 60 Hz and left there on a 120 Hz display hands out
    // twice the time the frame has. Retuning is what keeps the budget and the
    // overrun threshold honest.
    #[test]
    fn retuning_the_refresh_rate_shortens_the_frame_and_its_budget() {
        let mut governor = FrameGovernor::for_refresh_rate(60.0);
        governor.begin_frame();
        let at_sixty = governor.idle_budget().time_remaining();

        governor.set_refresh_rate(120.0);
        governor.begin_frame();
        let at_one_twenty = governor.idle_budget().time_remaining();

        assert!(governor.frame_time() < Duration::from_micros(8_400));
        assert!(
            at_one_twenty < at_sixty,
            "a shorter frame must hand out less: {at_one_twenty:?} vs {at_sixty:?}"
        );
    }

    // A platform that cannot answer must not be able to break the governor.
    #[test]
    fn an_unreportable_refresh_rate_leaves_the_frame_alone() {
        let mut governor = FrameGovernor::for_refresh_rate(120.0);
        let tuned = governor.frame_time();

        governor.set_refresh_rate(0.0);
        governor.set_refresh_rate(-60.0);
        governor.set_refresh_rate(f32::NAN);
        governor.set_refresh_rate(f32::INFINITY);

        assert_eq!(governor.frame_time(), tuned);
    }

    #[test]
    fn a_governor_withholds_idle_time_after_an_overrun() {
        let mut governor =
            FrameGovernor::new(Duration::from_millis(1)).safety_margin(Duration::ZERO);

        governor.begin_frame();
        assert!(governor.idle_budget().time_remaining() > Duration::ZERO);

        thread::sleep(Duration::from_millis(2));
        governor.end_frame();

        assert!(governor.previous_frame_overran());
        governor.begin_frame();
        assert!(governor.idle_budget().is_exhausted());

        governor.end_frame();
        assert!(!governor.previous_frame_overran());
        governor.begin_frame();
        assert!(!governor.idle_budget().is_exhausted());
    }
}
