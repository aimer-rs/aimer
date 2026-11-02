use aimer_picker::{
    CalendarNavigation, CancelReason, Date, DateTime, DateTimeBounds, DateTimeError,
    DateTimePicker, DateTimePickerError, DateTimePickerPolicy, PickerOutcome, TimeOfDay,
    TimePicker, TimeZonePolicy,
};

#[test]
fn bounds_reject_values_with_a_different_timezone() {
    let date = Date::try_new(2024, 6, 15).unwrap();
    let utc = TimeZonePolicy::Utc;
    let offset = TimeZonePolicy::fixed_offset(60).unwrap();
    let minimum = DateTime::try_new(date, TimeOfDay::midnight(), utc).unwrap();
    let maximum = DateTime::try_new(date, TimeOfDay::try_new(23, 59, 59, 0).unwrap(), utc).unwrap();
    let bounds = DateTimeBounds::try_new(Some(minimum), Some(maximum)).unwrap();
    let value = DateTime::try_new(date, TimeOfDay::try_new(12, 0, 0, 0).unwrap(), offset).unwrap();

    assert!(!bounds.contains(value));
}

#[test]
fn time_of_day_accepts_finite_components_and_rejects_each_upper_boundary() {
    let latest = TimeOfDay::try_new(23, 59, 59, 999_999_999).unwrap();
    assert_eq!(latest.hour(), 23);
    assert_eq!(latest.minute(), 59);
    assert_eq!(latest.second(), 59);
    assert_eq!(latest.nanosecond(), 999_999_999);

    assert!(matches!(
        TimeOfDay::try_new(24, 0, 0, 0),
        Err(aimer_picker::TimeError::InvalidHour(24))
    ));
    assert!(matches!(
        TimeOfDay::try_new(0, 60, 0, 0),
        Err(aimer_picker::TimeError::InvalidMinute(60))
    ));
    assert!(matches!(
        TimeOfDay::try_new(0, 0, 60, 0),
        Err(aimer_picker::TimeError::InvalidSecond(60))
    ));
    assert!(matches!(
        TimeOfDay::try_new(0, 0, 0, 1_000_000_000),
        Err(aimer_picker::TimeError::InvalidNanosecond(1_000_000_000))
    ));
}

#[test]
fn time_picker_commits_a_changed_draft_and_cancels_without_leaking_it() {
    let initial = TimeOfDay::try_new(9, 30, 45, 123).unwrap();
    let changed = TimeOfDay::try_new(18, 5, 12, 456).unwrap();
    let mut picker = TimePicker::new(Some(initial));

    assert_eq!(picker.value(), initial);
    picker.open();
    assert_eq!(picker.draft(), initial);
    picker.set_time(changed).unwrap();
    assert_eq!(
        picker.cancel(CancelReason::Escape).unwrap(),
        PickerOutcome::Cancelled {
            reason: CancelReason::Escape,
            value: initial,
        }
    );
    assert_eq!(picker.value(), initial);

    picker.open();
    picker.set_time(changed).unwrap();
    assert_eq!(picker.confirm().unwrap(), PickerOutcome::Confirmed(changed));
    assert_eq!(picker.value(), changed);
}

#[test]
fn bounds_are_inclusive_and_reversed_ranges_are_rejected() {
    let date = Date::try_new(2024, 6, 15).unwrap();
    let zone = TimeZonePolicy::Utc;
    let minimum = DateTime::try_new(date, TimeOfDay::try_new(9, 30, 0, 0).unwrap(), zone).unwrap();
    let maximum = DateTime::try_new(date, TimeOfDay::try_new(17, 45, 0, 0).unwrap(), zone).unwrap();
    let bounds = DateTimeBounds::try_new(Some(minimum), Some(maximum)).unwrap();

    assert!(bounds.contains(minimum));
    assert!(bounds.contains(maximum));
    assert!(!bounds.contains(
        DateTime::try_new(date, TimeOfDay::try_new(9, 29, 59, 0).unwrap(), zone).unwrap()
    ));
    assert!(!bounds.contains(
        DateTime::try_new(date, TimeOfDay::try_new(17, 45, 0, 1).unwrap(), zone).unwrap()
    ));
    assert_eq!(
        DateTimeBounds::try_new(Some(maximum), Some(minimum)),
        Err(DateTimeError::ReversedBounds {
            min: Some(maximum),
            max: Some(minimum)
        })
    );
}

