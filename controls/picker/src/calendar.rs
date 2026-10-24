//! Calendar viewport, cell identity, navigation, and date-range selection.

use core::fmt;

use super::semantics::CalendarSemantics;
use super::{Date, DateBounds, Month};

/// Whether a range that is selected in reverse order should swap its ends or
/// begin a new range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeOrderPolicy {
    /// Selecting an earlier second date produces an ascending range.
    Swap,
    /// Selecting an earlier second date starts a new range at that date.
    Restart,
}

/// Explicit policy for an inclusive date range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateRangePolicy {
    allow_same_day: bool,
    reverse_order: RangeOrderPolicy,
}

impl DateRangePolicy {
    /// Creates a range policy with explicit same-day and reverse-order rules.
    pub const fn new(allow_same_day: bool, reverse_order: RangeOrderPolicy) -> Self {
        Self {
            allow_same_day,
            reverse_order,
        }
    }

    /// Creates the usual inclusive range policy: same-day ranges are valid and
    /// reverse selection swaps the endpoints.
    #[inline]
    pub const fn inclusive() -> Self {
        Self::new(true, RangeOrderPolicy::Swap)
    }

    /// Returns whether selecting the same date twice is valid.
    #[inline]
    pub const fn allow_same_day(self) -> bool {
        self.allow_same_day
    }

    /// Returns the reverse-order behavior.
    #[inline]
    pub const fn reverse_order(self) -> RangeOrderPolicy {
        self.reverse_order
    }
}

/// Selection mode used by a [`Calendar`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateSelectionMode {
    /// Select exactly one date at a time.
    Single,
    /// Select an inclusive range under the supplied policy.
    Range(DateRangePolicy),
}

/// The selection currently held by a calendar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateSelection {
    /// No date or one selected date.
    Single(Option<Date>),
    /// A possibly incomplete inclusive range.
    Range {
        /// The first endpoint, if selected.
        start: Option<Date>,
        /// The second endpoint, if selected.
        end: Option<Date>,
    },
}

impl DateSelection {
    /// Returns an empty selection for `mode`.
    pub const fn empty(mode: DateSelectionMode) -> Self {
        match mode {
            DateSelectionMode::Single => Self::Single(None),
            DateSelectionMode::Range(_) => Self::Range {
                start: None,
                end: None,
            },
        }
    }

    /// Returns whether `date` is selected, including both endpoints of a range.
    pub fn contains(self, date: Date) -> bool {
        match self {
            Self::Single(Some(selected)) => selected == date,
            Self::Single(None) => false,
            Self::Range {
                start: Some(start),
                end: Some(end),
            } => date >= start && date <= end,
            Self::Range {
                start: Some(start),
                end: None,
            } => date == start,
            Self::Range { start: None, .. } => false,
        }
    }

    /// Returns whether `date` is a visible endpoint of the current selection.
    ///
    /// A single selection is treated as one endpoint. An incomplete range has
    /// only its start endpoint, while a complete range has both endpoints.
    #[inline]
    pub fn is_range_endpoint(self, date: Date) -> bool {
        match self {
            Self::Single(Some(selected)) => selected == date,
            Self::Single(None) => false,
            Self::Range { start, end } => {
                matches!(start, Some(start) if start == date)
                    || matches!(end, Some(end) if end == date)
            }
        }
    }

    /// Returns whether `date` lies strictly inside a complete range.
    #[inline]
    pub fn is_range_interior(self, date: Date) -> bool {
        match self {
            Self::Range {
                start: Some(start),
                end: Some(end),
            } => date > start && date < end,
            Self::Single(_) | Self::Range { .. } => false,
        }
    }

    /// Returns whether the selection has all endpoints required by its mode.
    #[inline]
    pub const fn is_complete(self) -> bool {
        match self {
            Self::Single(Some(_)) => true,
            Self::Single(None) => false,
            Self::Range {
                start: Some(_),
                end: Some(_),
            } => true,
            Self::Range { .. } => false,
        }
    }
}

/// Errors returned by calendar focus and selection operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarError {
    /// The requested date is outside the calendar's inclusive bounds.
    OutOfBounds(Date),
    /// The requested date was explicitly disabled by the calendar.
    DisabledDate(Date),
    /// A same-day range was disallowed by [`DateRangePolicy`].
    SameDayRangeNotAllowed(Date),
}

