//! Validated local date-times with explicit timezone and bound policies.

use core::fmt;

use super::{
    CalendarNavigation, CancelReason, Date, FocusRestorer, FocusTarget, Month, OverlayConsumer,
    OverlayRequest, PickerOutcome, PickerOverlay, PickerSession, PickerSessionError,
};

/// Errors returned while constructing a [`TimeOfDay`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeError {
    /// The hour is outside `0..=23`.
    InvalidHour(u8),
    /// The minute is outside `0..=59`.
    InvalidMinute(u8),
    /// The second is outside `0..=59`.
    InvalidSecond(u8),
    /// Nanoseconds are outside one second.
    InvalidNanosecond(u32),
}

impl fmt::Display for TimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHour(value) => write!(formatter, "hour {value} is outside 0..=23"),
            Self::InvalidMinute(value) => write!(formatter, "minute {value} is outside 0..=59"),
            Self::InvalidSecond(value) => write!(formatter, "second {value} is outside 0..=59"),
            Self::InvalidNanosecond(value) => {
                write!(formatter, "nanosecond {value} is outside 0..=999999999")
            }
        }
    }
}

impl std::error::Error for TimeError {}

/// A validated wall-clock time without an implicit timezone.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimeOfDay {
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

impl TimeOfDay {
    /// Creates a validated wall-clock time.
    pub const fn try_new(
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
    ) -> Result<Self, TimeError> {
        if hour > 23 {
            return Err(TimeError::InvalidHour(hour));
        }
        if minute > 59 {
            return Err(TimeError::InvalidMinute(minute));
        }
        if second > 59 {
            return Err(TimeError::InvalidSecond(second));
        }
        if nanosecond >= 1_000_000_000 {
            return Err(TimeError::InvalidNanosecond(nanosecond));
        }
        Ok(Self {
            hour,
            minute,
            second,
            nanosecond,
        })
    }

    /// Returns midnight.
    #[inline]
    pub const fn midnight() -> Self {
        Self {
            hour: 0,
            minute: 0,
            second: 0,
            nanosecond: 0,
        }
    }

    /// Returns the hour.
    #[inline]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// Returns the minute.
    #[inline]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// Returns the second.
    #[inline]
    pub const fn second(self) -> u8 {
        self.second
    }

    /// Returns the nanosecond fraction.
    #[inline]
    pub const fn nanosecond(self) -> u32 {
        self.nanosecond
    }
}

/// An explicit timezone interpretation for a local date-time.
///
/// There is deliberately no implicit host-local variant. Callers choose UTC
/// or provide a fixed offset, while a platform adapter may translate its own
/// timezone into this policy before constructing a picker.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TimeZonePolicy {
    /// Coordinated Universal Time.
    Utc,
    /// A fixed offset from UTC in signed minutes.
    FixedOffset {
        /// Offset from UTC, constrained to `-1439..=1439` minutes.
        minutes: i16,
    },
}

impl TimeZonePolicy {
    /// Creates a fixed offset after validating its range.
    pub const fn fixed_offset(minutes: i32) -> Result<Self, DateTimeError> {
        if minutes < -1_439 || minutes > 1_439 {
            return Err(DateTimeError::InvalidOffset(minutes));
        }
        Ok(Self::FixedOffset {
            minutes: minutes as i16,
        })
    }

    /// Returns this policy's offset from UTC in minutes.
    #[inline]
    pub const fn offset_minutes(self) -> i16 {
        match self {
            Self::Utc => 0,
            Self::FixedOffset { minutes } => minutes,
        }
    }

    const fn is_valid(self) -> bool {
        match self {
            Self::Utc => true,
            Self::FixedOffset { minutes } => minutes >= -1_439 && minutes <= 1_439,
        }
    }
}

/// A validated date-time carrying its explicit timezone policy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DateTime {
    date: Date,
    time: TimeOfDay,
    timezone: TimeZonePolicy,
}

impl DateTime {
    /// Creates a date-time with an explicit timezone policy.
    pub const fn try_new(
        date: Date,
        time: TimeOfDay,
        timezone: TimeZonePolicy,
    ) -> Result<Self, DateTimeError> {
        if !timezone.is_valid() {
            return Err(DateTimeError::InvalidOffset(
                timezone.offset_minutes() as i32
            ));
        }
        Ok(Self {
            date,
            time,
            timezone,
        })
    }

