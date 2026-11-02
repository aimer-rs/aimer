use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_events::element::{Modifiers, NamedKey};
use aimer_range::Slider;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, Widget};

use super::{
    CALENDAR_HEADER, CALENDAR_WEEKDAYS, Calendar, CalendarNavigation, ColorChannel, Date,
    DateSelection, DateTime, DateTimePicker, DateTimePickerPolicy, PICKER_FIELD_HEIGHT,
    PICKER_FOOTER_HEIGHT, TimeOfDay, TimeZonePolicy,
};
use crate::Rgba;
use crate::TimePicker;

pub(super) fn color_sliders(
    runtime: &Rc<super::color::ColorPickerRuntime>,
    width: f32,
    ctx: &BuildContext,
) -> Vec<AnyElement> {
    let picker = runtime.picker.borrow();
    let draft = picker.draft();
    let channels = [
        (ColorChannel::Hue, draft.hue(), 360_u16),
        (
            ColorChannel::Saturation,
            u16::from(draft.saturation()),
            100_u16,
        ),
        (ColorChannel::Value, u16::from(draft.value()), 100_u16),
        (ColorChannel::Alpha, u16::from(draft.alpha()), 100_u16),
    ];
    let open = picker.is_open();
    let alpha_enabled = picker.alpha_enabled();
    channels
        .into_iter()
        .map(|(channel, value, maximum)| {
            let runtime = Rc::clone(runtime);
            Slider::new()
                .range(0_u16..maximum)
                .step(1_u16)
                .value(value)
                .width((width - 48.0).max(0.0))
                .height(super::COLOR_CHANNEL_ROW_HEIGHT)
                .disabled(!open || (channel == ColorChannel::Alpha && !alpha_enabled))
                .on_change(move |value| {
                    runtime.channel.set(channel);
                    let _ = runtime
                        .picker
                        .borrow_mut()
                        .set_channel_value(channel, value);
                })
                .to_element(ctx)
        })
        .collect()
}

pub(super) fn default_datetime_picker() -> DateTimePicker {
    let value = DateTime::try_new(default_date(), TimeOfDay::midnight(), TimeZonePolicy::Utc)
        .expect("default date-time is valid");
    DateTimePicker::try_new(
        Some(value),
        DateTimePickerPolicy::unbounded(TimeZonePolicy::Utc),
    )
    .expect("default date-time policy accepts its value")
}

pub(super) fn default_time_picker() -> TimePicker {
    TimePicker::new(Some(TimeOfDay::midnight()))
}

pub(super) fn default_date() -> Date {
    Date::try_new(1970, 1, 1).expect("the epoch is a valid date")
}

pub(super) fn popup_height(total_height: f32) -> f32 {
    (total_height - PICKER_FIELD_HEIGHT).max(PICKER_FOOTER_HEIGHT)
}

pub(super) fn calendar_navigation(
    key: &NamedKey,
    modifiers: &Modifiers,
) -> Option<CalendarNavigation> {
    Some(match key {
        NamedKey::ArrowLeft => CalendarNavigation::PreviousDay,
        NamedKey::ArrowRight => CalendarNavigation::NextDay,
        NamedKey::ArrowUp => CalendarNavigation::PreviousWeek,
        NamedKey::ArrowDown => CalendarNavigation::NextWeek,
        NamedKey::PageUp => CalendarNavigation::PreviousMonth,
        NamedKey::PageDown => CalendarNavigation::NextMonth,
        NamedKey::Home if modifiers.ctrl => CalendarNavigation::PreviousYear,
        NamedKey::End if modifiers.ctrl => CalendarNavigation::NextYear,
        _ => return None,
    })
}

pub(super) fn calendar_date_at(
    calendar: &Calendar,
    origin: Vec2d,
    width: f32,
    height: f32,
    x: f32,
    y: f32,
) -> Option<Date> {
    let local_x = x - origin.x;
    let local_y = y - origin.y;
    let cells_top = CALENDAR_HEADER + CALENDAR_WEEKDAYS;
    let cell_width = width / 7.0;
    let cell_height = (height - cells_top).max(0.0) / 6.0;
    if cell_width <= 0.0
        || cell_height <= 0.0
        || local_x < 0.0
        || local_x >= width
        || local_y < cells_top
        || local_y >= height
    {
        return None;
    }
    let column = (local_x / cell_width).floor() as usize;
    let row = ((local_y - cells_top) / cell_height).floor() as usize;
    calendar
        .cells()
        .get(row * 7 + column)
        .map(|cell| cell.date())
}

pub(super) fn selection_label(selection: DateSelection) -> String {
    match selection {
        DateSelection::Single(Some(date)) => format_date(date),
        DateSelection::Single(None) => "Select a date".to_owned(),
        DateSelection::Range { start, end } => match (start, end) {
            (Some(start), Some(end)) => format!("{} – {}", format_date(start), format_date(end)),
            (Some(start), None) => format!("{} – …", format_date(start)),
            _ => "Select a date range".to_owned(),
        },
    }
}

fn format_date(date: Date) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

pub(super) fn format_datetime(value: DateTime) -> String {
    let time = value.time();
    format!(
        "{} {:02}:{:02}:{:02} ({:+}m)",
        format_date(value.date()),
        time.hour(),
        time.minute(),
        time.second(),
        value.timezone().offset_minutes(),
    )
}

pub(super) fn format_time(time: TimeOfDay, use_24_hours: bool) -> String {
    if use_24_hours {
        format!(
            "{:02}:{:02}:{:02}",
            time.hour(),
            time.minute(),
            time.second()
        )
    } else {
        format!(
            "{:02}:{:02}:{:02} {}",
            hour12(time.hour()),
            time.minute(),
            time.second(),
            if time.hour() >= 12 { "PM" } else { "AM" },
        )
    }
}

#[inline]
fn hour12(hour: u8) -> u8 {
    if hour % 12 == 0 { 12 } else { hour % 12 }
}

pub(super) fn format_rgba(value: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        value.red(),
        value.green(),
        value.blue(),
        value.alpha()
    )
}