impl fmt::Display for CalendarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds(date) => write!(formatter, "date {date:?} is outside picker bounds"),
            Self::DisabledDate(date) => write!(formatter, "date {date:?} is disabled"),
            Self::SameDayRangeNotAllowed(date) => {
                write!(formatter, "same-day range at {date:?} is not allowed")
            }
        }
    }
}

impl std::error::Error for CalendarError {}

/// Stable identity for a calendar cell.
///
/// The identity is the represented date rather than a row/column position, so
/// a retained cell can be reconciled correctly when the visible month changes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CalendarCellId(Date);

impl CalendarCellId {
    /// Creates a stable cell identity for `date`.
    #[inline]
    pub const fn from_date(date: Date) -> Self {
        Self(date)
    }

    /// Returns the date represented by the cell identity.
    #[inline]
    pub const fn date(self) -> Date {
        self.0
    }
}

/// One date cell in a calendar's six-week viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalendarCell {
    id: CalendarCellId,
    date: Date,
    in_visible_month: bool,
    disabled: bool,
    selected: bool,
}

impl CalendarCell {
    /// Returns the stable date-based identity.
    #[inline]
    pub const fn id(self) -> CalendarCellId {
        self.id
    }

    /// Returns the represented date.
    #[inline]
    pub const fn date(self) -> Date {
        self.date
    }

    /// Returns whether the date belongs to the visible month.
    #[inline]
    pub const fn in_visible_month(self) -> bool {
        self.in_visible_month
    }

    /// Returns whether the date cannot be selected under the calendar bounds.
    #[inline]
    pub const fn is_disabled(self) -> bool {
        self.disabled
    }

    /// Returns whether the date is part of the current selection.
    #[inline]
    pub const fn is_selected(self) -> bool {
        self.selected
    }
}

/// Keyboard-like navigation operations supported by a calendar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarNavigation {
    /// Move focus to the previous date.
    PreviousDay,
    /// Move focus to the next date.
    NextDay,
    /// Move focus one week backward.
    PreviousWeek,
    /// Move focus one week forward.
    NextWeek,
    /// Move focus one month backward.
    PreviousMonth,
    /// Move focus one month forward.
    NextMonth,
    /// Move focus one year backward.
    PreviousYear,
    /// Move focus one year forward.
    NextYear,
}

/// A platform-neutral calendar viewport and selection model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Calendar {
    visible_month: Month,
    focused: Date,
    bounds: DateBounds,
    mode: DateSelectionMode,
    selection: DateSelection,
    disabled_dates: Vec<Date>,
}

impl Calendar {
    /// Creates an unbounded single-date calendar focused on `focused`.
    pub fn new(focused: Date) -> Self {
        Self::try_new(focused, DateBounds::unbounded(), DateSelectionMode::Single)
            .expect("an unbounded calendar accepts every valid date")
    }

    /// Creates a calendar with explicit bounds and single/range policy.
    pub fn try_new(
        focused: Date,
        bounds: DateBounds,
        mode: DateSelectionMode,
    ) -> Result<Self, CalendarError> {
        if !bounds.contains(focused) {
            return Err(CalendarError::OutOfBounds(focused));
        }
        Ok(Self {
            visible_month: focused.month_key(),
            focused,
            bounds,
            mode,
            selection: DateSelection::empty(mode),
            disabled_dates: Vec::new(),
        })
    }

    /// Returns a calendar that also disables the supplied dates.
    ///
    /// Disabled dates remain visible in the calendar, but cannot be selected.
    /// Dates are sorted and deduplicated so repeated entries do not change the
    /// observable calendar state.
    pub fn with_disabled_dates<I>(mut self, dates: I) -> Self
    where
        I: IntoIterator<Item = Date>,
    {
        self.set_disabled_dates(dates);
        self
    }

    /// Replaces the explicitly disabled dates.
    pub fn set_disabled_dates<I>(&mut self, dates: I)
    where
        I: IntoIterator<Item = Date>,
    {
        self.disabled_dates = dates.into_iter().collect();
        self.disabled_dates.sort_unstable();
        self.disabled_dates.dedup();
        self.repair_focus_after_disabled_dates_change();
    }

