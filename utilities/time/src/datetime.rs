use std::fmt::{self, Write};
use std::str::FromStr;
use std::time::Duration;

use crate::calendar::{ParseError, take_byte, take_date, take_digits};
use crate::{Date, wall};

const SECONDS_PER_DAY: i64 = 86_400;
const NANOS_PER_SECOND: u32 = 1_000_000_000;

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A calendar date and time of day in UTC, with nanosecond precision.
///
/// Values order chronologically. The text form is ISO 8601,
/// `YYYY-MM-DDTHH:MM:SS[.fraction]Z`, and [`FromStr`] additionally accepts a
/// `±HH:MM` offset, which is converted to UTC. Leap seconds are not
/// represented: the second is always `0..=59`.
///
/// Local time zones are out of scope; convert at the edges of your program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    date: Date,
    second_of_day: u32,
    nanosecond: u32,
}

impl DateTime {
    /// The Unix epoch, 1970-01-01T00:00:00Z.
    pub const UNIX_EPOCH: DateTime = DateTime {
        date: Date::UNIX_EPOCH,
        second_of_day: 0,
        nanosecond: 0,
    };

    /// The current wall-clock time in UTC.
    ///
    /// Not monotonic: the system clock may be adjusted. Use
    /// [`Instant`](crate::Instant) to measure durations.
    pub fn now() -> DateTime {
        let (secs, nanos) = wall::unix_now();
        DateTime::from_unix_timestamp(secs, nanos).unwrap_or(DateTime::UNIX_EPOCH)
    }

    /// Creates a date-time, or `None` if any field is out of range.
    #[inline]
    pub fn new(date: Date, hour: u8, minute: u8, second: u8, nanosecond: u32) -> Option<DateTime> {
        if hour > 23 || minute > 59 || second > 59 || nanosecond >= NANOS_PER_SECOND {
            return None;
        }
        Some(DateTime {
            date,
            second_of_day: hour as u32 * 3_600 + minute as u32 * 60 + second as u32,
            nanosecond,
        })
    }

    /// Creates a date-time from Unix seconds plus a sub-second part.
    ///
    /// Returns `None` if `nanos` is a second or more, or the year does not fit
    /// `i32`.
    #[inline]
    pub fn from_unix_timestamp(secs: i64, nanos: u32) -> Option<DateTime> {
        if nanos >= NANOS_PER_SECOND {
            return None;
        }
        Some(DateTime {
            date: Date::from_days_since_epoch(secs.div_euclid(SECONDS_PER_DAY))?,
            second_of_day: secs.rem_euclid(SECONDS_PER_DAY) as u32,
            nanosecond: nanos,
        })
    }

    /// Whole seconds since the Unix epoch, rounded towards negative infinity.
    #[inline]
    pub fn unix_timestamp(self) -> i64 {
        self.date.days_since_epoch() * SECONDS_PER_DAY + self.second_of_day as i64
    }

    /// Milliseconds since the Unix epoch, rounded towards negative infinity.
    #[inline]
    pub fn unix_timestamp_millis(self) -> i64 {
        self.unix_timestamp() * 1_000 + (self.nanosecond / 1_000_000) as i64
    }

    /// The calendar date.
    #[inline]
    pub fn date(self) -> Date {
        self.date
    }

    /// The hour, `0..=23`.
    #[inline]
    pub fn hour(self) -> u8 {
        (self.second_of_day / 3_600) as u8
    }

    /// The minute, `0..=59`.
    #[inline]
    pub fn minute(self) -> u8 {
        (self.second_of_day / 60 % 60) as u8
    }

    /// The second, `0..=59`.
    #[inline]
    pub fn second(self) -> u8 {
        (self.second_of_day % 60) as u8
    }

    /// The nanosecond within the second, `0..1_000_000_000`.
    #[inline]
    pub fn nanosecond(self) -> u32 {
        self.nanosecond
    }

