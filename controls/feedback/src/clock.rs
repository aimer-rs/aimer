use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A monotonic time source used by feedback lifecycle models.
pub trait Clock {
    /// Returns elapsed time from an arbitrary, monotonic origin.
    fn now(&self) -> Duration;
}

/// A manually advanced clock useful for deterministic application and unit tests.
#[derive(Clone, Debug)]
pub struct ManualClock {
    now: Rc<Cell<Duration>>,
}

impl ManualClock {
    /// Creates a clock at elapsed time zero.
    #[inline]
    pub fn new() -> Self {
        Self {
            now: Rc::new(Cell::new(Duration::ZERO)),
        }
    }

    /// Sets the current elapsed time.
    #[inline]
    pub fn set(&self, now: Duration) {
        self.now.set(now);
    }

    /// Advances the clock without blocking the calling thread.
    #[inline]
    pub fn advance(&self, amount: Duration) {
        self.now.set(self.now.get().saturating_add(amount));
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    #[inline]
    fn now(&self) -> Duration {
        self.now.get()
    }
}

/// A production clock backed by [`Instant`].
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    started_at: Instant,
}

impl SystemClock {
    /// Starts a clock at the current monotonic instant.
    #[inline]
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    #[inline]
    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }
}
