//! Gregorian date primitives used by the picker models.

use core::fmt;

const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9999;

/// A validated proleptic-Gregorian calendar date.
///
/// The picker model intentionally stores a date without a locale or timezone.
/// Formatting and timezone conversion belong to adapters above this crate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// Creates a date after validating its year, month, and day.
    ///
    /// Years are limited to `1..=9999`, and the Gregorian leap-year rules are
    /// applied when validating February.
    pub const fn try_new(year: i32, month: u8, day: u8) -> Result<Self, DateError> {
        if year < MIN_YEAR || year > MAX_YEAR {
            return Err(DateError::InvalidYear { year });
        }
        let Some(days) = Self::days_in_month(year, month) else {
            return Err(DateError::InvalidMonth { month });
        };
        if day == 0 || day > days {
            return Err(DateError::InvalidDay { year, month, day });
        }
        Ok(Self { year, month, day })
    }

    /// Returns the year component.
    #[inline]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// Returns the one-based month component.
    #[inline]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// Returns the one-based day component.
    #[inline]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// Returns whether `year` has a February 29.
    #[inline]
    pub const fn is_leap_year(year: i32) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    }

    /// Returns the number of days in a month, or `None` for an invalid month.
    pub const fn days_in_month(year: i32, month: u8) -> Option<u8> {
        if year < MIN_YEAR || year > MAX_YEAR {
            return None;
        }
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if Self::is_leap_year(year) => 29,
            2 => 28,
            _ => return None,
        };
        Some(days)
    }

    /// Returns the month containing this date.
    #[inline]
    pub const fn month_key(self) -> Month {
        Month {
            year: self.year,
            month: self.month,
        }
    }

    /// Returns the first day of this date's month.
    #[inline]
    pub const fn first_of_month(self) -> Self {
        Self {
            year: self.year,
            month: self.month,
            day: 1,
        }
    }

    /// Moves by a number of days, returning `None` if the result leaves the
    /// supported year range.
    pub fn add_days(self, offset: i32) -> Option<Self> {
        let days = days_from_civil(self.year, self.month, self.day);
        let (year, month, day) = civil_from_days(days.checked_add(i64::from(offset))?);
        Self::try_new(year, month, day).ok()
    }

    /// Moves by calendar months while keeping the day when possible.
    ///
    /// If the target month is shorter, the result is clamped to its last day;
    /// for example, January 31 moved by one month becomes February 28 or 29.
    pub fn add_months(self, offset: i32) -> Option<Self> {
        let month = self.month_key().add_months(offset)?;
        let day = self.day.min(Self::days_in_month(month.year, month.month)?);
        Self::try_new(month.year, month.month, day).ok()
    }

    /// Returns the ISO-style weekday, with Monday as the first value.
    pub fn weekday(self) -> Weekday {
        let index = (days_from_civil(self.year, self.month, self.day) + 3).rem_euclid(7);
        Weekday::from_index(index as u8)
    }
}

/// A validated year/month pair used as a calendar viewport key.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Month {
    year: i32,
    month: u8,
}

impl Month {
    /// Creates a year/month pair after validating both components.
    pub const fn try_new(year: i32, month: u8) -> Result<Self, DateError> {
        if year < MIN_YEAR || year > MAX_YEAR {
            return Err(DateError::InvalidYear { year });
        }
        if month < 1 || month > 12 {
            return Err(DateError::InvalidMonth { month });
        }
        Ok(Self { year, month })
    }

    /// Returns the year component.
    #[inline]
    pub const fn year(self) -> i32 {
        self.year
    }

    /// Returns the one-based month component.
    #[inline]
    pub const fn month(self) -> u8 {
        self.month
    }

    /// Returns the first date in this month.
    #[inline]
    pub const fn first_day(self) -> Date {
        Date {
            year: self.year,
            month: self.month,
            day: 1,
        }
    }

    /// Moves by calendar months, returning `None` outside the supported year
    /// range.
    pub fn add_months(self, offset: i32) -> Option<Self> {
        let absolute = i64::from(self.year) * 12 + i64::from(self.month - 1);
        let target = absolute.checked_add(i64::from(offset))?;
        let year = target.div_euclid(12);
        let month = target.rem_euclid(12) + 1;
        let year = i32::try_from(year).ok()?;
        Self::try_new(year, month as u8).ok()
    }
}