#[test]
fn timezone_policy_is_explicit_and_bound_values_must_match_it() {
    assert_eq!(TimeZonePolicy::Utc.offset_minutes(), 0);
    assert_eq!(
        TimeZonePolicy::fixed_offset(-1_439)
            .unwrap()
            .offset_minutes(),
        -1_439
    );
    assert_eq!(
        TimeZonePolicy::fixed_offset(1_439)
            .unwrap()
            .offset_minutes(),
        1_439
    );
    assert!(matches!(
        TimeZonePolicy::fixed_offset(-1_440),
        Err(DateTimeError::InvalidOffset(-1_440))
    ));
    assert!(matches!(
        TimeZonePolicy::fixed_offset(1_440),
        Err(DateTimeError::InvalidOffset(1_440))
    ));

    let date = Date::try_new(2024, 6, 15).unwrap();
    let utc = DateTime::try_new(date, TimeOfDay::midnight(), TimeZonePolicy::Utc).unwrap();
    let offset = TimeZonePolicy::fixed_offset(60).unwrap();
    assert_eq!(
        aimer_picker::DateTimePickerPolicy::try_new(offset, Some(utc), None),
        Err(DateTimeError::TimeZoneMismatch {
            expected: offset,
            actual: TimeZonePolicy::Utc
        })
    );
}

#[test]
fn date_time_rejects_an_invalid_public_fixed_offset_variant() {
    let date = Date::try_new(2024, 6, 15).unwrap();
    let invalid = TimeZonePolicy::FixedOffset { minutes: 1_440 };

    assert_eq!(
        DateTime::try_new(date, TimeOfDay::midnight(), invalid),
        Err(DateTimeError::InvalidOffset(1_440))
    );
    assert_eq!(
        DateTimePickerPolicy::try_new(invalid, None, None),
        Err(DateTimeError::InvalidOffset(1_440))
    );
    assert_eq!(
        DateTimePicker::try_new(None, DateTimePickerPolicy::unbounded(invalid)),
        Err(DateTimePickerError::Invalid(DateTimeError::InvalidOffset(
            1_440
        )))
    );
}

#[test]
fn picker_replaces_date_and_time_only_inside_bounds() {
    let zone = TimeZonePolicy::fixed_offset(-300).unwrap();
    let initial_date = Date::try_new(2024, 6, 15).unwrap();
    let initial_time = TimeOfDay::try_new(9, 30, 45, 123).unwrap();
    let initial = DateTime::try_new(initial_date, initial_time, zone).unwrap();
    let minimum =
        DateTime::try_new(initial_date, TimeOfDay::try_new(9, 0, 0, 0).unwrap(), zone).unwrap();
    let maximum_date = Date::try_new(2024, 6, 20).unwrap();
    let maximum_time = TimeOfDay::try_new(18, 0, 0, 0).unwrap();
    let maximum = DateTime::try_new(maximum_date, maximum_time, zone).unwrap();
    let policy = DateTimePickerPolicy::try_new(zone, Some(minimum), Some(maximum)).unwrap();
    let mut picker = DateTimePicker::try_new(Some(initial), policy).unwrap();

    picker.open();
    let replacement_date = Date::try_new(2024, 6, 20).unwrap();
    picker.set_date(replacement_date).unwrap();
    assert_eq!(picker.draft().unwrap().date(), replacement_date);
    assert_eq!(picker.draft().unwrap().time(), initial_time);

    let replacement_time = TimeOfDay::try_new(18, 0, 0, 0).unwrap();
    picker.set_time(replacement_time).unwrap();
    assert_eq!(picker.draft().unwrap().date(), replacement_date);
    assert_eq!(picker.draft().unwrap().time(), replacement_time);

    let outside_date = Date::try_new(2024, 6, 21).unwrap();
    assert!(matches!(
        picker.set_date(outside_date),
        Err(DateTimePickerError::Invalid(DateTimeError::OutOfBounds(_)))
    ));
    assert_eq!(picker.draft().unwrap().date(), replacement_date);
    assert!(matches!(
        picker.set_time(TimeOfDay::try_new(18, 0, 0, 1).unwrap()),
        Err(DateTimePickerError::Invalid(DateTimeError::OutOfBounds(_)))
    ));
    assert_eq!(picker.draft().unwrap().time(), replacement_time);
}

