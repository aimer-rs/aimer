use std::fmt;
use std::str::FromStr;

/// The error returned when text is not a valid date or date-time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError {
    expected: &'static str,
}

impl ParseError {
    #[inline]
    pub(crate) fn new(expected: &'static str) -> Self {
        Self { expected }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid date or time: expected {}", self.expected)
    }
}

impl std::error::Error for ParseError {}

/// A day of the week, Monday first (ISO 8601).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

const WEEKDAYS: [Weekday; 7] = [
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];

impl Weekday {
    /// The full English name, such as `"Thursday"`.
    #[inline]
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 7] = [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ];
        NAMES[self as usize]
    }

    /// The three-letter English abbreviation, such as `"Thu"`.
    #[inline]
    pub fn short_name(self) -> &'static str {
        &self.name()[..3]
    }

    /// The ISO 8601 number: Monday is `1`, Sunday is `7`.
    #[inline]
    pub fn number_from_monday(self) -> u8 {
        self as u8 + 1
    }

    /// The following day of the week, wrapping Sunday to Monday.
    #[inline]
    pub fn next(self) -> Weekday {
        WEEKDAYS[(self as usize + 1) % 7]
    }

    /// The preceding day of the week, wrapping Monday to Sunday.
    #[inline]
    pub fn previous(self) -> Weekday {
        WEEKDAYS[(self as usize + 6) % 7]
    }
}

impl fmt::Display for Weekday {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Returns whether `year` is a leap year in the Gregorian calendar.
#[inline]
pub fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Returns the number of days in `month` (1-12) of `year`, or `0` for an
/// invalid month.
#[inline]
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to the given proleptic Gregorian date.
///
/// Howard Hinnant's `days_from_civil`; exact for every `i32` year.
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let (month, day) = (month as i64, day as i64);
    let year = year as i64 - (month <= 2) as i64;
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`]; `None` when the year does not fit `i32`.
fn civil_from_days(days: i64) -> Option<(i32, u8, u8)> {
    let z = days.checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u8;
    let year = year_of_era + era * 400 + (month <= 2) as i64;
    Some((i32::try_from(year).ok()?, month, day))
}

/// A calendar date in the proleptic Gregorian calendar, with no time zone.
///
/// Dates order chronologically. Their text form is `YYYY-MM-DD`, and
/// years `0..=9999` round-trip through [`FromStr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// The Unix epoch, 1970-01-01.
    pub const UNIX_EPOCH: Date = Date { year: 1970, month: 1, day: 1 };

    /// Creates a date, or `None` if `month` is not `1..=12` or `day` does not
    /// exist in that month.
    #[inline]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Date> {
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Date { year, month, day })
    }

    /// The year.
    #[inline]
    pub fn year(self) -> i32 {
        self.year
    }

    /// The month, `1..=12`.
    #[inline]
    pub fn month(self) -> u8 {
        self.month
    }

    /// The day of the month, starting at `1`.
    #[inline]
    pub fn day(self) -> u8 {
        self.day
    }

    /// The day of the year, `1..=366`.
    #[inline]
    pub fn ordinal(self) -> u16 {
        (days_from_civil(self.year, self.month, self.day) - days_from_civil(self.year, 1, 1)) as u16 + 1
    }

    /// The signed number of days since 1970-01-01.
    #[inline]
    pub fn days_since_epoch(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// The date `days` after 1970-01-01, or `None` if the year does not fit `i32`.
    #[inline]
    pub fn from_days_since_epoch(days: i64) -> Option<Date> {
        let (year, month, day) = civil_from_days(days)?;
        Some(Date { year, month, day })
    }

    /// The day of the week.
    #[inline]
    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday, index 3 counting from Monday.
        WEEKDAYS[(self.days_since_epoch() + 3).rem_euclid(7) as usize]
    }

    /// Returns whether this date falls in a leap year.
    #[inline]
    pub fn is_leap_year(self) -> bool {
        is_leap_year(self.year)
    }

    /// Adds `days` (negative to go back), or `None` on overflow.
    #[inline]
    pub fn checked_add_days(self, days: i64) -> Option<Date> {
        Date::from_days_since_epoch(self.days_since_epoch().checked_add(days)?)
    }

    /// The signed number of days from `self` to `other`.
    #[inline]
    pub fn days_until(self, other: Date) -> i64 {
        other.days_since_epoch() - self.days_since_epoch()
    }

    /// The day after this one, or `None` on overflow.
    #[inline]
    pub fn succ(self) -> Option<Date> {
        self.checked_add_days(1)
    }

    /// The day before this one, or `None` on overflow.
    #[inline]
    pub fn pred(self) -> Option<Date> {
        self.checked_add_days(-1)
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.year < 0 {
            write!(f, "-{:04}-{:02}-{:02}", self.year.unsigned_abs(), self.month, self.day)
        } else {
            write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
        }
    }
}

