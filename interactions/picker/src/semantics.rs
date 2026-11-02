//! Platform-neutral accessibility snapshots for picker controls.

use super::{
    Calendar, CalendarCellId, ColorPicker, Date, DatePicker, DateSelection, DateTime,
    DateTimePicker, DateTimePickerPolicy, Hsva, Month, SwatchId, TimeOfDay, TimePicker,
    TimeZonePolicy,
};

/// Accessibility state for one visible calendar cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CalendarCellSemantics {
    id: CalendarCellId,
    date: Date,
    in_visible_month: bool,
    disabled: bool,
    selected: bool,
    focused: bool,
}

impl CalendarCellSemantics {
    /// Returns the stable date-based cell identity.
    #[inline]
    pub const fn id(self) -> CalendarCellId {
        self.id
    }

    /// Returns the date represented by this cell.
    #[inline]
    pub const fn date(self) -> Date {
        self.date
    }

    /// Returns whether the cell belongs to the visible month.
    #[inline]
    pub const fn in_visible_month(self) -> bool {
        self.in_visible_month
    }

    /// Returns whether this date cannot be selected.
    #[inline]
    pub const fn is_disabled(self) -> bool {
        self.disabled
    }

    /// Returns whether this date is part of the current selection.
    #[inline]
    pub const fn is_selected(self) -> bool {
        self.selected
    }

    /// Returns whether keyboard focus is on this date.
    #[inline]
    pub const fn is_focused(self) -> bool {
        self.focused
    }

    /// Returns whether a platform adapter may focus this cell.
    #[inline]
    pub const fn is_focusable(self) -> bool {
        !self.disabled
    }
}

/// An accessibility snapshot of the calendar viewport and its cells.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarSemantics {
    visible_month: Month,
    focused_date: Date,
    selection: DateSelection,
    cells: Vec<CalendarCellSemantics>,
}

impl CalendarSemantics {
    /// Returns the month currently visible to the user.
    #[inline]
    pub const fn visible_month(&self) -> Month {
        self.visible_month
    }

    /// Returns the date currently holding calendar focus.
    #[inline]
    pub const fn focused_date(&self) -> Date {
        self.focused_date
    }

    /// Returns the current date or range selection.
    #[inline]
    pub const fn selection(&self) -> DateSelection {
        self.selection
    }

    /// Returns the visible cells in deterministic row-major order.
    #[inline]
    pub fn cells(&self) -> &[CalendarCellSemantics] {
        &self.cells
    }

    /// Finds a visible cell by its stable identity.
    #[inline]
    pub fn cell(&self, id: CalendarCellId) -> Option<&CalendarCellSemantics> {
        self.cells.iter().find(|cell| cell.id() == id)
    }

    pub(crate) fn from_calendar(calendar: &Calendar) -> Self {
        let focused_date = calendar.focused_date();
        let cells = calendar
            .cells()
            .into_iter()
            .map(|cell| CalendarCellSemantics {
                id: cell.id(),
                date: cell.date(),
                in_visible_month: cell.in_visible_month(),
                disabled: cell.is_disabled(),
                selected: cell.is_selected(),
                focused: cell.date() == focused_date,
            })
            .collect();
        Self {
            visible_month: calendar.visible_month(),
            focused_date,
            selection: calendar.selection(),
            cells,
        }
    }
}

/// An accessibility snapshot of a transactional date picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatePickerSemantics {
    selection: DateSelection,
    draft: DateSelection,
    open: bool,
    calendar: CalendarSemantics,
}

impl DatePickerSemantics {
    /// Returns the last confirmed selection.
    #[inline]
    pub const fn selection(&self) -> DateSelection {
        self.selection
    }

    /// Returns the editable selection, which may not be committed yet.
    #[inline]
    pub const fn draft(&self) -> DateSelection {
        self.draft
    }

    /// Returns whether the picker is currently editable.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Returns the calendar snapshot used to edit the draft.
    #[inline]
    pub const fn calendar(&self) -> &CalendarSemantics {
        &self.calendar
    }

    pub(crate) fn from_picker(picker: &DatePicker) -> Self {
        Self {
            selection: picker.selection(),
            draft: picker.draft(),
            open: picker.is_open(),
            calendar: picker.calendar().semantics(),
        }
    }
}

/// An accessibility snapshot of one caller-owned color swatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColorSwatchSemantics {
    id: SwatchId,
    color: Hsva,
    disabled: bool,
    selected: bool,
}

