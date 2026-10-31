use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::time::Duration;

use crate::sys;

/// A point on the monotonic clock.
///
/// Unlike [`std::time::Instant`] this works in the browser, where it reads
/// `performance.now()`. Subtraction saturates at zero instead of panicking, so a
/// stale instant can never crash a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(sys::Instant);

impl Instant {
    /// Reads the monotonic clock.
    #[inline]
    pub fn now() -> Self {
        Self(sys::Instant::now())
    }

    /// Returns the time from `earlier` to `self`, or zero if `earlier` is later.
    #[inline]
    pub fn duration_since(self, earlier: Instant) -> Duration {
        self.0.saturating_duration_since(earlier.0)
    }

    /// Same as [`Instant::duration_since`]; kept so code written against
    /// `std::time::Instant` migrates without edits.
    #[inline]
    pub fn saturating_duration_since(self, earlier: Instant) -> Duration {
        self.duration_since(earlier)
    }

    /// Returns the time from `earlier` to `self`, or `None` if `earlier` is later.
    #[inline]
    pub fn checked_duration_since(self, earlier: Instant) -> Option<Duration> {
        self.0.checked_duration_since(earlier.0)
    }

    /// Returns the time elapsed since this instant.
    #[inline]
    pub fn elapsed(self) -> Duration {
        Self::now().duration_since(self)
    }

    /// Returns `self + duration`, or `None` if it cannot be represented.
    #[inline]
    pub fn checked_add(self, duration: Duration) -> Option<Instant> {
        self.0.checked_add(duration).map(Self)
    }

    /// Returns `self - duration`, or `None` if it cannot be represented.
    #[inline]
    pub fn checked_sub(self, duration: Duration) -> Option<Instant> {
        self.0.checked_sub(duration).map(Self)
    }
}

/// On native targets the clock is `std::time::Instant`, so interop with APIs that
/// take the std type (windowing, I/O timeouts) is free.
#[cfg(not(target_arch = "wasm32"))]
impl From<std::time::Instant> for Instant {
    #[inline]
    fn from(value: std::time::Instant) -> Self {
        Self(value)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Instant> for std::time::Instant {
    #[inline]
    fn from(value: Instant) -> Self {
        value.0
    }
}

impl Add<Duration> for Instant {
    type Output = Instant;

    #[inline]
    fn add(self, rhs: Duration) -> Instant {
        Instant(self.0 + rhs)
    }
}

impl AddAssign<Duration> for Instant {
    #[inline]
    fn add_assign(&mut self, rhs: Duration) {
        self.0 += rhs;
    }
}

impl Sub<Duration> for Instant {
    type Output = Instant;

    #[inline]
    fn sub(self, rhs: Duration) -> Instant {
        Instant(self.0 - rhs)
    }
}

impl SubAssign<Duration> for Instant {
    #[inline]
    fn sub_assign(&mut self, rhs: Duration) {
        self.0 -= rhs;
    }
}

impl Sub<Instant> for Instant {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: Instant) -> Duration {
        self.duration_since(rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtraction_saturates() {
        let a = Instant::now();
        let b = a + Duration::from_secs(1);
        assert_eq!(a - b, Duration::ZERO);
        assert_eq!(b - a, Duration::from_secs(1));
        assert_eq!(a.checked_duration_since(b), None);
    }

    #[test]
    fn add_and_sub_round_trip() {
        let a = Instant::now();
        let step = Duration::from_millis(250);
        assert_eq!((a + step) - step, a);
        let mut b = a;
        b += step;
        b -= step;
        assert_eq!(a, b);
    }
}