#[test]
fn picker_navigates_by_day_week_month_and_year_while_preserving_time() {
    let zone = TimeZonePolicy::Utc;
    let date = Date::try_new(2024, 1, 15).unwrap();
    let time = TimeOfDay::try_new(13, 45, 12, 345).unwrap();
    let initial = DateTime::try_new(date, time, zone).unwrap();
    let mut picker =
        DateTimePicker::try_new(Some(initial), DateTimePickerPolicy::unbounded(zone)).unwrap();
    picker.open();

    assert!(picker.navigate(CalendarNavigation::NextDay).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2024, 1, 16).unwrap()
    );
    assert_eq!(picker.draft().unwrap().time(), time);
    assert!(picker.navigate(CalendarNavigation::PreviousDay).unwrap());
    assert_eq!(picker.draft().unwrap().date(), date);
    assert!(picker.navigate(CalendarNavigation::NextWeek).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2024, 1, 22).unwrap()
    );
    assert!(picker.navigate(CalendarNavigation::PreviousWeek).unwrap());
    assert_eq!(picker.draft().unwrap().date(), date);
    assert!(picker.navigate(CalendarNavigation::NextMonth).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2024, 2, 15).unwrap()
    );
    assert!(picker.navigate(CalendarNavigation::PreviousMonth).unwrap());
    assert_eq!(picker.draft().unwrap().date(), date);
    assert!(picker.navigate(CalendarNavigation::NextYear).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2025, 1, 15).unwrap()
    );
    assert!(picker.navigate(CalendarNavigation::PreviousYear).unwrap());
    assert_eq!(picker.draft().unwrap().date(), date);
    assert_eq!(picker.draft().unwrap().time(), time);
}

#[test]
fn navigation_rejects_out_of_bounds_and_supported_date_edges_without_mutation() {
    let zone = TimeZonePolicy::fixed_offset(90).unwrap();
    let minimum = DateTime::try_new(
        Date::try_new(2024, 1, 10).unwrap(),
        TimeOfDay::try_new(12, 0, 0, 0).unwrap(),
        zone,
    )
    .unwrap();
    let maximum = DateTime::try_new(
        Date::try_new(2024, 1, 20).unwrap(),
        TimeOfDay::try_new(12, 0, 0, 0).unwrap(),
        zone,
    )
    .unwrap();
    let policy = DateTimePickerPolicy::try_new(zone, Some(minimum), Some(maximum)).unwrap();

    let mut at_minimum = DateTimePicker::try_new(Some(minimum), policy).unwrap();
    at_minimum.open();
    for navigation in [
        CalendarNavigation::PreviousDay,
        CalendarNavigation::PreviousWeek,
        CalendarNavigation::PreviousMonth,
        CalendarNavigation::PreviousYear,
    ] {
        assert!(!at_minimum.navigate(navigation).unwrap());
        assert_eq!(at_minimum.draft(), Some(minimum));
    }

    let mut at_maximum = DateTimePicker::try_new(Some(maximum), policy).unwrap();
    at_maximum.open();
    for navigation in [
        CalendarNavigation::NextDay,
        CalendarNavigation::NextWeek,
        CalendarNavigation::NextMonth,
        CalendarNavigation::NextYear,
    ] {
        assert!(!at_maximum.navigate(navigation).unwrap());
        assert_eq!(at_maximum.draft(), Some(maximum));
    }

    let first = DateTime::try_new(
        Date::try_new(1, 1, 1).unwrap(),
        TimeOfDay::midnight(),
        TimeZonePolicy::Utc,
    )
    .unwrap();
    let mut at_first_supported_date = DateTimePicker::try_new(
        Some(first),
        DateTimePickerPolicy::unbounded(TimeZonePolicy::Utc),
    )
    .unwrap();
    at_first_supported_date.open();
    for navigation in [
        CalendarNavigation::PreviousDay,
        CalendarNavigation::PreviousWeek,
        CalendarNavigation::PreviousMonth,
        CalendarNavigation::PreviousYear,
    ] {
        assert!(!at_first_supported_date.navigate(navigation).unwrap());
    }
    assert_eq!(at_first_supported_date.draft(), Some(first));

    let last = DateTime::try_new(
        Date::try_new(9_999, 12, 31).unwrap(),
        TimeOfDay::midnight(),
        TimeZonePolicy::Utc,
    )
    .unwrap();
    let mut at_last_supported_date = DateTimePicker::try_new(
        Some(last),
        DateTimePickerPolicy::unbounded(TimeZonePolicy::Utc),
    )
    .unwrap();
    at_last_supported_date.open();
    for navigation in [
        CalendarNavigation::NextDay,
        CalendarNavigation::NextWeek,
        CalendarNavigation::NextMonth,
        CalendarNavigation::NextYear,
    ] {
        assert!(!at_last_supported_date.navigate(navigation).unwrap());
    }
    assert_eq!(at_last_supported_date.draft(), Some(last));
}