    /// Returns the explicitly disabled dates in sorted order.
    #[inline]
    pub fn disabled_dates(&self) -> &[Date] {
        &self.disabled_dates
    }

    /// Returns whether `date` is unavailable because of bounds or an explicit
    /// disabled-date rule.
    #[inline]
    pub fn is_date_disabled(&self, date: Date) -> bool {
        !self.bounds.contains(date) || self.disabled_dates.binary_search(&date).is_ok()
    }

    /// Returns the currently visible month.
    #[inline]
    pub const fn visible_month(&self) -> Month {
        self.visible_month
    }

    /// Returns the focused date.
    #[inline]
    pub const fn focused_date(&self) -> Date {
        self.focused
    }

    /// Returns the inclusive date bounds.
    #[inline]
    pub const fn bounds(&self) -> DateBounds {
        self.bounds
    }

    /// Returns the configured selection mode.
    #[inline]
    pub const fn selection_mode(&self) -> DateSelectionMode {
        self.mode
    }

    /// Returns the current selection, which may be incomplete for a range.
    #[inline]
    pub const fn selection(&self) -> DateSelection {
        self.selection
    }

    /// Builds a platform-neutral accessibility snapshot of this viewport.
    #[inline]
    pub fn semantics(&self) -> CalendarSemantics {
        CalendarSemantics::from_calendar(self)
    }

    /// Returns the visible month as up to 42 date-keyed cells.
    ///
    /// At the supported year-range edges, cells outside years `1..=9999` are
    /// omitted because the date model cannot represent them.
    pub fn cells(&self) -> Vec<CalendarCell> {
        let first = self.visible_month.first_day();
        let leading = i32::from(first.weekday().index());
        let Some(start) = first.add_days(-leading) else {
            return Vec::new();
        };
        (0..42)
            .filter_map(|offset| {
                let date = start.add_days(offset)?;
                Some(CalendarCell {
                    id: CalendarCellId::from_date(date),
                    date,
                    in_visible_month: date.month_key() == self.visible_month,
                    disabled: self.is_date_disabled(date),
                    selected: self.selection.contains(date),
                })
            })
            .collect()
    }

    /// Moves focus according to `navigation`.
    ///
    /// Day and week moves are rejected when their target is outside the date
    /// bounds. Month and year moves are rejected only when the target month has
    /// no dates in bounds; when it is partially bounded, focus is clamped to
    /// the nearest in-bounds date in that target month.
    pub fn navigate(&mut self, navigation: CalendarNavigation) -> bool {
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
        let candidate = match navigation {
            CalendarNavigation::PreviousDay => self.focused.add_days(-1),
            CalendarNavigation::NextDay => self.focused.add_days(1),
            CalendarNavigation::PreviousWeek => self.focused.add_days(-7),
            CalendarNavigation::NextWeek => self.focused.add_days(7),
            CalendarNavigation::PreviousMonth => self.bounded_month_target(-1),
            CalendarNavigation::NextMonth => self.bounded_month_target(1),
            CalendarNavigation::PreviousYear => self.bounded_month_target(-12),
            CalendarNavigation::NextYear => self.bounded_month_target(12),
        };
        let Some(candidate) = candidate else {
            return false;
        };
        let Some(candidate) = self.enabled_navigation_target(candidate, navigation, direction)
        else {
            return false;
        };
        self.focused = candidate;
        self.visible_month = candidate.month_key();
        true
    }

    fn enabled_navigation_target(
        &self,
        candidate: Date,
        navigation: CalendarNavigation,
        direction: i32,
    ) -> Option<Date> {
        if !self.bounds.contains(candidate) {
            return None;
        }
        if !self.is_explicitly_disabled(candidate) {
            return Some(candidate);
        }

        match navigation {
            CalendarNavigation::PreviousMonth
            | CalendarNavigation::NextMonth
            | CalendarNavigation::PreviousYear
            | CalendarNavigation::NextYear => {
                self.find_enabled_in_month(candidate.month_key(), candidate, direction)
            }
            CalendarNavigation::PreviousDay
            | CalendarNavigation::NextDay
            | CalendarNavigation::PreviousWeek
            | CalendarNavigation::NextWeek => self.find_enabled_by_day(candidate, direction),
        }
    }

