use aimer_picker::{
    Calendar, CalendarError, Date, DateBounds, DateError, DateSelection, DateSelectionMode, Month,
    Weekday,
};

fn date(year: i32, month: u8, day: u8) -> Date {
    Date::try_new(year, month, day).unwrap()
}

#[test]
fn individual_disabled_dates_are_visible_but_not_selectable() {
    let holiday = date(2024, 5, 20);
    let mut calendar = Calendar::try_new(
        date(2024, 5, 15),
        DateBounds::unbounded(),
        DateSelectionMode::Single,
    )
    .unwrap()
    .with_disabled_dates([holiday]);

    let holiday_cell = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == holiday)
        .unwrap();
    assert!(holiday_cell.is_disabled());
    assert_eq!(
        calendar.select(holiday),
        Err(CalendarError::DisabledDate(holiday))
    );
    assert_eq!(calendar.selection(), DateSelection::Single(None));
}

#[test]
fn calendar_keyboard_focus_skips_disabled_dates() {
    let focused = date(2024, 5, 15);
    let disabled_one = date(2024, 5, 16);
    let disabled_two = date(2024, 5, 17);
    let mut calendar =
        Calendar::try_new(focused, DateBounds::unbounded(), DateSelectionMode::Single)
            .unwrap()
            .with_disabled_dates([disabled_one, disabled_two]);

    assert_eq!(
        calendar.focus(disabled_one),
        Err(CalendarError::DisabledDate(disabled_one))
    );
    assert_eq!(calendar.focused_date(), focused);
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextDay));
    assert_eq!(calendar.focused_date(), date(2024, 5, 18));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousDay));
    assert_eq!(calendar.focused_date(), focused);
}

#[test]
fn dates_validate_gregorian_edges_and_arithmetic_boundaries() {
    let first = date(1, 1, 1);
    let last = date(9_999, 12, 31);

    assert_eq!(first.weekday(), Weekday::Monday);
    assert_eq!(date(2024, 2, 29).weekday(), Weekday::Thursday);
    assert_eq!(date(2_000, 2, 29).day(), 29);
    assert!(Date::try_new(1_900, 2, 29).is_err());
    assert!(Date::try_new(2_023, 2, 29).is_err());
    assert!(matches!(
        Date::try_new(0, 1, 1),
        Err(DateError::InvalidYear { year: 0 })
    ));
    assert!(matches!(
        Date::try_new(10_000, 1, 1),
        Err(DateError::InvalidYear { year: 10_000 })
    ));
    assert!(matches!(
        Date::try_new(2024, 0, 1),
        Err(DateError::InvalidMonth { month: 0 })
    ));
    assert!(matches!(
        Date::try_new(2024, 13, 1),
        Err(DateError::InvalidMonth { month: 13 })
    ));
    assert!(matches!(
        Date::try_new(2024, 2, 0),
        Err(DateError::InvalidDay { .. })
    ));
    assert!(matches!(
        Date::try_new(2024, 4, 31),
        Err(DateError::InvalidDay { .. })
    ));

    assert_eq!(date(2024, 1, 31).add_months(1), Some(date(2024, 2, 29)));
    assert_eq!(date(2023, 1, 31).add_months(1), Some(date(2023, 2, 28)));
    assert_eq!(date(2024, 2, 29).add_days(1), Some(date(2024, 3, 1)));
    assert!(Date::is_leap_year(2_000));
    assert!(!Date::is_leap_year(1_900));
    assert_eq!(Date::days_in_month(2024, 2), Some(29));
    assert_eq!(Date::days_in_month(2023, 2), Some(28));
    assert_eq!(Date::days_in_month(2024, 13), None);
    assert_eq!(first.add_days(-1), None);
    assert_eq!(last.add_days(1), None);
    assert_eq!(first.month_key(), Month::try_new(1, 1).unwrap());
    assert_eq!(Month::try_new(1, 1).unwrap().add_months(-1), None);
    assert_eq!(Month::try_new(9_999, 12).unwrap().add_months(1), None);
}