    /// Adds `duration`, or `None` on overflow.
    pub fn checked_add(self, duration: Duration) -> Option<DateTime> {
        let mut nanos = self.nanosecond + duration.subsec_nanos();
        let mut secs = self.unix_timestamp().checked_add_unsigned(duration.as_secs())?;
        if nanos >= NANOS_PER_SECOND {
            nanos -= NANOS_PER_SECOND;
            secs = secs.checked_add(1)?;
        }
        DateTime::from_unix_timestamp(secs, nanos)
    }

    /// Subtracts `duration`, or `None` on overflow.
    pub fn checked_sub(self, duration: Duration) -> Option<DateTime> {
        let mut secs = self.unix_timestamp().checked_sub_unsigned(duration.as_secs())?;
        let nanos = match self.nanosecond.checked_sub(duration.subsec_nanos()) {
            Some(nanos) => nanos,
            None => {
                secs = secs.checked_sub(1)?;
                self.nanosecond + NANOS_PER_SECOND - duration.subsec_nanos()
            }
        };
        DateTime::from_unix_timestamp(secs, nanos)
    }

    /// The time from `earlier` to `self`, or `None` if `earlier` is later.
    pub fn checked_duration_since(self, earlier: DateTime) -> Option<Duration> {
        if self < earlier {
            return None;
        }
        let mut secs = (self.unix_timestamp() - earlier.unix_timestamp()) as u64;
        let nanos = match self.nanosecond.checked_sub(earlier.nanosecond) {
            Some(nanos) => nanos,
            None => {
                secs -= 1;
                self.nanosecond + NANOS_PER_SECOND - earlier.nanosecond
            }
        };
        Some(Duration::new(secs, nanos))
    }

    /// Formats with `strftime`-style specifiers.
    ///
    /// | Specifier | Meaning |
    /// | --- | --- |
    /// | `%Y` `%y` | year, two-digit year |
    /// | `%m` `%d` `%e` | month, day (`01`), day padded with a space |
    /// | `%B` `%b` | full and abbreviated month name |
    /// | `%A` `%a` | full and abbreviated weekday name |
    /// | `%j` | day of the year, `001..=366` |
    /// | `%H` `%I` `%p` | 24-hour, 12-hour, `AM`/`PM` |
    /// | `%M` `%S` | minute, second |
    /// | `%f` | milliseconds, three digits |
    /// | `%%` | a literal `%` |
    ///
    /// An unknown specifier is copied through unchanged.
    pub fn format(self, pattern: &str) -> String {
        let mut out = String::with_capacity(pattern.len() + 16);
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                out.push(ch);
                continue;
            }
            let Some(spec) = chars.next() else {
                out.push('%');
                break;
            };
            let date = self.date;
            let hour12 = match self.hour() % 12 {
                0 => 12,
                hour => hour,
            };
            // Writing to a `String` cannot fail.
            let _ = match spec {
                'Y' => write!(out, "{}", date.year()),
                'y' => write!(out, "{:02}", date.year().rem_euclid(100)),
                'm' => write!(out, "{:02}", date.month()),
                'd' => write!(out, "{:02}", date.day()),
                'e' => write!(out, "{:>2}", date.day()),
                'B' => write!(out, "{}", MONTH_NAMES[date.month() as usize - 1]),
                'b' => write!(out, "{}", &MONTH_NAMES[date.month() as usize - 1][..3]),
                'A' => write!(out, "{}", date.weekday().name()),
                'a' => write!(out, "{}", date.weekday().short_name()),
                'j' => write!(out, "{:03}", date.ordinal()),
                'H' => write!(out, "{:02}", self.hour()),
                'I' => write!(out, "{hour12:02}"),
                'p' => write!(out, "{}", if self.hour() < 12 { "AM" } else { "PM" }),
                'M' => write!(out, "{:02}", self.minute()),
                'S' => write!(out, "{:02}", self.second()),
                'f' => write!(out, "{:03}", self.nanosecond / 1_000_000),
                '%' => write!(out, "%"),
                other => write!(out, "%{other}"),
            };
        }
        out
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}T{:02}:{:02}:{:02}",
            self.date,
            self.hour(),
            self.minute(),
            self.second()
        )?;
        if self.nanosecond != 0 {
            let mut fraction = format!("{:09}", self.nanosecond);
            while fraction.ends_with('0') {
                fraction.pop();
            }
            write!(f, ".{fraction}")?;
        }
        f.write_char('Z')
    }
}