    fn find_enabled_by_day(&self, start: Date, direction: i32) -> Option<Date> {
        let mut candidate = Some(start);
        while let Some(date) = candidate {
            if self.bounds.contains(date) && !self.is_explicitly_disabled(date) {
                return Some(date);
            }
            candidate = date.add_days(direction);
        }
        None
    }

    fn find_enabled_in_month(&self, month: Month, start: Date, direction: i32) -> Option<Date> {
        let mut candidate = Some(start);
        while let Some(date) = candidate {
            if date.month_key() != month {
                return None;
            }
            if self.bounds.contains(date) && !self.is_explicitly_disabled(date) {
                return Some(date);
            }
            candidate = date.add_days(direction);
        }
        None
    }

    fn is_explicitly_disabled(&self, date: Date) -> bool {
        self.disabled_dates.binary_search(&date).is_ok()
    }

    fn repair_focus_after_disabled_dates_change(&mut self) {
        if !self.is_explicitly_disabled(self.focused) {
            return;
        }
        if let Some(date) = self
            .focused
            .add_days(1)
            .and_then(|date| self.find_enabled_by_day(date, 1))
            .or_else(|| {
                self.focused
                    .add_days(-1)
                    .and_then(|date| self.find_enabled_by_day(date, -1))
            })
        {
            self.focused = date;
            self.visible_month = date.month_key();
        }
    }

    fn bounded_month_target(&self, offset: i32) -> Option<Date> {
        let target_month = self.focused.month_key().add_months(offset)?;
        let first = target_month.first_day();
        let last = Date::try_new(
            target_month.year(),
            target_month.month(),
            Date::days_in_month(target_month.year(), target_month.month())?,
        )
        .ok()?;

        if self.bounds.min().is_some_and(|min| min > last)
            || self.bounds.max().is_some_and(|max| max < first)
        {
            return None;
        }

        Some(self.bounds.clamp(self.focused.add_months(offset)?))
    }

    /// Moves focus to a bounded date without changing the selection.
    pub fn focus(&mut self, date: Date) -> Result<(), CalendarError> {
        if !self.bounds.contains(date) {
            return Err(CalendarError::OutOfBounds(date));
        }
        if self.is_explicitly_disabled(date) {
            return Err(CalendarError::DisabledDate(date));
        }
        self.focused = date;
        self.visible_month = date.month_key();
        Ok(())
    }

    /// Selects a date according to the configured single/range policy.
    pub fn select(&mut self, date: Date) -> Result<DateSelection, CalendarError> {
        if !self.bounds.contains(date) {
            return Err(CalendarError::OutOfBounds(date));
        }
        if self.disabled_dates.binary_search(&date).is_ok() {
            return Err(CalendarError::DisabledDate(date));
        }

        let next = match (self.mode, self.selection) {
            (DateSelectionMode::Single, _) => DateSelection::Single(Some(date)),
            (
                DateSelectionMode::Range(_),
                DateSelection::Range {
                    start: None,
                    end: _,
                },
            ) => DateSelection::Range {
                start: Some(date),
                end: None,
            },
            (
                DateSelectionMode::Range(policy),
                DateSelection::Range {
                    start: Some(start),
                    end: None,
                },
            ) if date == start && !policy.allow_same_day() => {
                return Err(CalendarError::SameDayRangeNotAllowed(date));
            }
            (
                DateSelectionMode::Range(_),
                DateSelection::Range {
                    start: Some(start),
                    end: None,
                },
            ) if date >= start => DateSelection::Range {
                start: Some(start),
                end: Some(date),
            },
            (
                DateSelectionMode::Range(policy),
                DateSelection::Range {
                    start: Some(start),
                    end: None,
                },
            ) => match policy.reverse_order() {
                RangeOrderPolicy::Swap => DateSelection::Range {
                    start: Some(date),
                    end: Some(start),
                },
                RangeOrderPolicy::Restart => DateSelection::Range {
                    start: Some(date),
                    end: None,
                },
            },
            (DateSelectionMode::Range(_), DateSelection::Range { .. }) => DateSelection::Range {
                start: Some(date),
                end: None,
            },
            _ => unreachable!("selection shape is always aligned with the mode"),
        };

        self.focus(date)?;
        self.selection = next;
        Ok(next)
    }

    /// Clears the current date or range selection.
    pub fn clear_selection(&mut self) {
        self.selection = DateSelection::empty(self.mode);
    }
}
