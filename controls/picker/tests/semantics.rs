use aimer_picker::{
    Calendar, CalendarCellId, ColorPicker, Date, DateBounds, DatePicker, DateSelectionMode,
    DateTime, DateTimePicker, DateTimePickerPolicy, Hsva, Swatch, SwatchId, TimeOfDay,
    TimeZonePolicy,
};

#[test]
fn calendar_publishes_date_keyed_accessible_cells() {
    let focused = Date::try_new(2024, 5, 15).unwrap();
    let disabled = Date::try_new(2024, 5, 16).unwrap();
    let selected = Date::try_new(2024, 5, 17).unwrap();
    let mut calendar =
        Calendar::try_new(focused, DateBounds::unbounded(), DateSelectionMode::Single)
            .unwrap()
            .with_disabled_dates([disabled]);
    calendar.select(selected).unwrap();

    let semantics = calendar.semantics();
    assert_eq!(semantics.focused_date(), selected);
    assert_eq!(semantics.selection().is_complete(), true);

    let selected_cell = semantics
        .cell(CalendarCellId::from_date(selected))
        .expect("selected date is visible in the month snapshot");
    assert!(selected_cell.is_selected());
    assert!(selected_cell.is_focused());
    assert!(selected_cell.is_focusable());

    let disabled_cell = semantics
        .cell(CalendarCellId::from_date(disabled))
        .expect("disabled date remains visible to accessibility");
    assert!(disabled_cell.is_disabled());
    assert!(!disabled_cell.is_focusable());
}

#[test]
fn date_picker_semantics_keep_committed_and_draft_values_distinct() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let next = Date::try_new(2024, 5, 16).unwrap();
    let mut picker = DatePicker::new(Some(initial));

    picker.open();
    picker.select(next).unwrap();

    let semantics = picker.semantics();
    assert!(semantics.is_open());
    assert_eq!(
        semantics.selection(),
        aimer_picker::DateSelection::Single(Some(initial))
    );
    assert_eq!(
        semantics.draft(),
        aimer_picker::DateSelection::Single(Some(next))
    );
    assert_eq!(semantics.calendar().focused_date(), next);
}

#[test]
fn datetime_picker_semantics_expose_policy_and_draft_state() {
    let timezone = TimeZonePolicy::fixed_offset(330).unwrap();
    let date = Date::try_new(2024, 5, 15).unwrap();
    let initial = DateTime::try_new(date, TimeOfDay::midnight(), timezone).unwrap();
    let policy = DateTimePickerPolicy::unbounded(timezone);
    let mut picker = DateTimePicker::try_new(Some(initial), policy).unwrap();
    let next_time = TimeOfDay::try_new(14, 30, 0, 0).unwrap();

    picker.open();
    picker.set_time(next_time).unwrap();

    let semantics = picker.semantics();
    assert!(semantics.is_open());
    assert_eq!(semantics.value(), Some(initial));
    assert_eq!(semantics.draft().unwrap().time(), next_time);
    assert_eq!(semantics.policy(), policy);
    assert_eq!(semantics.timezone(), timezone);
}

#[test]
fn color_picker_semantics_expose_alpha_and_swatch_states() {
    let initial = Hsva::try_new(0, 100, 100, 100).unwrap();
    let disabled = Swatch::new(
        SwatchId::new(1),
        Hsva::try_new(120, 100, 100, 100).unwrap(),
        true,
    );
    let selected = Swatch::new(
        SwatchId::new(2),
        Hsva::try_new(240, 100, 100, 100).unwrap(),
        false,
    );
    let mut picker = ColorPicker::new(initial, true);
    picker.add_swatch(disabled).unwrap();
    picker.add_swatch(selected).unwrap();
    picker.open();
    picker.select_swatch(selected.id()).unwrap();

    let semantics = picker.semantics();
    assert!(semantics.is_open());
    assert!(semantics.alpha_enabled());
    assert_eq!(semantics.value(), initial);
    assert_eq!(semantics.draft(), selected.color());
    assert!(semantics.swatch(disabled.id()).unwrap().is_disabled());
    assert!(!semantics.swatch(disabled.id()).unwrap().is_selected());
    assert!(semantics.swatch(selected.id()).unwrap().is_selected());
}