impl FromStr for DateTime {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<DateTime, ParseError> {
        const EXPECTED: &str = "YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)";
        let fail = || ParseError::new(EXPECTED);
        let mut bytes = text.as_bytes();

        let date = take_date(&mut bytes).ok_or_else(fail)?;
        take_byte(&mut bytes, b'T').ok_or_else(fail)?;
        let hour = take_digits(&mut bytes, 2).ok_or_else(fail)? as u8;
        take_byte(&mut bytes, b':').ok_or_else(fail)?;
        let minute = take_digits(&mut bytes, 2).ok_or_else(fail)? as u8;
        take_byte(&mut bytes, b':').ok_or_else(fail)?;
        let second = take_digits(&mut bytes, 2).ok_or_else(fail)? as u8;

        let mut nanos = 0u32;
        if take_byte(&mut bytes, b'.').is_some() {
            let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
            if digits == 0 {
                return Err(fail());
            }
            // Keep nine digits of precision and ignore any beyond them.
            let mut scale = NANOS_PER_SECOND / 10;
            for &byte in &bytes[..digits] {
                nanos += (byte - b'0') as u32 * scale;
                scale /= 10;
                if scale == 0 {
                    break;
                }
            }
            bytes = &bytes[digits..];
        }

        let offset_secs = match bytes.first() {
            Some(b'Z') => {
                bytes = &bytes[1..];
                0
            }
            Some(&sign @ (b'+' | b'-')) => {
                bytes = &bytes[1..];
                let hours = take_digits(&mut bytes, 2).ok_or_else(fail)?;
                take_byte(&mut bytes, b':').ok_or_else(fail)?;
                let minutes = take_digits(&mut bytes, 2).ok_or_else(fail)?;
                if hours > 23 || minutes > 59 {
                    return Err(fail());
                }
                let magnitude = (hours * 3_600 + minutes * 60) as i64;
                if sign == b'-' { -magnitude } else { magnitude }
            }
            _ => return Err(fail()),
        };
        if !bytes.is_empty() {
            return Err(fail());
        }

        let local = DateTime::new(date, hour, minute, second, nanos).ok_or_else(fail)?;
        DateTime::from_unix_timestamp(local.unix_timestamp() - offset_secs, nanos).ok_or_else(fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64, nanos: u32) -> DateTime {
        DateTime::from_unix_timestamp(secs, nanos).unwrap()
    }

    #[test]
    fn unix_timestamp_round_trip() {
        assert_eq!(DateTime::UNIX_EPOCH.to_string(), "1970-01-01T00:00:00Z");
        let y2038 = at(2_147_483_647, 0);
        assert_eq!(y2038.to_string(), "2038-01-19T03:14:07Z");
        assert_eq!(y2038.date().weekday().name(), "Tuesday");
        assert_eq!(y2038.unix_timestamp(), 2_147_483_647);
        for secs in (-5_000_000_000i64..5_000_000_000).step_by(86_399_977) {
            assert_eq!(at(secs, 7).unix_timestamp(), secs);
        }
    }

    #[test]
    fn before_the_epoch_floors() {
        let moment = at(-1, 500_000_000);
        assert_eq!(moment.to_string(), "1969-12-31T23:59:59.5Z");
        assert_eq!(moment.unix_timestamp(), -1);
        assert_eq!(moment.unix_timestamp_millis(), -500);
    }

