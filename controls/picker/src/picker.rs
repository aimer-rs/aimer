//! Transactional single-date and range picker models.

use core::fmt;

use super::semantics::DatePickerSemantics;
use super::{
    Calendar, CalendarError, CancelReason, Date, DateBounds, DateSelection, DateSelectionMode,
    FocusRestorer, FocusTarget, OverlayConsumer, OverlayRequest, PickerOutcome, PickerOverlay,
    PickerSession, PickerSessionError,
};

/// Errors returned by [`DatePicker`] operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatePickerError {
    /// The requested calendar operation was rejected.
    Calendar(CalendarError),
    /// The picker must be opened before it can be edited or closed.
    Closed,
    /// The initial selection does not match the requested selection mode.
    ModeMismatch,
    /// A range must have both endpoints before it can be confirmed.
    IncompleteRange,
    /// No overlay host is installed for the presentation request.
    MissingHost,
    /// An installed overlay host cannot represent the presentation request.
    UnsupportedHost,
    /// The overlay policy does not allow this user-driven dismissal reason.
    DismissalNotAllowed(CancelReason),
}

impl fmt::Display for DatePickerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Calendar(error) => error.fmt(formatter),
            Self::Closed => formatter.write_str("date picker is closed"),
            Self::ModeMismatch => {
                formatter.write_str("initial selection does not match picker mode")
            }
            Self::IncompleteRange => formatter.write_str("date range is incomplete"),
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

impl std::error::Error for DatePickerError {}

impl From<CalendarError> for DatePickerError {
    fn from(error: CalendarError) -> Self {
        Self::Calendar(error)
    }
}

impl From<PickerSessionError> for DatePickerError {
    fn from(error: PickerSessionError) -> Self {
        match error {
            PickerSessionError::Closed => Self::Closed,
            PickerSessionError::MissingHost => Self::MissingHost,
            PickerSessionError::UnsupportedHost => Self::UnsupportedHost,
            PickerSessionError::DismissalNotAllowed(reason) => Self::DismissalNotAllowed(reason),
        }
    }
}

/// A date picker with an explicit calendar bounds and selection policy.
///
/// The calendar is the draft editor. [`DatePicker::confirm`] commits it, and
/// [`DatePicker::cancel`] discards it. Overlay presentation and focus return
/// remain caller-owned through [`crate::PickerOverlay`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatePicker {
    calendar: Calendar,
    session: PickerSession<DateSelection>,
}

impl DatePicker {
    /// Creates an unbounded single-date picker.
    pub fn new(initial: Option<Date>) -> Self {
        Self::try_new(initial, DateBounds::unbounded())
            .expect("an unbounded picker accepts every valid initial date")
    }

    /// Creates a bounded single-date picker.
    pub fn try_new(initial: Option<Date>, bounds: DateBounds) -> Result<Self, DatePickerError> {
        Self::try_with_selection(
            DateSelection::Single(initial),
            bounds,
            DateSelectionMode::Single,
        )
    }

    /// Creates a range picker with an explicit reverse-order and same-day
    /// policy.
    pub fn try_range(
        initial: DateSelection,
        bounds: DateBounds,
        policy: super::DateRangePolicy,
    ) -> Result<Self, DatePickerError> {
        Self::try_with_selection(initial, bounds, DateSelectionMode::Range(policy))
    }

    /// Creates a picker from an explicit initial selection and mode.
    pub fn try_with_selection(
        initial: DateSelection,
        bounds: DateBounds,
        mode: DateSelectionMode,
    ) -> Result<Self, DatePickerError> {
        if !selection_matches_mode(initial, mode) {
            return Err(DatePickerError::ModeMismatch);
        }
        let focus = initial_focus(initial, bounds);
        let mut calendar = Calendar::try_new(focus, bounds, mode)?;
        install_selection(&mut calendar, initial)?;
        let selection = calendar.selection();
        Ok(Self {
            calendar,
            session: PickerSession::new(selection),
        })
    }

    /// Returns a picker that marks the supplied dates unavailable.
    ///
    /// Disabled dates remain visible in the calendar but cannot be selected.
    /// The dates are sorted and deduplicated by the underlying calendar.
    pub fn with_disabled_dates<I>(mut self, dates: I) -> Self
    where
        I: IntoIterator<Item = Date>,
    {
        self.calendar = self.calendar.with_disabled_dates(dates);
        self
    }

    /// Replaces the dates unavailable for selection.
    pub fn set_disabled_dates<I>(&mut self, dates: I)
    where
        I: IntoIterator<Item = Date>,
    {
        self.calendar.set_disabled_dates(dates);
    }

    /// Returns whether `date` is unavailable because of bounds or a disabled
    /// date rule.
    #[inline]
    pub fn is_date_disabled(&self, date: Date) -> bool {
        self.calendar.is_date_disabled(date)
    }

    /// Returns the explicitly disabled dates in sorted order.
    #[inline]
    pub fn disabled_dates(&self) -> &[Date] {
        self.calendar.disabled_dates()
    }

    /// Opens the picker and resets the calendar draft to the last confirmation.
    pub fn open(&mut self) {
        self.session.open();
        self.sync_calendar_to_draft();
    }

    /// Presents the picker through a checked caller-owned overlay host and
    /// opens its transactional draft.
    pub fn open_with_overlay<C: OverlayConsumer>(
        &mut self,
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<PickerOverlay<C::Handle>, DatePickerError> {
        let overlay = self
            .session
            .open_with_overlay(consumer, request, restore_focus)
            .map_err(DatePickerError::from)?;
        self.sync_calendar_to_draft();
        Ok(overlay)
    }

    /// Returns whether the picker is open.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.session.is_open()
    }