    /// Returns the calendar date.
    #[inline]
    pub const fn date(self) -> Date {
        self.date
    }

    /// Returns the wall-clock time.
    #[inline]
    pub const fn time(self) -> TimeOfDay {
        self.time
    }

    /// Returns the explicit timezone policy.
    #[inline]
    pub const fn timezone(self) -> TimeZonePolicy {
        self.timezone
    }
}

/// Errors returned by date-time bounds and policy construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateTimeError {
    /// A fixed timezone offset is outside the supported range.
    InvalidOffset(i32),
    /// The two values use different timezone policies.
    TimeZoneMismatch {
        /// Policy required by the picker.
        expected: TimeZonePolicy,
        /// Policy carried by the supplied value.
        actual: TimeZonePolicy,
    },
    /// The lower bound is after the upper bound.
    ReversedBounds {
        /// Optional lower bound.
        min: Option<DateTime>,
        /// Optional upper bound.
        max: Option<DateTime>,
    },
    /// A value falls outside the inclusive bounds.
    OutOfBounds(DateTime),
}

impl fmt::Display for DateTimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOffset(minutes) => write!(
                formatter,
                "timezone offset {minutes} is outside -1439..=1439"
            ),
            Self::TimeZoneMismatch { expected, actual } => {
                write!(
                    formatter,
                    "timezone mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::ReversedBounds { min, max } => {
                write!(
                    formatter,
                    "date-time bounds are reversed: min={min:?}, max={max:?}"
                )
            }
            Self::OutOfBounds(value) => {
                write!(formatter, "date-time {value:?} is outside picker bounds")
            }
        }
    }
}

impl std::error::Error for DateTimeError {}

/// Inclusive date-time bounds. When both bounds exist they must use the same
/// timezone policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTimeBounds {
    min: Option<DateTime>,
    max: Option<DateTime>,
}

impl DateTimeBounds {
    /// Creates validated inclusive bounds.
    pub fn try_new(min: Option<DateTime>, max: Option<DateTime>) -> Result<Self, DateTimeError> {
        if let (Some(min), Some(max)) = (min, max) {
            if min.timezone() != max.timezone() {
                return Err(DateTimeError::TimeZoneMismatch {
                    expected: min.timezone(),
                    actual: max.timezone(),
                });
            }
            if min > max {
                return Err(DateTimeError::ReversedBounds {
                    min: Some(min),
                    max: Some(max),
                });
            }
        }
        Ok(Self { min, max })
    }

    /// Creates unbounded date-time constraints.
    #[inline]
    pub const fn unbounded() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Returns the inclusive lower bound, if present.
    #[inline]
    pub const fn min(self) -> Option<DateTime> {
        self.min
    }

    /// Returns the inclusive upper bound, if present.
    #[inline]
    pub const fn max(self) -> Option<DateTime> {
        self.max
    }

    /// Returns whether a date-time is inside the inclusive bounds.
    pub fn contains(self, value: DateTime) -> bool {
        if let Some(min) = self.min {
            if value.timezone() != min.timezone() {
                return false;
            }
            if value < min {
                return false;
            }
        }
        if let Some(max) = self.max {
            if value.timezone() != max.timezone() {
                return false;
            }
            if value > max {
                return false;
            }
        }
        true
    }
}

/// The complete timezone and min/max policy used by a [`DateTimePicker`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTimePickerPolicy {
    timezone: TimeZonePolicy,
    bounds: DateTimeBounds,
}

impl DateTimePickerPolicy {
    /// Creates an explicit timezone policy and validates its optional bounds.
    pub fn try_new(
        timezone: TimeZonePolicy,
        min: Option<DateTime>,
        max: Option<DateTime>,
    ) -> Result<Self, DateTimeError> {
        if !timezone.is_valid() {
            return Err(DateTimeError::InvalidOffset(
                timezone.offset_minutes() as i32
            ));
        }
        for value in [min, max].into_iter().flatten() {
            if value.timezone() != timezone {
                return Err(DateTimeError::TimeZoneMismatch {
                    expected: timezone,
                    actual: value.timezone(),
                });
            }
        }
        Ok(Self {
            timezone,
            bounds: DateTimeBounds::try_new(min, max)?,
        })
    }