#[test]
fn bounded_month_navigation_reaches_partial_target_months() {
    let minimum = date(2024, 5, 20);
    let maximum = date(2024, 6, 5);
    let bounds = DateBounds::new(Some(minimum), Some(maximum)).unwrap();
    let mut calendar = Calendar::try_new(minimum, bounds, DateSelectionMode::Single).unwrap();

    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextMonth));
    assert_eq!(calendar.focused_date(), maximum);
    assert_eq!(calendar.visible_month(), maximum.month_key());

    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousMonth));
    assert_eq!(calendar.focused_date(), minimum);
    assert_eq!(calendar.visible_month(), minimum.month_key());
}

#[test]
fn bounded_year_navigation_reaches_partial_target_year_months() {
    let minimum = date(2024, 2, 20);
    let maximum = date(2025, 2, 5);
    let bounds = DateBounds::new(Some(minimum), Some(maximum)).unwrap();
    let mut calendar = Calendar::try_new(minimum, bounds, DateSelectionMode::Single).unwrap();

    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextYear));
    assert_eq!(calendar.focused_date(), maximum);
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousYear));
    assert_eq!(calendar.focused_date(), minimum);
}

#[test]
fn date_bounds_are_inclusive_and_rejected_operations_preserve_state() {
    let minimum = date(2024, 5, 10);
    let maximum = date(2024, 6, 20);
    let bounds = DateBounds::new(Some(minimum), Some(maximum)).unwrap();
    assert!(bounds.contains(minimum));
    assert!(bounds.contains(maximum));
    assert!(!bounds.contains(date(2024, 5, 9)));
    assert!(!bounds.contains(date(2024, 6, 21)));
    assert!(bounds.validate(minimum).is_ok());
    assert!(matches!(
        bounds.validate(date(2024, 5, 9)),
        Err(DateError::OutOfBounds { .. })
    ));
    assert_eq!(bounds.clamp(date(2024, 5, 1)), minimum);
    assert_eq!(bounds.clamp(date(2024, 7, 1)), maximum);
    assert!(matches!(
        DateBounds::new(Some(maximum), Some(minimum)),
        Err(DateError::InvalidBounds { .. })
    ));

    let mut calendar =
        Calendar::try_new(date(2024, 5, 15), bounds, DateSelectionMode::Single).unwrap();
    let minimum_cell = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == minimum)
        .unwrap();
    let before_minimum_cell = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == date(2024, 5, 9))
        .unwrap();
    assert!(!minimum_cell.is_disabled());
    assert!(before_minimum_cell.is_disabled());

    assert_eq!(
        calendar.focus(date(2024, 5, 9)),
        Err(CalendarError::OutOfBounds(date(2024, 5, 9)))
    );
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));
    assert_eq!(
        calendar.select(date(2024, 5, 9)),
        Err(CalendarError::OutOfBounds(date(2024, 5, 9)))
    );
    assert_eq!(calendar.selection(), DateSelection::Single(None));
    assert!(!calendar.navigate(aimer_picker::CalendarNavigation::PreviousWeek));
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));

    calendar.focus(minimum).unwrap();
    assert!(!calendar.navigate(aimer_picker::CalendarNavigation::PreviousDay));
    assert_eq!(calendar.focused_date(), minimum);
    calendar.focus(maximum).unwrap();
    assert!(!calendar.navigate(aimer_picker::CalendarNavigation::NextDay));
    assert_eq!(calendar.focused_date(), maximum);

    calendar.navigate(aimer_picker::CalendarNavigation::NextMonth);
    let maximum_cell = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == maximum)
        .unwrap();
    let after_maximum_cell = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == date(2024, 6, 21))
        .unwrap();
    assert!(!maximum_cell.is_disabled());
    assert!(after_maximum_cell.is_disabled());
}

