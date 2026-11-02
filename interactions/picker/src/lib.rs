//! Platform-neutral picker models for dates, times, colors, and overlay
//! dismissal.

mod calendar;
mod color;
mod date;
mod datetime;
mod overlay;
mod paint;
mod picker;
mod semantics;
mod theme;
mod widgets;

pub(crate) const CALENDAR_HEADER: f32 = 34.0;
pub(crate) const CALENDAR_WEEKDAYS: f32 = 24.0;
pub(crate) const PICKER_FIELD_HEIGHT: f32 = 42.0;
pub(crate) const PICKER_FOOTER_HEIGHT: f32 = 40.0;
pub(crate) const TIME_WHEEL_CONTENT_TOP: f32 = 8.0;
pub(crate) const TIME_WHEEL_ROW_HEIGHT: f32 = 32.0;
pub(crate) const TIME_WHEEL_ROWS: usize = 7;

pub use calendar::{
    Calendar, CalendarCell, CalendarCellId, CalendarError, CalendarNavigation, DateRangePolicy,
    DateSelection, DateSelectionMode, RangeOrderPolicy,
};
pub use color::{ColorChannel, ColorError, ColorKey, ColorPicker, Hsva, Rgba, Swatch, SwatchId};
pub use date::{Date, DateBounds, DateError, Month, Weekday};
pub use datetime::{
    DateTime, DateTimeBounds, DateTimeError, DateTimePicker, DateTimePickerError,
    DateTimePickerPolicy, TimeError, TimeOfDay, TimePicker, TimePickerError, TimeZonePolicy,
};
pub use overlay::{
    CancelReason, FocusRestorer, FocusTarget, OverlayAnchor, OverlayConsumer, OverlayRequest,
    PickerOutcome, PickerOverlay, PickerSession, PickerSessionError,
};
pub use picker::{DatePicker, DatePickerError};
pub use semantics::{
    CalendarCellSemantics, CalendarSemantics, ColorPickerSemantics, ColorSwatchSemantics,
    DatePickerSemantics, DateTimePickerSemantics, TimePickerSemantics,
};
pub use widgets::{
    CalendarSelectionCallback, CalendarView, CalendarViewState, ColorPickerView,
    ColorPickerViewState, ColorSelectionCallback, DatePickerView, DatePickerViewState,
    DateSelectionCallback, DateTimePickerView, DateTimePickerViewState, DateTimeSelectionCallback,
    TimePickerView, TimePickerViewState, TimeSelectionCallback,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_dates_are_rejected_at_the_public_constructor() {
        assert!(Date::try_new(2024, 2, 29).is_ok());
        assert!(Date::try_new(2023, 2, 29).is_err());
        assert!(Date::try_new(2024, 13, 1).is_err());
        assert!(Date::try_new(2024, 4, 31).is_err());
    }

    struct RecordingOverlay {
        presented: Option<OverlayRequest>,
        dismissed: Option<u32>,
    }

    impl OverlayConsumer for RecordingOverlay {
        type Handle = u32;

        fn present(&mut self, request: OverlayRequest) -> Self::Handle {
            self.presented = Some(request);
            7
        }

        fn dismiss(&mut self, handle: Self::Handle) {
            self.dismissed = Some(handle);
        }
    }

    struct RecordingFocus {
        restored: Option<FocusTarget>,
    }

    impl FocusRestorer for RecordingFocus {
        fn restore_focus(&mut self, target: FocusTarget) {
            self.restored = Some(target);
        }
    }

    #[test]
    fn cancellation_discards_draft_and_restores_focus_through_the_host_seams() {
        let mut session = PickerSession::new(11_u8);
        session.open();
        session.set_draft(22).unwrap();
        assert_eq!(
            session.cancel(CancelReason::Escape).unwrap(),
            PickerOutcome::Cancelled {
                reason: CancelReason::Escape,
                value: 11,
            }
        );
        assert_eq!(session.committed(), &11);
        assert!(!session.is_open());

        let mut overlay = RecordingOverlay {
            presented: None,
            dismissed: None,
        };
        let request = OverlayRequest::new(OverlayAnchor::new(3), true);
        let picker_overlay = PickerOverlay::present(&mut overlay, request, FocusTarget::new(99));
        assert_eq!(overlay.presented, Some(request));

        let mut focus = RecordingFocus { restored: None };
        picker_overlay.close(&mut overlay, &mut focus);
        assert_eq!(overlay.dismissed, Some(7));
        assert_eq!(focus.restored, Some(FocusTarget::new(99)));
    }

    #[test]
    fn calendar_cells_are_date_keyed_and_range_selection_honors_bounds() {
        let focused = Date::try_new(2024, 5, 15).unwrap();
        let minimum = Date::try_new(2024, 5, 10).unwrap();
        let maximum = Date::try_new(2024, 6, 20).unwrap();
        let bounds = DateBounds::new(Some(minimum), Some(maximum)).unwrap();
        let mode = DateSelectionMode::Range(DateRangePolicy::inclusive());
        let mut calendar = Calendar::try_new(focused, bounds, mode).unwrap();

        let key_before_navigation = CalendarCellId::from_date(focused);
        assert!(
            calendar
                .cells()
                .iter()
                .any(|cell| cell.id() == key_before_navigation)
        );
        assert_eq!(calendar.navigate(CalendarNavigation::NextMonth), true);
        assert_eq!(calendar.focused_date(), Date::try_new(2024, 6, 15).unwrap());
        assert_eq!(CalendarCellId::from_date(focused).date(), focused);
        assert!(
            calendar
                .cells()
                .iter()
                .all(|cell| cell.id() == CalendarCellId::from_date(cell.date()))
        );

        assert_eq!(
            calendar
                .select(Date::try_new(2024, 5, 15).unwrap())
                .unwrap(),
            DateSelection::Range {
                start: Some(Date::try_new(2024, 5, 15).unwrap()),
                end: None,
            }
        );
        assert_eq!(
            calendar
                .select(Date::try_new(2024, 5, 10).unwrap())
                .unwrap(),
            DateSelection::Range {
                start: Some(minimum),
                end: Some(Date::try_new(2024, 5, 15).unwrap()),
            }
        );
        assert!(matches!(
            calendar.select(Date::try_new(2024, 6, 21).unwrap()),
            Err(CalendarError::OutOfBounds(_))
        ));
    }

    #[test]
    fn color_boundaries_keyboard_steps_and_disabled_swatches_are_explicit() {
        let red = Hsva::try_new(0, 100, 100, 100).unwrap();
        assert_eq!(red.to_rgba(), Rgba::new(255, 0, 0, 255));
        assert!(Hsva::try_new(361, 100, 100, 100).is_err());

        let mut picker = ColorPicker::new(red, true);
        picker.open();
        picker.set_steps(30, 10).unwrap();
        picker.handle_key(ColorChannel::Hue, ColorKey::End).unwrap();
        assert_eq!(picker.draft().hue(), 360);
        picker
            .handle_key(ColorChannel::Hue, ColorKey::Increase)
            .unwrap();
        assert_eq!(picker.draft().hue(), 360);
        picker
            .handle_key(ColorChannel::Hue, ColorKey::Decrease)
            .unwrap();
        assert_eq!(picker.draft().hue(), 330);

        let disabled = Swatch::new(
            SwatchId::new(1),
            Hsva::try_new(120, 100, 100, 100).unwrap(),
            true,
        );
        let enabled = Swatch::new(
            SwatchId::new(2),
            Hsva::try_new(240, 100, 100, 100).unwrap(),
            false,
        );
        picker.add_swatch(disabled).unwrap();
        picker.add_swatch(enabled).unwrap();
        assert_eq!(
            picker.select_swatch(SwatchId::new(1)),
            Err(ColorError::DisabledSwatch(SwatchId::new(1)))
        );
        picker.select_swatch(SwatchId::new(2)).unwrap();
        assert_eq!(picker.draft().hue(), 240);

        let mut no_alpha = ColorPicker::new(red, false);
        no_alpha.open();
        assert_eq!(
            no_alpha.handle_key(ColorChannel::Alpha, ColorKey::End),
            Err(ColorError::AlphaDisabled)
        );
    }

    #[test]
    fn date_picker_confirms_or_cancels_a_bounded_draft_without_leaking_state() {
        let initial = Date::try_new(2024, 5, 15).unwrap();
        let minimum = Date::try_new(2024, 5, 10).unwrap();
        let maximum = Date::try_new(2024, 5, 20).unwrap();
        let bounds = DateBounds::new(Some(minimum), Some(maximum)).unwrap();
        let mut picker = DatePicker::try_new(Some(initial), bounds).unwrap();

        picker.open();
        picker.select(maximum).unwrap();
        assert_eq!(picker.draft(), DateSelection::Single(Some(maximum)));
        picker.cancel(CancelReason::OutsideClick).unwrap();
        assert_eq!(picker.selection(), DateSelection::Single(Some(initial)));

        picker.open();
        assert!(!picker.navigate(CalendarNavigation::NextMonth).unwrap());
        picker.select(minimum).unwrap();
        assert_eq!(
            picker.confirm().unwrap(),
            PickerOutcome::Confirmed(DateSelection::Single(Some(minimum)))
        );
        assert_eq!(picker.selection(), DateSelection::Single(Some(minimum)));
    }

    #[test]
    fn datetime_picker_has_explicit_timezone_bounds_and_navigation_edges() {
        let zone = TimeZonePolicy::fixed_offset(330).unwrap();
        let other_zone = TimeZonePolicy::Utc;
        let date = Date::try_new(2024, 1, 10).unwrap();
        let time = TimeOfDay::try_new(9, 30, 0, 0).unwrap();
        assert!(TimeOfDay::try_new(24, 0, 0, 0).is_err());
        assert!(TimeZonePolicy::fixed_offset(1_440).is_err());

        let minimum = DateTime::try_new(date, TimeOfDay::midnight(), zone).unwrap();
        let maximum =
            DateTime::try_new(date, TimeOfDay::try_new(23, 59, 59, 0).unwrap(), zone).unwrap();
        let policy = DateTimePickerPolicy::try_new(zone, Some(minimum), Some(maximum)).unwrap();
        assert!(DateTimePickerPolicy::try_new(other_zone, Some(minimum), None).is_err());

        let initial = DateTime::try_new(date, time, zone).unwrap();
        let mut picker = DateTimePicker::try_new(Some(initial), policy).unwrap();
        picker.open();
        assert!(!picker.navigate(CalendarNavigation::PreviousDay).unwrap());
        let later = TimeOfDay::try_new(10, 15, 0, 0).unwrap();
        picker.set_time(later).unwrap();
        assert_eq!(picker.draft().unwrap().time(), later);
        picker.cancel(CancelReason::Escape).unwrap();
        assert_eq!(picker.value(), Some(initial));
    }
}