    /// Creates an unbounded policy in `timezone`.
    #[inline]
    pub const fn unbounded(timezone: TimeZonePolicy) -> Self {
        Self {
            timezone,
            bounds: DateTimeBounds::unbounded(),
        }
    }

    /// Returns the explicit timezone policy.
    #[inline]
    pub const fn timezone(self) -> TimeZonePolicy {
        self.timezone
    }

    /// Returns the inclusive date-time bounds.
    #[inline]
    pub const fn bounds(self) -> DateTimeBounds {
        self.bounds
    }
}

/// Errors returned by [`DateTimePicker`] editing operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateTimePickerError {
    /// The requested value violates the picker policy.
    Invalid(DateTimeError),
    /// The requested calendar date is explicitly disabled.
    DisabledDate(Date),
    /// The picker must be opened before it can be edited or closed.
    Closed,
    /// No overlay host is installed for the presentation request.
    MissingHost,
    /// An installed overlay host cannot represent the presentation request.
    UnsupportedHost,
    /// The overlay policy does not allow this user-driven dismissal reason.
    DismissalNotAllowed(CancelReason),
}

impl fmt::Display for DateTimePickerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(formatter),
            Self::DisabledDate(date) => write!(formatter, "date {date:?} is disabled"),
            Self::Closed => formatter.write_str("date-time picker is closed"),
            Self::MissingHost => formatter.write_str("picker overlay host is missing"),
            Self::UnsupportedHost => formatter.write_str("picker overlay host is unsupported"),
            Self::DismissalNotAllowed(reason) => {
                write!(
                    formatter,
                    "picker overlay dismissal is disabled for {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for DateTimePickerError {}

/// A transactional date-time picker with explicit timezone and bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DateTimePicker {
    policy: DateTimePickerPolicy,
    session: PickerSession<Option<DateTime>>,
    disabled_dates: Vec<Date>,
}

impl DateTimePicker {
    /// Creates a picker, deriving a valid initial value from the first bound
    /// or the UTC epoch when `initial` is `None`.
    pub fn try_new(
        initial: Option<DateTime>,
        policy: DateTimePickerPolicy,
    ) -> Result<Self, DateTimePickerError> {
        if !policy.timezone().is_valid() {
            return Err(DateTimePickerError::Invalid(DateTimeError::InvalidOffset(
                policy.timezone().offset_minutes() as i32,
            )));
        }
        let initial = match initial {
            Some(value) => {
                validate_value(value, policy)?;
                value
            }
            None => policy
                .bounds()
                .min()
                .or(policy.bounds().max())
                .unwrap_or_else(|| {
                    DateTime::try_new(
                        Date::try_new(1970, 1, 1).expect("the epoch is a valid date"),
                        TimeOfDay::midnight(),
                        policy.timezone(),
                    )
                    .expect("validated date-time components cannot fail")
                }),
        };
        Ok(Self {
            policy,
            session: PickerSession::new(Some(initial)),
            disabled_dates: Vec::new(),
        })
    }

    /// Returns a picker that marks the supplied calendar dates unavailable.
    ///
    /// Disabled dates remain visible in the inline calendar but cannot be
    /// selected. The dates are sorted and deduplicated.
    pub fn with_disabled_dates<I>(mut self, dates: I) -> Self
    where
        I: IntoIterator<Item = Date>,
    {
        self.set_disabled_dates(dates);
        self
    }

    /// Replaces the calendar dates unavailable for selection.
    pub fn set_disabled_dates<I>(&mut self, dates: I)
    where
        I: IntoIterator<Item = Date>,
    {
        self.disabled_dates = dates.into_iter().collect();
        self.disabled_dates.sort_unstable();
        self.disabled_dates.dedup();
    }

    /// Returns whether `date` is unavailable because of the picker bounds or
    /// an explicit disabled-date rule.
    #[inline]
    pub fn is_date_disabled(&self, date: Date) -> bool {
        self.policy
            .bounds()
            .min()
            .is_some_and(|minimum| date < minimum.date())
            || self
                .policy
                .bounds()
                .max()
                .is_some_and(|maximum| date > maximum.date())
            || self.disabled_dates.binary_search(&date).is_ok()
    }

    /// Returns the explicitly disabled calendar dates in sorted order.
    #[inline]
    pub fn disabled_dates(&self) -> &[Date] {
        &self.disabled_dates
    }

    /// Opens the picker and resets its draft to the last confirmed value.
    pub fn open(&mut self) {
        self.session.open();
    }

    /// Presents the picker through a checked caller-owned overlay host and
    /// opens its transactional draft.
    pub fn open_with_overlay<C: OverlayConsumer>(
        &mut self,
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<PickerOverlay<C::Handle>, DateTimePickerError> {
        self.session
            .open_with_overlay(consumer, request, restore_focus)
            .map_err(DateTimePickerError::from)
    }

    /// Returns whether the picker is open.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.session.is_open()
    }

    /// Returns the configured timezone and bounds policy.
    #[inline]
    pub const fn policy(&self) -> DateTimePickerPolicy {
        self.policy
    }

    /// Returns the last confirmed date-time.
    #[inline]
    pub const fn value(&self) -> Option<DateTime> {
        *self.session.committed()
    }

    /// Returns the current date-time draft.
    #[inline]
    pub const fn draft(&self) -> Option<DateTime> {
        *self.session.draft()
    }

    /// Builds a platform-neutral accessibility snapshot of this picker.
    #[inline]
    pub fn semantics(&self) -> super::DateTimePickerSemantics {
        super::DateTimePickerSemantics::from_picker(self)
    }

    /// Replaces the date portion of the open draft.
    pub fn set_date(&mut self, date: Date) -> Result<(), DateTimePickerError> {
        self.ensure_open()?;
        if self.disabled_dates.binary_search(&date).is_ok() {
            return Err(DateTimePickerError::DisabledDate(date));
        }
        let current = self.draft().ok_or(DateTimePickerError::Closed)?;
        let value = DateTime::try_new(date, current.time(), self.policy.timezone())
            .map_err(DateTimePickerError::Invalid)?;
        validate_value(value, self.policy)?;
        self.session
            .set_draft(Some(value))
            .map_err(|_| DateTimePickerError::Closed)
    }

    /// Replaces the time portion of the open draft.
    pub fn set_time(&mut self, time: TimeOfDay) -> Result<(), DateTimePickerError> {
        self.ensure_open()?;
        let current = self.draft().ok_or(DateTimePickerError::Closed)?;
        let value = DateTime::try_new(current.date(), time, self.policy.timezone())
            .map_err(DateTimePickerError::Invalid)?;
        validate_value(value, self.policy)?;
        self.session
            .set_draft(Some(value))
            .map_err(|_| DateTimePickerError::Closed)
    }

    /// Moves the date portion of the open draft and reports whether it stayed
    /// within the configured bounds.
    pub fn navigate(
        &mut self,
        navigation: CalendarNavigation,
    ) -> Result<bool, DateTimePickerError> {
        self.ensure_open()?;
        let current = self.draft().ok_or(DateTimePickerError::Closed)?;
        let next = match navigation {
            CalendarNavigation::PreviousDay => current.date().add_days(-1),
            CalendarNavigation::NextDay => current.date().add_days(1),
            CalendarNavigation::PreviousWeek => current.date().add_days(-7),
            CalendarNavigation::NextWeek => current.date().add_days(7),
            CalendarNavigation::PreviousMonth => current.date().add_months(-1),
            CalendarNavigation::NextMonth => current.date().add_months(1),
            CalendarNavigation::PreviousYear => current.date().add_months(-12),
            CalendarNavigation::NextYear => current.date().add_months(12),
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let direction = match navigation {
            CalendarNavigation::PreviousDay
            | CalendarNavigation::PreviousWeek
            | CalendarNavigation::PreviousMonth
            | CalendarNavigation::PreviousYear => -1,
            CalendarNavigation::NextDay
            | CalendarNavigation::NextWeek
            | CalendarNavigation::NextMonth
            | CalendarNavigation::NextYear => 1,
        };
        let target_month = match navigation {
            CalendarNavigation::PreviousMonth
            | CalendarNavigation::NextMonth
            | CalendarNavigation::PreviousYear
            | CalendarNavigation::NextYear => Some(next.month_key()),
            CalendarNavigation::PreviousDay
            | CalendarNavigation::NextDay
            | CalendarNavigation::PreviousWeek
            | CalendarNavigation::NextWeek => None,
        };
        let Some(next) =
            self.find_enabled_navigation_date(next, direction, target_month, current.time())?
        else {
            return Ok(false);
        };
        self.set_date(next)?;
        Ok(true)
    }

    fn find_enabled_navigation_date(
        &self,
        start: Date,
        direction: i32,
        target_month: Option<Month>,
        time: TimeOfDay,
    ) -> Result<Option<Date>, DateTimePickerError> {
        if target_month.is_some() {
            if let Some(date) = self.scan_navigation_dates(start, direction, target_month, time)? {
                return Ok(Some(date));
            }
            // The preferred direction can run into a partial bound before
            // reaching an enabled date. Search the other side of the target
            // month so month/year traversal still reaches its valid portion.
            return self.scan_navigation_dates(start, -direction, target_month, time);
        }
        self.scan_navigation_dates(start, direction, None, time)
    }

    fn scan_navigation_dates(
        &self,
        start: Date,
        direction: i32,
        target_month: Option<Month>,
        time: TimeOfDay,
    ) -> Result<Option<Date>, DateTimePickerError> {
        let mut candidate = Some(start);
        while let Some(date) = candidate {
            if target_month.is_some_and(|month| date.month_key() != month) {
                return Ok(None);
            }
            if self.disabled_dates.binary_search(&date).is_err() {
                let value = DateTime::try_new(date, time, self.policy.timezone())
                    .map_err(DateTimePickerError::Invalid)?;
                if self.policy.bounds().contains(value) {
                    return Ok(Some(date));
                }
            }
            candidate = date.add_days(direction);
        }
        Ok(None)
    }

    /// Confirms the draft and closes the picker.
    pub fn confirm(&mut self) -> Result<PickerOutcome<Option<DateTime>>, DateTimePickerError> {
        self.ensure_open()?;
        Ok(self
            .session
            .confirm()
            .map_err(|_| DateTimePickerError::Closed)?)
    }

    /// Confirms the draft, dismisses its overlay, and restores focus.
    pub fn confirm_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Option<DateTime>>, DateTimePickerError> {
        if !self.is_open() {
            let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
            return Err(DateTimePickerError::Closed);
        }
        self.session
            .confirm_with_overlay(overlay, consumer, focus)
            .map_err(DateTimePickerError::from)
    }

    /// Cancels the draft and closes the picker without changing its value.
    pub fn cancel(
        &mut self,
        reason: CancelReason,
    ) -> Result<PickerOutcome<Option<DateTime>>, DateTimePickerError> {
        self.ensure_open()?;
        Ok(self
            .session
            .cancel(reason)
            .map_err(|_| DateTimePickerError::Closed)?)
    }

    /// Cancels the draft according to the overlay policy, dismisses the
    /// overlay, and restores focus.
    pub fn cancel_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Option<DateTime>>, DateTimePickerError> {
        self.session
            .cancel_with_overlay(overlay, reason, consumer, focus)
            .map_err(DateTimePickerError::from)
    }

    /// Closes the picker as a programmatic cancellation through its overlay.
    pub fn close_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<Option<DateTime>>, DateTimePickerError> {
        self.cancel_with_overlay(overlay, CancelReason::Programmatic, consumer, focus)
    }

    fn ensure_open(&self) -> Result<(), DateTimePickerError> {
        if self.is_open() {
            Ok(())
        } else {
            Err(DateTimePickerError::Closed)
        }
    }
}

impl From<PickerSessionError> for DateTimePickerError {
    fn from(error: PickerSessionError) -> Self {
        match error {
            PickerSessionError::Closed => Self::Closed,
            PickerSessionError::MissingHost => Self::MissingHost,
            PickerSessionError::UnsupportedHost => Self::UnsupportedHost,
            PickerSessionError::DismissalNotAllowed(reason) => Self::DismissalNotAllowed(reason),
        }
    }
}

/// Errors returned by [`TimePicker`] editing operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimePickerError {
    /// The picker must be opened before it can be edited or closed.
    Closed,
    /// No overlay host is installed for the presentation request.
    MissingHost,
    /// An installed overlay host cannot represent the presentation request.
    UnsupportedHost,
    /// The overlay policy does not allow this user-driven dismissal reason.
    DismissalNotAllowed(CancelReason),
}

impl fmt::Display for TimePickerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("time picker is closed"),
            Self::MissingHost => formatter.write_str("picker overlay host is missing"),
            Self::UnsupportedHost => formatter.write_str("picker overlay host is unsupported"),
            Self::DismissalNotAllowed(reason) => {
                write!(
                    formatter,
                    "picker overlay dismissal is disabled for {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for TimePickerError {}

/// A transactional standalone wall-clock time picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimePicker {
    session: PickerSession<TimeOfDay>,
}

impl TimePicker {
    /// Creates a picker with `initial` as its confirmed time, or midnight when
    /// no initial time is supplied.
    pub fn new(initial: Option<TimeOfDay>) -> Self {
        Self {
            session: PickerSession::new(initial.unwrap_or_else(TimeOfDay::midnight)),
        }
    }

    /// Opens the picker and resets its draft to the last confirmed time.
    #[inline]
    pub fn open(&mut self) {
        self.session.open();
    }

    /// Presents the picker through a checked caller-owned overlay host and
    /// opens its transactional draft.
    pub fn open_with_overlay<C: OverlayConsumer>(
        &mut self,
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<PickerOverlay<C::Handle>, TimePickerError> {
        self.session
            .open_with_overlay(consumer, request, restore_focus)
            .map_err(TimePickerError::from)
    }

    /// Returns whether the picker is open.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.session.is_open()
    }

    /// Returns the last confirmed time.
    #[inline]
    pub const fn value(&self) -> TimeOfDay {
        *self.session.committed()
    }

    /// Returns the current open draft, or the confirmed time while closed.
    #[inline]
    pub const fn draft(&self) -> TimeOfDay {
        *self.session.draft()
    }

    /// Builds a platform-neutral accessibility snapshot of this picker.
    #[inline]
    pub fn semantics(&self) -> super::TimePickerSemantics {
        super::TimePickerSemantics::from_picker(self)
    }

    /// Replaces the time portion of the open draft.
    pub fn set_time(&mut self, time: TimeOfDay) -> Result<(), TimePickerError> {
        self.session
            .set_draft(time)
            .map_err(|_| TimePickerError::Closed)
    }

    /// Confirms the draft and closes the picker.
    pub fn confirm(&mut self) -> Result<PickerOutcome<TimeOfDay>, TimePickerError> {
        self.session.confirm().map_err(|_| TimePickerError::Closed)
    }

    /// Confirms the draft, dismisses its overlay, and restores focus.
    pub fn confirm_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<TimeOfDay>, TimePickerError> {
        if !self.is_open() {
            let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
            return Err(TimePickerError::Closed);
        }
        self.session
            .confirm_with_overlay(overlay, consumer, focus)
            .map_err(TimePickerError::from)
    }

    /// Cancels the draft and closes the picker without changing its value.
    pub fn cancel(
        &mut self,
        reason: CancelReason,
    ) -> Result<PickerOutcome<TimeOfDay>, TimePickerError> {
        self.session
            .cancel(reason)
            .map_err(|_| TimePickerError::Closed)
    }

    /// Cancels the draft according to the overlay policy, dismisses the
    /// overlay, and restores focus.
    pub fn cancel_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<TimeOfDay>, TimePickerError> {
        self.session
            .cancel_with_overlay(overlay, reason, consumer, focus)
            .map_err(TimePickerError::from)
    }

    /// Closes the picker as a programmatic cancellation through its overlay.
    pub fn close_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<TimeOfDay>, TimePickerError> {
        self.cancel_with_overlay(overlay, CancelReason::Programmatic, consumer, focus)
    }
}

impl From<PickerSessionError> for TimePickerError {
    fn from(error: PickerSessionError) -> Self {
        match error {
            PickerSessionError::Closed => Self::Closed,
            PickerSessionError::MissingHost => Self::MissingHost,
            PickerSessionError::UnsupportedHost => Self::UnsupportedHost,
            PickerSessionError::DismissalNotAllowed(reason) => Self::DismissalNotAllowed(reason),
        }
    }
}

fn validate_value(
    value: DateTime,
    policy: DateTimePickerPolicy,
) -> Result<(), DateTimePickerError> {
    if value.timezone() != policy.timezone() {
        return Err(DateTimePickerError::Invalid(
            DateTimeError::TimeZoneMismatch {
                expected: policy.timezone(),
                actual: value.timezone(),
            },
        ));
    }
    if !policy.bounds().contains(value) {
        return Err(DateTimePickerError::Invalid(DateTimeError::OutOfBounds(
            value,
        )));
    }
    Ok(())
}