#[test]
fn calendar_supports_day_week_month_and_year_navigation() {
    let mut calendar = Calendar::new(date(2024, 5, 15));

    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousDay));
    assert_eq!(calendar.focused_date(), date(2024, 5, 14));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextDay));
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousWeek));
    assert_eq!(calendar.focused_date(), date(2024, 5, 8));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextWeek));
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousMonth));
    assert_eq!(calendar.focused_date(), date(2024, 4, 15));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextMonth));
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::PreviousYear));
    assert_eq!(calendar.focused_date(), date(2023, 5, 15));
    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextYear));
    assert_eq!(calendar.focused_date(), date(2024, 5, 15));

    let mut leap_day = Calendar::new(date(2024, 1, 31));
    assert!(leap_day.navigate(aimer_picker::CalendarNavigation::NextMonth));
    assert_eq!(leap_day.focused_date(), date(2024, 2, 29));
    assert!(leap_day.navigate(aimer_picker::CalendarNavigation::NextYear));
    assert_eq!(leap_day.focused_date(), date(2025, 2, 28));

    let first = date(1, 1, 1);
    let mut at_first = Calendar::new(first);
    assert!(!at_first.navigate(aimer_picker::CalendarNavigation::PreviousDay));
    assert!(!at_first.navigate(aimer_picker::CalendarNavigation::PreviousWeek));
    assert!(!at_first.navigate(aimer_picker::CalendarNavigation::PreviousMonth));
    assert!(!at_first.navigate(aimer_picker::CalendarNavigation::PreviousYear));
    assert_eq!(at_first.focused_date(), first);

    let last = date(9_999, 12, 31);
    let mut at_last = Calendar::new(last);
    assert!(!at_last.navigate(aimer_picker::CalendarNavigation::NextDay));
    assert!(!at_last.navigate(aimer_picker::CalendarNavigation::NextWeek));
    assert!(!at_last.navigate(aimer_picker::CalendarNavigation::NextMonth));
    assert!(!at_last.navigate(aimer_picker::CalendarNavigation::NextYear));
    assert_eq!(at_last.focused_date(), last);
}

#[test]
fn calendar_cell_identity_is_the_same_date_across_month_changes() {
    let mut calendar = Calendar::new(date(2024, 5, 15));
    let carried_date = date(2024, 5, 30);
    let before = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == carried_date)
        .unwrap();

    assert!(calendar.navigate(aimer_picker::CalendarNavigation::NextMonth));
    let after = calendar
        .cells()
        .into_iter()
        .find(|cell| cell.date() == carried_date)
        .unwrap();
    assert_eq!(before.id(), after.id());
    assert_eq!(
        before.id(),
        aimer_picker::CalendarCellId::from_date(carried_date)
    );
    assert_eq!(after.id().date(), after.date());

    let cells = calendar.cells();
    for (index, cell) in cells.iter().enumerate() {
        assert_eq!(
            cell.id(),
            aimer_picker::CalendarCellId::from_date(cell.date())
        );
        assert!(
            cells
                .iter()
                .skip(index + 1)
                .all(|other| other.id() != cell.id())
        );
    }
}

