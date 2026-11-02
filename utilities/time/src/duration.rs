use std::fmt;
use std::time::Duration;

/// Frequency helpers for [`Duration`].
pub trait DurationExt {
    /// The period of a signal ticking `hz` times per second.
    ///
    /// Returns [`Duration::ZERO`] when `hz` is not a positive finite number.
    fn from_hz(hz: f64) -> Duration;

    /// The frequency, in hertz, of a signal with this period.
    ///
    /// Returns `0.0` for a zero duration.
    fn as_hz(&self) -> f64;
}

impl DurationExt for Duration {
    #[inline]
    fn from_hz(hz: f64) -> Duration {
        if hz.is_finite() && hz > 0.0 {
            Duration::try_from_secs_f64(1.0 / hz).unwrap_or(Duration::MAX)
        } else {
            Duration::ZERO
        }
    }

    #[inline]
    fn as_hz(&self) -> f64 {
        let secs = self.as_secs_f64();
        if secs > 0.0 { 1.0 / secs } else { 0.0 }
    }
}

/// A compact, allocation-free [`Display`](fmt::Display) view of a duration.
///
/// Sub-second values print as `ns`, `µs` or `ms` with up to two decimals;
/// longer ones print as `1.50s`, `2m 05s`, `1h 02m 03s` or `3d 04h 05m 06s`.
/// Created by [`human`].
#[derive(Debug, Clone, Copy)]
pub struct HumanDuration(Duration);

/// Wraps `duration` for compact display.
#[inline]
pub fn human(duration: Duration) -> HumanDuration {
    HumanDuration(duration)
}

fn write_scaled(f: &mut fmt::Formatter<'_>, nanos: u32, unit_nanos: u32, unit: &str) -> fmt::Result {
    let whole = nanos / unit_nanos;
    let hundredths = (nanos % unit_nanos) as u64 * 100 / unit_nanos as u64;
    if hundredths == 0 {
        write!(f, "{whole}{unit}")
    } else {
        write!(f, "{whole}.{hundredths:02}{unit}")
    }
}

impl fmt::Display for HumanDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.0.as_secs();
        let nanos = self.0.subsec_nanos();
        if total == 0 {
            return match nanos {
                0..1_000 => write!(f, "{nanos}ns"),
                1_000..1_000_000 => write_scaled(f, nanos, 1_000, "µs"),
                _ => write_scaled(f, nanos, 1_000_000, "ms"),
            };
        }
        let (days, hours) = (total / 86_400, total / 3_600 % 24);
        let (minutes, seconds) = (total / 60 % 60, total % 60);
        if days > 0 {
            write!(f, "{days}d {hours:02}h {minutes:02}m {seconds:02}s")
        } else if hours > 0 {
            write!(f, "{hours}h {minutes:02}m {seconds:02}s")
        } else if minutes > 0 {
            write!(f, "{minutes}m {seconds:02}s")
        } else {
            let hundredths = nanos / 10_000_000;
            if hundredths == 0 {
                write!(f, "{seconds}s")
            } else {
                write!(f, "{seconds}.{hundredths:02}s")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hz_round_trip() {
        let period = Duration::from_hz(60.0);
        assert!((period.as_hz() - 60.0).abs() < 1e-3);
        assert_eq!(Duration::from_hz(0.0), Duration::ZERO);
        assert_eq!(Duration::from_hz(f64::NAN), Duration::ZERO);
        assert_eq!(Duration::from_hz(-5.0), Duration::ZERO);
        assert_eq!(Duration::ZERO.as_hz(), 0.0);
    }

    #[test]
    fn human_display() {
        assert_eq!(human(Duration::from_nanos(7)).to_string(), "7ns");
        assert_eq!(human(Duration::from_micros(1500)).to_string(), "1.50ms");
        assert_eq!(human(Duration::from_millis(250)).to_string(), "250ms");
        assert_eq!(human(Duration::from_millis(1500)).to_string(), "1.50s");
        assert_eq!(human(Duration::from_secs(5)).to_string(), "5s");
        assert_eq!(human(Duration::from_secs(125)).to_string(), "2m 05s");
        assert_eq!(human(Duration::from_secs(3723)).to_string(), "1h 02m 03s");
        assert_eq!(human(Duration::from_secs(273_906)).to_string(), "3d 04h 05m 06s");
        assert_eq!(human(Duration::ZERO).to_string(), "0ns");
    }
}
