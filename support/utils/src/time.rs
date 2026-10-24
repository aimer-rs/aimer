//! Cross-platform time abstraction.
//!
//! Uses `web_time::Instant`, which delegates to the browser's monotonic clock
//! on ordinary WASM instead of the unsupported `std::time` clock. Portable
//! hot-reload guests use the host-provided logical frame clock so they do not
//! import browser APIs into the isolated interpreter.

use std::time::Duration;

#[cfg(any(aimer_portable_guest, test))]
use std::cell::Cell;

#[cfg(any(aimer_portable_guest, test))]
thread_local! {
    static PORTABLE_FRAME_TIME_NANOS: Cell<u64> = const { Cell::new(1_000_000_000) };
}

/// A cross-platform instant in time.
///
/// Comparable and orderable, because state that records *when* something
/// happened is compared in tests — an instant derived from `now() - 600ms` is
/// how a five-hundred-millisecond threshold is exercised without a sleeping,
/// flaky test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnimInstant {
    #[cfg(any(aimer_portable_guest, test))]
    inner: u64,
    #[cfg(not(any(aimer_portable_guest, test)))]
    inner: web_time::Instant,
}

impl AnimInstant {
    /// Capture the current monotonic time.
    pub fn now() -> Self {
        Self {
            #[cfg(any(aimer_portable_guest, test))]
            inner: PORTABLE_FRAME_TIME_NANOS.with(Cell::get),
            #[cfg(not(any(aimer_portable_guest, test)))]
            inner: web_time::Instant::now(),
        }
    }

    /// Returns the duration elapsed since `earlier`.
    /// If `earlier` is after `self`, returns zero.
    pub fn duration_since(&self, earlier: AnimInstant) -> Duration {
        #[cfg(any(aimer_portable_guest, test))]
        return Duration::from_nanos(self.inner.saturating_sub(earlier.inner));

        #[cfg(not(any(aimer_portable_guest, test)))]
        self.inner.duration_since(earlier.inner)
    }

    /// Returns the duration elapsed since this instant.
    pub fn elapsed(&self) -> Duration {
        self.elapsed_at(Self::now())
    }

    fn elapsed_at(&self, now: AnimInstant) -> Duration {
        now.duration_since(*self)
    }
}

impl std::ops::Add<Duration> for AnimInstant {
    type Output = AnimInstant;

    fn add(self, rhs: Duration) -> Self::Output {
        #[cfg(any(aimer_portable_guest, test))]
        return AnimInstant {
            inner: self.inner.saturating_add(duration_nanos(rhs)),
        };

        #[cfg(not(any(aimer_portable_guest, test)))]
        AnimInstant {
            inner: self.inner + rhs,
        }
    }
}

impl std::ops::Sub<Duration> for AnimInstant {
    type Output = AnimInstant;

    fn sub(self, rhs: Duration) -> Self::Output {
        #[cfg(any(aimer_portable_guest, test))]
        return AnimInstant {
            inner: self.inner.saturating_sub(duration_nanos(rhs)),
        };

        #[cfg(not(any(aimer_portable_guest, test)))]
        AnimInstant {
            inner: self.inner - rhs,
        }
    }
}

impl std::ops::AddAssign<Duration> for AnimInstant {
    fn add_assign(&mut self, rhs: Duration) {
        #[cfg(any(aimer_portable_guest, test))]
        {
            self.inner = self.inner.saturating_add(duration_nanos(rhs));
        }

        #[cfg(not(any(aimer_portable_guest, test)))]
        {
            self.inner += rhs;
        }
    }
}

#[cfg(any(aimer_portable_guest, test))]
#[inline]
fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

/// Installs the logical time associated with one portable guest build.
///
/// This is a no-op for native and ordinary browser builds. Keeping the symbol
/// available in every build configuration lets generated guest code share its
/// build-context implementation with the host crate graph, even when Cargo
/// compiles a dependency without the guest-only cfg.
#[doc(hidden)]
pub fn set_portable_frame_time(frame: u64) {
    #[cfg(any(aimer_portable_guest, test))]
    {
        const CLOCK_ORIGIN_NANOS: u64 = 1_000_000_000;
        const FRAME_NANOS: u64 = 16_000_000;
        PORTABLE_FRAME_TIME_NANOS.with(|time| {
            time.set(CLOCK_ORIGIN_NANOS.saturating_add(frame.saturating_mul(FRAME_NANOS)));
        });
    }

    #[cfg(not(any(aimer_portable_guest, test)))]
    let _ = frame;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_frame_clock_advances_between_guest_builds() {
        set_portable_frame_time(3);
        let first = AnimInstant::now();

        set_portable_frame_time(4);
        let second = AnimInstant::now();

        assert_eq!(second.duration_since(first), Duration::from_millis(16));
    }

    #[test]
    fn test_now_does_not_panic() {
        let _t = AnimInstant::now();
    }

    #[test]
    fn test_duration_since() {
        let a = AnimInstant::now();
        let b = a + Duration::from_millis(150);
        let dur = b.duration_since(a);
        assert_eq!(dur, Duration::from_millis(150));
    }

    #[test]
    fn test_duration_since_earlier_is_zero() {
        let b = AnimInstant::now();
        let a = b + Duration::from_millis(200);
        let dur = b.duration_since(a);
        assert_eq!(dur, Duration::ZERO);
    }

    #[test]
    fn test_add_duration() {
        let a = AnimInstant::now();
        let b = a + Duration::from_millis(50);
        assert_eq!(b.duration_since(a), Duration::from_millis(50));
    }

    #[test]
    fn test_sub_duration() {
        let b = AnimInstant::now();
        let a = b - Duration::from_millis(50);
        assert_eq!(b.duration_since(a), Duration::from_millis(50));
    }

    #[test]
    fn test_add_assign_duration() {
        let a = AnimInstant::now();
        let mut b = a;
        b += Duration::from_millis(50);
        assert_eq!(b.duration_since(a), Duration::from_millis(50));
    }

    #[test]
    fn an_earlier_instant_orders_before_a_later_one() {
        let earlier = AnimInstant::now();
        let later = earlier + Duration::from_millis(1);

        assert!(earlier < later);
        assert_eq!(earlier, earlier);
        assert_ne!(earlier, later);
    }

    #[test]
    fn test_now_returns_reasonable_value() {
        let earlier = AnimInstant::now();
        let later = AnimInstant::now();
        assert!(later.duration_since(earlier) < Duration::from_secs(1));
    }

    #[test]
    fn test_elapsed_returns_time_since_instant() {
        let earlier = AnimInstant::now();
        let later = earlier + Duration::from_millis(150);
        assert_eq!(earlier.elapsed_at(later), Duration::from_millis(150));
    }
}