    /// Returns the committed date or range.
    #[inline]
    pub const fn selection(&self) -> DateSelection {
        *self.session.committed()
    }

    /// Returns the current calendar draft, or the committed value while closed.
    #[inline]
    pub fn draft(&self) -> DateSelection {
        if self.is_open() {
            self.calendar.selection()
        } else {
            *self.session.draft()
        }
    }

    /// Builds a platform-neutral accessibility snapshot of this picker.
    #[inline]
    pub fn semantics(&self) -> DatePickerSemantics {
        DatePickerSemantics::from_picker(self)
    }

    /// Returns the committed single selected date, if this is a single picker.
    #[inline]
    pub fn selected_date(&self) -> Option<Date> {
        match self.selection() {
            DateSelection::Single(date) => date,
            DateSelection::Range { .. } => None,
        }
    }

    /// Returns the calendar draft model used by the picker.
    #[inline]
    pub const fn calendar(&self) -> &Calendar {
        &self.calendar
    }

    /// Moves focus in the open calendar.
    pub fn navigate(
        &mut self,
        navigation: super::CalendarNavigation,
    ) -> Result<bool, DatePickerError> {
        self.ensure_open()?;
        Ok(self.calendar.navigate(navigation))
    }

    /// Selects a bounded date in the open calendar.
    pub fn select(&mut self, date: Date) -> Result<DateSelection, DatePickerError> {
        self.ensure_open()?;
        self.calendar.select(date).map_err(Into::into)
    }

    /// Confirms the current draft and closes the picker.
    pub fn confirm(&mut self) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        self.ensure_open()?;
        let draft = self.calendar.selection();
        if matches!(draft, DateSelection::Range { .. }) && !draft.is_complete() {
            return Err(DatePickerError::IncompleteRange);
        }
        self.session.set_draft(draft)?;
        Ok(self.session.confirm()?)
    }

    /// Confirms the calendar draft, dismisses its overlay, and restores focus.
    pub fn confirm_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        if !self.is_open() {
            let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
            return Err(DatePickerError::Closed);
        }
        let draft = self.calendar.selection();
        if matches!(draft, DateSelection::Range { .. }) && !draft.is_complete() {
            return Err(DatePickerError::IncompleteRange);
        }
        self.session.set_draft(draft)?;
        Ok(self
            .session
            .confirm_with_overlay(overlay, consumer, focus)?)
    }

    /// Cancels the current draft and closes the picker.
    pub fn cancel(
        &mut self,
        reason: super::CancelReason,
    ) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        self.ensure_open()?;
        let outcome = self.session.cancel(reason)?;
        self.sync_calendar_to_draft();
        Ok(outcome)
    }

    /// Cancels the calendar draft according to the overlay policy, dismisses
    /// its overlay, and restores focus.
    pub fn cancel_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        let outcome = self
            .session
            .cancel_with_overlay(overlay, reason, consumer, focus)
            .map_err(DatePickerError::from)?;
        self.sync_calendar_to_draft();
        Ok(outcome)
    }

    /// Closes the picker as a programmatic cancellation.
    pub fn close(&mut self) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        self.cancel(CancelReason::Programmatic)
    }

    /// Closes the picker overlay as a programmatic cancellation and restores
    /// focus to its opening control.
    pub fn close_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<DateSelection>, DatePickerError> {
        self.cancel_with_overlay(overlay, CancelReason::Programmatic, consumer, focus)
    }

    fn ensure_open(&self) -> Result<(), DatePickerError> {
        if self.is_open() {
            Ok(())
        } else {
            Err(DatePickerError::Closed)
        }
    }

    fn sync_calendar_to_draft(&mut self) {
        let selection = *self.session.draft();
        self.calendar.clear_selection();
        let _ = install_selection(&mut self.calendar, selection);
    }
}

fn selection_matches_mode(selection: DateSelection, mode: DateSelectionMode) -> bool {
    matches!(
        (selection, mode),
        (DateSelection::Single(_), DateSelectionMode::Single)
            | (DateSelection::Range { .. }, DateSelectionMode::Range(_))
    )
}

fn initial_focus(selection: DateSelection, bounds: DateBounds) -> Date {
    let candidate = match selection {
        DateSelection::Single(Some(date)) => Some(date),
        DateSelection::Range {
            start: Some(date), ..
        } => Some(date),
        DateSelection::Single(None) | DateSelection::Range { start: None, .. } => None,
    };
    candidate
        .or(bounds.min())
        .or(bounds.max())
        .unwrap_or_else(|| Date::try_new(1970, 1, 1).expect("the epoch is a valid date"))
}

fn install_selection(
    calendar: &mut Calendar,
    selection: DateSelection,
) -> Result<(), CalendarError> {
    match selection {
        DateSelection::Single(Some(date)) => {
            calendar.select(date)?;
        }
        DateSelection::Single(None)
        | DateSelection::Range {
            start: None,
            end: None,
        } => {}
        DateSelection::Range {
            start: Some(start),
            end: None,
        } => {
            calendar.select(start)?;
        }
        DateSelection::Range {
            start: Some(start),
            end: Some(end),
        } => {
            calendar.select(start)?;
            calendar.select(end)?;
        }
        DateSelection::Range {
            start: None,
            end: Some(end),
        } => {
            calendar.select(end)?;
        }
    }
    Ok(())
}
