use std::cell::Cell;
use std::time::Duration;

use crate::Instant;

/// A source of monotonic time.
///
/// Code that needs "now" can take a `&impl Clock`, which lets tests drive it
/// with a [`ManualClock`] instead of sleeping.
pub trait Clock {
    /// Returns the current instant.
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    #[inline]
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that only moves when told to.
///
/// It is anchored at the real instant it was created, so the instants it hands
/// out interoperate with every API in this crate.
#[derive(Debug)]
pub struct ManualClock {
    base: Instant,
    offset: Cell<Duration>,
}

impl ManualClock {
    /// Creates a clock frozen at the current real instant.
    #[inline]
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            offset: Cell::new(Duration::ZERO),
        }
    }

    /// Moves the clock forward by `by`.
    #[inline]
    pub fn advance(&self, by: Duration) {
        self.offset.set(self.offset.get().saturating_add(by));
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    #[inline]
    fn now(&self) -> Instant {
        self.base + self.offset.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_only_moves_when_advanced() {
        let clock = ManualClock::new();
        let start = clock.now();
        assert_eq!(clock.now(), start);
        clock.advance(Duration::from_millis(40));
        clock.advance(Duration::from_millis(2));
        assert_eq!(clock.now() - start, Duration::from_millis(42));
    }

    #[test]
    fn system_clock_is_monotonic() {
        let a = SystemClock.now();
        let b = SystemClock.now();
        assert!(b >= a);
    }
}