#[test]
fn picker_rejects_initial_values_outside_bounds_or_timezone_policy() {
    let zone = TimeZonePolicy::Utc;
    let date = Date::try_new(2024, 6, 15).unwrap();
    let minimum = DateTime::try_new(date, TimeOfDay::midnight(), zone).unwrap();
    let maximum =
        DateTime::try_new(date, TimeOfDay::try_new(23, 59, 59, 0).unwrap(), zone).unwrap();
    let policy = DateTimePickerPolicy::try_new(zone, Some(minimum), Some(maximum)).unwrap();
    let outside = DateTime::try_new(
        Date::try_new(2024, 6, 16).unwrap(),
        TimeOfDay::midnight(),
        zone,
    )
    .unwrap();
    let other_zone = TimeZonePolicy::fixed_offset(60).unwrap();
    let mismatched = DateTime::try_new(date, TimeOfDay::midnight(), other_zone).unwrap();

    assert_eq!(
        DateTimePicker::try_new(Some(outside), policy),
        Err(DateTimePickerError::Invalid(DateTimeError::OutOfBounds(
            outside
        )))
    );
    assert_eq!(
        DateTimePicker::try_new(Some(mismatched), policy),
        Err(DateTimePickerError::Invalid(
            DateTimeError::TimeZoneMismatch {
                expected: zone,
                actual: other_zone,
            }
        ))
    );
}

#[test]
fn picker_exposes_disabled_dates_and_rejects_them_without_mutating_the_draft() {
    let timezone = TimeZonePolicy::Utc;
    let initial_date = Date::try_new(2024, 6, 15).unwrap();
    let disabled_date = Date::try_new(2024, 6, 16).unwrap();
    let initial = DateTime::try_new(initial_date, TimeOfDay::midnight(), timezone).unwrap();
    let mut picker =
        DateTimePicker::try_new(Some(initial), DateTimePickerPolicy::unbounded(timezone))
            .unwrap()
            .with_disabled_dates([disabled_date]);

    assert!(picker.is_date_disabled(disabled_date));
    picker.open();
    assert_eq!(
        picker.set_date(disabled_date),
        Err(DateTimePickerError::DisabledDate(disabled_date))
    );
    assert_eq!(picker.draft(), Some(initial));
}

#[test]
fn picker_keyboard_navigation_skips_disabled_dates() {
    let timezone = TimeZonePolicy::Utc;
    let initial_date = Date::try_new(2024, 6, 15).unwrap();
    let initial = DateTime::try_new(initial_date, TimeOfDay::midnight(), timezone).unwrap();
    let mut picker =
        DateTimePicker::try_new(Some(initial), DateTimePickerPolicy::unbounded(timezone))
            .unwrap()
            .with_disabled_dates([
                Date::try_new(2024, 6, 16).unwrap(),
                Date::try_new(2024, 6, 17).unwrap(),
            ]);

    picker.open();
    assert!(picker.navigate(CalendarNavigation::NextDay).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2024, 6, 18).unwrap()
    );
}

