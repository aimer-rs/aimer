//! Cross-platform time abstraction.
//!
//! Uses `aimer_time::Instant`, which delegates to the browser's monotonic clock
//! on ordinary WASM instead of the unsupported `std::time` clock.

use std::time::Duration;

/// A cross-platform instant in time.
///
/// Comparable and orderable, because state that records *when* something
/// happened is compared in tests — an instant derived from `now() - 600ms` is
/// how a five-hundred-millisecond threshold is exercised without a sleeping,
/// flaky test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnimInstant {
    inner: aimer_time::Instant,
}

impl AnimInstant {
    /// Capture the current monotonic time.
    pub fn now() -> Self {
        Self {
            inner: aimer_time::Instant::now(),
        }
    }

    /// Returns the duration elapsed since `earlier`.
    /// If `earlier` is after `self`, returns zero.
    pub fn duration_since(&self, earlier: AnimInstant) -> Duration {
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
        AnimInstant {
            inner: self.inner + rhs,
        }
    }
}

impl std::ops::Sub<Duration> for AnimInstant {
    type Output = AnimInstant;

    fn sub(self, rhs: Duration) -> Self::Output {
        AnimInstant {
            inner: self.inner - rhs,
        }
    }
}

impl std::ops::AddAssign<Duration> for AnimInstant {
    fn add_assign(&mut self, rhs: Duration) {
        self.inner += rhs;
    }
}

/// The current UTC time of day as `HH:MM:SS.mmm`.
///
/// For log lines that have to be lined up with what a person did and when:
/// instants from [`AnimInstant`] only measure how long, not when.
pub fn wall_clock_label() -> String {
    {
        let millis = aimer_time::DateTime::now().unix_timestamp_millis();
        format_wall_clock(millis.max(0) as u128)
    }
}

/// Formats `epoch_millis` (milliseconds since the Unix epoch) as the UTC time
/// of day, `HH:MM:SS.mmm`.
fn format_wall_clock(epoch_millis: u128) -> String {
    let millis_of_day = (epoch_millis % 86_400_000) as u64;
    let (hours, rest) = (millis_of_day / 3_600_000, millis_of_day % 3_600_000);
    let (minutes, rest) = (rest / 60_000, rest % 60_000);
    format!("{hours:02}:{minutes:02}:{:02}.{:03}", rest / 1000, rest % 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_wall_clock_label_is_the_utc_time_of_day_to_the_millisecond() {
        assert_eq!(format_wall_clock(0), "00:00:00.000");
        assert_eq!(format_wall_clock(3_723_456), "01:02:03.456");
        assert_eq!(
            format_wall_clock(86_400_000 + 3_723_456),
            "01:02:03.456",
            "the day it is does not show"
        );
        assert_eq!(format_wall_clock(86_399_999), "23:59:59.999");
    }

    #[test]
    fn the_live_wall_clock_label_has_the_same_shape() {
        let label = wall_clock_label();
        assert_eq!(label.len(), "00:00:00.000".len(), "{label}");
        assert_eq!(&label[2..3], ":");
        assert_eq!(&label[8..9], ".");
    }

    #[test]
    fn test_elapsed_returns_time_since_instant() {
        let earlier = AnimInstant::now();
        let later = earlier + Duration::from_millis(150);
        assert_eq!(earlier.elapsed_at(later), Duration::from_millis(150));
    }
}