/// Weekdays in Monday-first order.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Weekday {
    /// Monday.
    Monday,
    /// Tuesday.
    Tuesday,
    /// Wednesday.
    Wednesday,
    /// Thursday.
    Thursday,
    /// Friday.
    Friday,
    /// Saturday.
    Saturday,
    /// Sunday.
    Sunday,
}

impl Weekday {
    /// Returns the zero-based Monday-first index.
    #[inline]
    pub const fn index(self) -> u8 {
        match self {
            Self::Monday => 0,
            Self::Tuesday => 1,
            Self::Wednesday => 2,
            Self::Thursday => 3,
            Self::Friday => 4,
            Self::Saturday => 5,
            Self::Sunday => 6,
        }
    }

    const fn from_index(index: u8) -> Self {
        match index {
            0 => Self::Monday,
            1 => Self::Tuesday,
            2 => Self::Wednesday,
            3 => Self::Thursday,
            4 => Self::Friday,
            5 => Self::Saturday,
            _ => Self::Sunday,
        }
    }
}

/// Errors returned while constructing or constraining a [`Date`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateError {
    /// The year is outside the supported `1..=9999` range.
    InvalidYear { year: i32 },
    /// The month is outside `1..=12`.
    InvalidMonth { month: u8 },
    /// The day does not exist in the requested month.
    InvalidDay { year: i32, month: u8, day: u8 },
    /// The lower bound is after the upper bound.
    InvalidBounds {
        /// Optional lower bound.
        min: Option<Date>,
        /// Optional upper bound.
        max: Option<Date>,
    },
    /// The date is outside inclusive bounds.
    OutOfBounds {
        /// Date checked against the bounds.
        date: Date,
        /// Optional lower bound.
        min: Option<Date>,
        /// Optional upper bound.
        max: Option<Date>,
    },
}

impl fmt::Display for DateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidYear { year } => write!(formatter, "year {year} is outside 1..=9999"),
            Self::InvalidMonth { month } => write!(formatter, "month {month} is outside 1..=12"),
            Self::InvalidDay { year, month, day } => {
                write!(
                    formatter,
                    "day {day} does not exist in {year:04}-{month:02}"
                )
            }
            Self::InvalidBounds { min, max } => {
                write!(
                    formatter,
                    "date bounds are reversed: min={min:?}, max={max:?}"
                )
            }
            Self::OutOfBounds { date, min, max } => {
                write!(
                    formatter,
                    "date {date:?} is outside bounds: min={min:?}, max={max:?}"
                )
            }
        }
    }
}

impl std::error::Error for DateError {}

/// Inclusive minimum and maximum date constraints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateBounds {
    min: Option<Date>,
    max: Option<Date>,
}

impl DateBounds {
    /// Creates inclusive bounds, rejecting a reversed pair.
    pub fn new(min: Option<Date>, max: Option<Date>) -> Result<Self, DateError> {
        if let (Some(min), Some(max)) = (min, max) {
            if min > max {
                return Err(DateError::InvalidBounds {
                    min: Some(min),
                    max: Some(max),
                });
            }
        }
        Ok(Self { min, max })
    }

    /// Creates an unconstrained date range.
    #[inline]
    pub const fn unbounded() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Returns the inclusive lower bound, if present.
    #[inline]
    pub const fn min(self) -> Option<Date> {
        self.min
    }

    /// Returns the inclusive upper bound, if present.
    #[inline]
    pub const fn max(self) -> Option<Date> {
        self.max
    }

    /// Returns whether `date` is within these inclusive bounds.
    #[inline]
    pub fn contains(self, date: Date) -> bool {
        if let Some(min) = self.min {
            if date < min {
                return false;
            }
        }
        if let Some(max) = self.max {
            if date > max {
                return false;
            }
        }
        true
    }

    /// Validates a date against the bounds.
    pub fn validate(self, date: Date) -> Result<(), DateError> {
        if self.contains(date) {
            Ok(())
        } else {
            Err(DateError::OutOfBounds {
                date,
                min: self.min,
                max: self.max,
            })
        }
    }

    /// Clamps a date to the nearest bound, when a bound exists.
    pub fn clamp(self, date: Date) -> Date {
        if let Some(min) = self.min {
            if date < min {
                return min;
            }
        }
        if let Some(max) = self.max {
            if date > max {
                return max;
            }
        }
        date
    }
}

fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 }.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 }.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * month_part + 2).div_euclid(5) + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year as i32, month as u8, day as u8)
}