impl ColorSwatchSemantics {
    /// Returns the stable application-owned swatch identity.
    #[inline]
    pub const fn id(self) -> SwatchId {
        self.id
    }

    /// Returns the HSVA value represented by the swatch.
    #[inline]
    pub const fn color(self) -> Hsva {
        self.color
    }

    /// Returns whether this swatch is unavailable for selection.
    #[inline]
    pub const fn is_disabled(self) -> bool {
        self.disabled
    }

    /// Returns whether this swatch matches the current editable color.
    #[inline]
    pub const fn is_selected(self) -> bool {
        self.selected
    }
}

/// An accessibility snapshot of a transactional color picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorPickerSemantics {
    value: Hsva,
    draft: Hsva,
    open: bool,
    alpha_enabled: bool,
    swatches: Vec<ColorSwatchSemantics>,
}

impl ColorPickerSemantics {
    /// Returns the last confirmed color.
    #[inline]
    pub const fn value(&self) -> Hsva {
        self.value
    }

    /// Returns the current editable color.
    #[inline]
    pub const fn draft(&self) -> Hsva {
        self.draft
    }

    /// Returns whether the picker is currently editable.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Returns whether the alpha channel is an interactive control.
    #[inline]
    pub const fn alpha_enabled(&self) -> bool {
        self.alpha_enabled
    }

    /// Returns swatches in their caller-provided insertion order.
    #[inline]
    pub fn swatches(&self) -> &[ColorSwatchSemantics] {
        &self.swatches
    }

    /// Finds a swatch by its stable application-owned identity.
    #[inline]
    pub fn swatch(&self, id: SwatchId) -> Option<&ColorSwatchSemantics> {
        self.swatches.iter().find(|swatch| swatch.id() == id)
    }

    pub(crate) fn from_picker(picker: &ColorPicker) -> Self {
        let draft = picker.draft();
        let swatches = picker
            .swatches()
            .iter()
            .map(|swatch| ColorSwatchSemantics {
                id: swatch.id(),
                color: swatch.color(),
                disabled: swatch.is_disabled(),
                selected: swatch.color() == draft,
            })
            .collect();
        Self {
            value: picker.value(),
            draft,
            open: picker.is_open(),
            alpha_enabled: picker.alpha_enabled(),
            swatches,
        }
    }
}

/// An accessibility snapshot of a transactional date-time picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTimePickerSemantics {
    value: Option<DateTime>,
    draft: Option<DateTime>,
    open: bool,
    policy: DateTimePickerPolicy,
}

impl DateTimePickerSemantics {
    /// Returns the last confirmed date-time.
    #[inline]
    pub const fn value(self) -> Option<DateTime> {
        self.value
    }

    /// Returns the current editable date-time.
    #[inline]
    pub const fn draft(self) -> Option<DateTime> {
        self.draft
    }

    /// Returns whether the picker is currently editable.
    #[inline]
    pub const fn is_open(self) -> bool {
        self.open
    }

    /// Returns the explicit timezone and bounds policy.
    #[inline]
    pub const fn policy(self) -> DateTimePickerPolicy {
        self.policy
    }

    /// Returns the timezone used to interpret all values.
    #[inline]
    pub const fn timezone(self) -> TimeZonePolicy {
        self.policy.timezone()
    }

    pub(crate) fn from_picker(picker: &DateTimePicker) -> Self {
        Self {
            value: picker.value(),
            draft: picker.draft(),
            open: picker.is_open(),
            policy: picker.policy(),
        }
    }
}

/// An accessibility snapshot of a transactional standalone time picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimePickerSemantics {
    value: TimeOfDay,
    draft: TimeOfDay,
    open: bool,
}

impl TimePickerSemantics {
    /// Returns the last confirmed time.
    #[inline]
    pub const fn value(self) -> TimeOfDay {
        self.value
    }

    /// Returns the current editable time.
    #[inline]
    pub const fn draft(self) -> TimeOfDay {
        self.draft
    }

    /// Returns whether the picker is currently editable.
    #[inline]
    pub const fn is_open(self) -> bool {
        self.open
    }

    pub(crate) fn from_picker(picker: &TimePicker) -> Self {
        Self {
            value: picker.value(),
            draft: picker.draft(),
            open: picker.is_open(),
        }
    }
}