#[test]
fn picker_confirm_and_cancel_are_transactional_and_require_an_open_session() {
    let zone = TimeZonePolicy::Utc;
    let initial = DateTime::try_new(
        Date::try_new(2024, 6, 15).unwrap(),
        TimeOfDay::try_new(9, 30, 0, 0).unwrap(),
        zone,
    )
    .unwrap();
    let changed_date = Date::try_new(2024, 6, 18).unwrap();
    let changed_time = TimeOfDay::try_new(16, 45, 0, 0).unwrap();
    let mut picker =
        DateTimePicker::try_new(Some(initial), DateTimePickerPolicy::unbounded(zone)).unwrap();

    assert_eq!(
        picker.set_date(changed_date),
        Err(DateTimePickerError::Closed)
    );
    assert_eq!(
        picker.set_time(changed_time),
        Err(DateTimePickerError::Closed)
    );
    assert_eq!(picker.confirm(), Err(DateTimePickerError::Closed));
    assert_eq!(
        picker.cancel(CancelReason::OutsideClick),
        Err(DateTimePickerError::Closed)
    );

    picker.open();
    picker.set_date(changed_date).unwrap();
    picker.set_time(changed_time).unwrap();
    let draft = DateTime::try_new(changed_date, changed_time, zone).unwrap();
    assert_eq!(picker.draft(), Some(draft));
    assert_eq!(picker.value(), Some(initial));
    assert_eq!(
        picker.cancel(CancelReason::Escape).unwrap(),
        PickerOutcome::Cancelled {
            reason: CancelReason::Escape,
            value: Some(initial)
        }
    );
    assert!(!picker.is_open());
    assert_eq!(picker.value(), Some(initial));
    assert_eq!(picker.draft(), Some(initial));

    picker.open();
    assert_eq!(picker.draft(), Some(initial));
    picker.set_date(changed_date).unwrap();
    assert_eq!(
        picker.confirm().unwrap(),
        PickerOutcome::Confirmed(Some(
            DateTime::try_new(changed_date, initial.time(), zone).unwrap()
        ))
    );
    assert!(!picker.is_open());
    assert_eq!(
        picker.value(),
        Some(DateTime::try_new(changed_date, initial.time(), zone).unwrap())
    );
}

#[test]
fn navigation_clamps_month_and_year_changes_to_valid_calendar_dates() {
    let zone = TimeZonePolicy::Utc;
    let time = TimeOfDay::try_new(8, 15, 0, 0).unwrap();
    let march_end = DateTime::try_new(Date::try_new(2024, 3, 31).unwrap(), time, zone).unwrap();
    let mut month_picker =
        DateTimePicker::try_new(Some(march_end), DateTimePickerPolicy::unbounded(zone)).unwrap();
    month_picker.open();
    assert!(
        month_picker
            .navigate(CalendarNavigation::PreviousMonth)
            .unwrap()
    );
    assert_eq!(
        month_picker.draft().unwrap().date(),
        Date::try_new(2024, 2, 29).unwrap()
    );

    let leap_day = DateTime::try_new(Date::try_new(2024, 2, 29).unwrap(), time, zone).unwrap();
    let mut year_picker =
        DateTimePicker::try_new(Some(leap_day), DateTimePickerPolicy::unbounded(zone)).unwrap();
    year_picker.open();
    assert!(year_picker.navigate(CalendarNavigation::NextYear).unwrap());
    assert_eq!(
        year_picker.draft().unwrap().date(),
        Date::try_new(2025, 2, 28).unwrap()
    );
}

#[test]
fn bounded_month_navigation_clamps_to_a_valid_date_time_in_the_target_month() {
    let zone = TimeZonePolicy::Utc;
    let minimum = DateTime::try_new(
        Date::try_new(2024, 1, 1).unwrap(),
        TimeOfDay::midnight(),
        zone,
    )
    .unwrap();
    let maximum = DateTime::try_new(
        Date::try_new(2024, 2, 10).unwrap(),
        TimeOfDay::try_new(23, 59, 59, 0).unwrap(),
        zone,
    )
    .unwrap();
    let initial = DateTime::try_new(
        Date::try_new(2024, 1, 31).unwrap(),
        TimeOfDay::try_new(12, 0, 0, 0).unwrap(),
        zone,
    )
    .unwrap();
    let policy = DateTimePickerPolicy::try_new(zone, Some(minimum), Some(maximum)).unwrap();
    let mut picker = DateTimePicker::try_new(Some(initial), policy).unwrap();

    picker.open();
    assert!(picker.navigate(CalendarNavigation::NextMonth).unwrap());
    assert_eq!(
        picker.draft().unwrap().date(),
        Date::try_new(2024, 2, 10).unwrap()
    );
}