/// Parses exactly `digits` ASCII digits from the front of `bytes`.
pub(crate) fn take_digits(bytes: &mut &[u8], digits: usize) -> Option<u32> {
    if bytes.len() < digits {
        return None;
    }
    let (head, tail) = bytes.split_at(digits);
    let mut value = 0u32;
    for &byte in head {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (byte - b'0') as u32;
    }
    *bytes = tail;
    Some(value)
}

/// Consumes `byte` from the front of `bytes`.
pub(crate) fn take_byte(bytes: &mut &[u8], byte: u8) -> Option<()> {
    match bytes.split_first() {
        Some((&first, tail)) if first == byte => {
            *bytes = tail;
            Some(())
        }
        _ => None,
    }
}

/// Parses `YYYY-MM-DD` from the front of `bytes`.
pub(crate) fn take_date(bytes: &mut &[u8]) -> Option<Date> {
    let year = take_digits(bytes, 4)? as i32;
    take_byte(bytes, b'-')?;
    let month = take_digits(bytes, 2)? as u8;
    take_byte(bytes, b'-')?;
    let day = take_digits(bytes, 2)? as u8;
    Date::new(year, month, day)
}

impl FromStr for Date {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Date, ParseError> {
        let mut bytes = text.as_bytes();
        match take_date(&mut bytes) {
            Some(date) if bytes.is_empty() => Ok(date),
            _ => Err(ParseError::new("YYYY-MM-DD")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years_and_month_lengths() {
        assert!(is_leap_year(2000) && is_leap_year(2024));
        assert!(!is_leap_year(1900) && !is_leap_year(2023));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(days_in_month(2023, 4), 30);
        assert_eq!(days_in_month(2023, 13), 0);
    }

    #[test]
    fn rejects_invalid_dates() {
        assert!(Date::new(2023, 2, 29).is_none());
        assert!(Date::new(2023, 0, 1).is_none());
        assert!(Date::new(2023, 13, 1).is_none());
        assert!(Date::new(2023, 1, 0).is_none());
        assert!(Date::new(2023, 4, 31).is_none());
        assert!(Date::new(2024, 2, 29).is_some());
    }

    #[test]
    fn known_epoch_days() {
        assert_eq!(Date::UNIX_EPOCH.days_since_epoch(), 0);
        assert_eq!(Date::new(2000, 1, 1).unwrap().days_since_epoch(), 10_957);
        assert_eq!(Date::new(2000, 2, 29).unwrap().days_since_epoch(), 11_016);
        assert_eq!(Date::new(1969, 12, 31).unwrap().days_since_epoch(), -1);
    }

    #[test]
    fn days_round_trip() {
        for days in (-800_000..800_000).step_by(37) {
            let date = Date::from_days_since_epoch(days).unwrap();
            assert_eq!(date.days_since_epoch(), days);
            assert_eq!(Date::new(date.year(), date.month(), date.day()), Some(date));
        }
        assert!(Date::from_days_since_epoch(i64::MAX).is_none());
    }

    #[test]
    fn weekdays() {
        assert_eq!(Date::UNIX_EPOCH.weekday(), Weekday::Thursday);
        assert_eq!(Date::new(2000, 1, 1).unwrap().weekday(), Weekday::Saturday);
        assert_eq!(Date::new(2024, 2, 29).unwrap().weekday(), Weekday::Thursday);
        assert_eq!(Date::new(1969, 12, 31).unwrap().weekday(), Weekday::Wednesday);
        assert_eq!(Weekday::Sunday.next(), Weekday::Monday);
        assert_eq!(Weekday::Monday.previous(), Weekday::Sunday);
        assert_eq!(Weekday::Thursday.short_name(), "Thu");
        assert_eq!(Weekday::Sunday.number_from_monday(), 7);
    }

    #[test]
    fn ordinal_and_arithmetic() {
        assert_eq!(Date::new(2024, 1, 1).unwrap().ordinal(), 1);
        assert_eq!(Date::new(2024, 12, 31).unwrap().ordinal(), 366);
        assert_eq!(Date::new(2023, 12, 31).unwrap().ordinal(), 365);
        let leap_day = Date::new(2024, 2, 29).unwrap();
        assert_eq!(leap_day.succ(), Date::new(2024, 3, 1));
        assert_eq!(leap_day.pred(), Date::new(2024, 2, 28));
        assert_eq!(leap_day.checked_add_days(366), Date::new(2025, 3, 1));
        assert_eq!(Date::new(2024, 1, 1).unwrap().days_until(Date::new(2025, 1, 1).unwrap()), 366);
    }

    #[test]
    fn display_and_parse() {
        let date = Date::new(2024, 3, 9).unwrap();
        assert_eq!(date.to_string(), "2024-03-09");
        assert_eq!("2024-03-09".parse::<Date>(), Ok(date));
        assert_eq!(Date::new(7, 1, 2).unwrap().to_string(), "0007-01-02");
        for bad in ["", "2024-3-09", "2024-02-30", "2024-03-09x", "2024/03/09", "-2024-03-09"] {
            assert!(bad.parse::<Date>().is_err(), "{bad:?} should not parse");
        }
    }
}