    #[test]
    fn rejects_out_of_range_fields() {
        assert!(DateTime::from_unix_timestamp(0, 1_000_000_000).is_none());
        let date = Date::new(2024, 1, 1).unwrap();
        assert!(DateTime::new(date, 24, 0, 0, 0).is_none());
        assert!(DateTime::new(date, 0, 60, 0, 0).is_none());
        assert!(DateTime::new(date, 0, 0, 60, 0).is_none());
        assert!(DateTime::new(date, 23, 59, 59, 999_999_999).is_some());
    }

    #[test]
    fn duration_arithmetic_carries() {
        let start = at(100, 900_000_000);
        let later = start.checked_add(Duration::new(1, 200_000_000)).unwrap();
        assert_eq!((later.unix_timestamp(), later.nanosecond()), (102, 100_000_000));
        assert_eq!(later.checked_sub(Duration::new(1, 200_000_000)), Some(start));
        assert_eq!(later.checked_duration_since(start), Some(Duration::new(1, 200_000_000)));
        assert_eq!(start.checked_duration_since(later), None);
        assert!(start.checked_add(Duration::MAX).is_none());
        assert!(start.checked_sub(Duration::MAX).is_none());
    }

    #[test]
    fn displays_trimmed_fraction() {
        assert_eq!(at(0, 120_000_000).to_string(), "1970-01-01T00:00:00.12Z");
        assert_eq!(at(0, 1).to_string(), "1970-01-01T00:00:00.000000001Z");
    }

    #[test]
    fn parses_iso_8601() {
        assert_eq!("1970-01-01T00:00:00Z".parse(), Ok(DateTime::UNIX_EPOCH));
        assert_eq!("2024-03-10T12:00:00+02:00".parse::<DateTime>().unwrap().to_string(), "2024-03-10T10:00:00Z");
        assert_eq!("2024-03-10T00:30:00-01:30".parse::<DateTime>().unwrap().to_string(), "2024-03-10T02:00:00Z");
        assert_eq!("2024-01-01T00:00:00+05:00".parse::<DateTime>().unwrap().to_string(), "2023-12-31T19:00:00Z");
        assert_eq!("2024-03-10T12:00:00.123456789123Z".parse::<DateTime>().unwrap().nanosecond(), 123_456_789);
        assert_eq!("2024-03-10T12:00:00.5Z".parse::<DateTime>().unwrap().nanosecond(), 500_000_000);
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in [
            "",
            "2024-03-10",
            "2024-03-10 12:00:00Z",
            "2024-03-10T12:00:00",
            "2024-03-10T24:00:00Z",
            "2024-03-10T12:60:00Z",
            "2024-03-10T12:00:60Z",
            "2024-03-10T12:00:00.Z",
            "2024-03-10T12:00:00Zjunk",
            "2024-03-10T12:00:00+2:00",
            "2024-03-10T12:00:00+24:00",
            "2024-02-30T12:00:00Z",
        ] {
            assert!(bad.parse::<DateTime>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        let moment = at(1_700_000_000, 42_000_000);
        assert_eq!(moment.to_string().parse(), Ok(moment));
    }

    #[test]
    fn formats_specifiers() {
        let moment = "2024-03-09T15:04:05.678Z".parse::<DateTime>().unwrap();
        assert_eq!(moment.format("%Y-%m-%d %H:%M:%S.%f"), "2024-03-09 15:04:05.678");
        assert_eq!(moment.format("%A, %B %e, %Y"), "Saturday, March  9, 2024");
        assert_eq!(moment.format("%a %b %y %j"), "Sat Mar 24 069");
        assert_eq!(moment.format("%I:%M %p"), "03:04 PM");
        assert_eq!(DateTime::UNIX_EPOCH.format("%I %p"), "12 AM");
        assert_eq!(at(12 * 3_600, 0).format("%I %p"), "12 PM");
        assert_eq!(moment.format("100%% %q %"), "100% %q %");
    }

    #[test]
    fn now_is_after_2020() {
        assert!(DateTime::now().date().year() >= 2020);
    }
}