#[test]
fn range_selection_is_inclusive_and_swaps_reversed_endpoints() {
    let start = date(2024, 5, 20);
    let end = date(2024, 5, 25);
    let mode = DateSelectionMode::Range(aimer_picker::DateRangePolicy::inclusive());
    let mut calendar = Calendar::try_new(start, DateBounds::unbounded(), mode).unwrap();

    assert_eq!(
        calendar.select(end).unwrap(),
        DateSelection::Range {
            start: Some(end),
            end: None
        }
    );
    assert_eq!(
        calendar.select(start).unwrap(),
        DateSelection::Range {
            start: Some(start),
            end: Some(end)
        }
    );
    assert!(calendar.selection().is_complete());
    assert!(calendar.selection().contains(start));
    assert!(calendar.selection().contains(date(2024, 5, 22)));
    assert!(calendar.selection().contains(end));
    assert!(!calendar.selection().contains(date(2024, 5, 19)));
    assert!(!calendar.selection().contains(date(2024, 5, 26)));

    let selected_cells = calendar
        .cells()
        .into_iter()
        .filter(|cell| cell.is_selected())
        .map(|cell| cell.date())
        .collect::<Vec<_>>();
    assert_eq!(
        selected_cells,
        vec![
            date(2024, 5, 20),
            date(2024, 5, 21),
            date(2024, 5, 22),
            date(2024, 5, 23),
            date(2024, 5, 24),
            date(2024, 5, 25),
        ]
    );
}

#[test]
fn range_selection_identifies_endpoints_and_interior_dates() {
    let start = date(2024, 5, 10);
    let middle = date(2024, 5, 12);
    let end = date(2024, 5, 15);
    let selection = DateSelection::Range {
        start: Some(start),
        end: Some(end),
    };

    assert!(selection.is_range_endpoint(start));
    assert!(selection.is_range_endpoint(end));
    assert!(!selection.is_range_endpoint(middle));
    assert!(selection.is_range_interior(middle));
    assert!(!selection.is_range_interior(start));
    assert!(!selection.is_range_interior(end));
}

#[test]
fn range_policy_handles_disallowed_same_day_and_restart_order() {
    let first = date(2024, 5, 20);
    let earlier = date(2024, 5, 15);
    let later = date(2024, 5, 25);
    let no_same_day =
        aimer_picker::DateRangePolicy::new(false, aimer_picker::RangeOrderPolicy::Restart);
    let mut calendar = Calendar::try_new(
        first,
        DateBounds::unbounded(),
        DateSelectionMode::Range(no_same_day),
    )
    .unwrap();

    calendar.select(first).unwrap();
    assert_eq!(
        calendar.select(first),
        Err(CalendarError::SameDayRangeNotAllowed(first))
    );
    assert_eq!(
        calendar.selection(),
        DateSelection::Range {
            start: Some(first),
            end: None
        }
    );
    assert_eq!(
        calendar.select(earlier).unwrap(),
        DateSelection::Range {
            start: Some(earlier),
            end: None
        }
    );
    assert_eq!(
        calendar.select(later).unwrap(),
        DateSelection::Range {
            start: Some(earlier),
            end: Some(later)
        }
    );
}

#[test]
fn disabled_dates_cannot_complete_a_range() {
    let start = date(2024, 5, 15);
    let disabled = date(2024, 5, 20);
    let mut calendar = Calendar::try_new(
        start,
        DateBounds::unbounded(),
        DateSelectionMode::Range(aimer_picker::DateRangePolicy::inclusive()),
    )
    .unwrap()
    .with_disabled_dates([disabled]);

    calendar.select(start).unwrap();
    assert_eq!(
        calendar.select(disabled),
        Err(CalendarError::DisabledDate(disabled))
    );
    assert_eq!(
        calendar.selection(),
        DateSelection::Range {
            start: Some(start),
            end: None
        }
    );
}

#[test]
fn single_selection_replaces_the_previous_date_and_can_be_cleared() {
    let first = date(2024, 5, 15);
    let second = date(2024, 5, 22);
    let mut calendar = Calendar::new(first);

    assert_eq!(
        calendar.select(first).unwrap(),
        DateSelection::Single(Some(first))
    );
    assert_eq!(
        calendar.select(second).unwrap(),
        DateSelection::Single(Some(second))
    );
    assert_eq!(calendar.focused_date(), second);
    assert!(
        calendar
            .cells()
            .into_iter()
            .find(|cell| cell.date() == second)
            .unwrap()
            .is_selected()
    );
    calendar.clear_selection();
    assert_eq!(calendar.selection(), DateSelection::Single(None));
}
