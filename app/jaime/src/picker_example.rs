//! A live, interactive showcase for Aimer's calendar, date, time, and color pickers.
//!
//! Each control is the real retained `StatefulWidget` adapter from
//! `aimer_picker`; the page itself does not own or mirror picker state.

use aimer::macros::widget;
use aimer::picker::{
    Calendar, CalendarView, ColorPicker, ColorPickerView, Date, DateBounds, DatePicker,
    DatePickerView, DateRangePolicy, DateSelectionMode, DateTime, DateTimePicker,
    DateTimePickerPolicy, DateTimePickerView, Hsva, TimeOfDay, TimePicker, TimePickerView,
    TimeZonePolicy,
};
use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextStyle, Theme, ThemeData,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Column, Container, Dimension, ScrollAxis, Scrollable,
    StatelessWidget, Text, Widget,
};

use crate::theme;

/// Builds the picker showcase without starting an application.
pub fn picker_example() -> impl Widget {
    PickerExample::new()
}

/// Starts the picker showcase as a standalone Jaime application.
pub fn start_picker_example() {
    AimerApp::start(theme::provide(picker_example()));
}

/// Demonstrates calendar navigation, transactional date/date-time editing,
/// scrollable 12/24-hour time wheels, explicit timezone and bounds policy,
/// and keyboard-accessible color editing.
#[widget(Stateless)]
pub struct PickerExample {}

impl PickerExample {
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for PickerExample {
    fn default() -> Self {
        Self::new()
    }
}

impl StatelessWidget for PickerExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let focused = example_date(2024, 5, 15);

        let calendar = CalendarView::new()
            .calendar(calendar_model())
            .width(320.0)
            .height(300.0);

        let date_picker = DatePickerView::new()
            .picker(date_picker_model())
            .width(320.0)
            .height(360.0);

        let date_time_picker = DateTimePickerView::new()
            .picker(date_time_model(Some(example_datetime(focused))))
            .width(320.0)
            .height(360.0)
            .use_24_hours(false);

        let time_picker = TimePickerView::new()
            .picker(TimePicker::new(Some(
                TimeOfDay::try_new(13, 30, 0, 0).expect("example time is valid"),
            )))
            .width(320.0)
            .height(300.0)
            .use_24_hours(true);

        let color_picker = ColorPickerView::new()
            .picker(ColorPicker::new(
                Hsva::try_new(210, 80, 90, 100).expect("example color is valid"),
                true,
            ))
            .width(280.0)
            .height(260.0);

        Container::new()
            .width(Dimension::Percent(100.0))
            .height(Dimension::Percent(100.0))
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Scrollable::new()
                    .axis(ScrollAxis::Vertical)
                    .vertical_scroll_bar(None)
                    .child(
                        Column::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(16)))
                            .children([
                        Text::new("Calendar, date/time, and color pickers")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Text::new(
                            "All five controls are retained widgets. Scroll or drag a time \
                             wheel, use the keyboard to \
                             navigate, confirm with Enter, or cancel drafts with Escape.",
                        )
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                        picker_card(
                            "Calendar range",
                            "Select two dates. The endpoints use full opacity and the days between them use 50%."
                                .to_owned(),
                            calendar.boxed(),
                            app_theme,
                        ),
                        picker_card(
                            "Date picker",
                            "Click the field or press Enter. The calendar opens in an anchored overlay; Confirm commits it."
                                .to_owned(),
                            date_picker.boxed(),
                            app_theme,
                        ),
                        picker_card(
                            "Date-time picker",
                            "Choose Date or Time from the segmented header. This example uses a 12-hour wheel; scroll or drag each column, then choose Done."
                                .to_owned(),
                            date_time_picker.boxed(),
                            app_theme,
                        ),
                        picker_card(
                            "Standalone time picker",
                            "A time-only editor using a 24-hour scroll wheel. Set .use_24_hours(false) to show one AM/PM selector."
                                .to_owned(),
                            time_picker.boxed(),
                            app_theme,
                        ),
                        picker_card(
                            "Color picker",
                            "Each HSVA channel is a Slider. Drag a channel, use arrow keys for fine changes, then Confirm or Escape."
                                .to_owned(),
                            color_picker.boxed(),
                            app_theme,
                        ),
                        Text::new("Every control retains its own draft, focus, and confirmed value.")
                            .text_style(
                                TextStyle::new()
                                    .font_size(13)
                                    .color(theme::muted_text(&app_theme)),
                            )
                            .boxed(),
                            ]),
                    ),
            )
    }
}

fn example_date(year: i32, month: u8, day: u8) -> Date {
    Date::try_new(year, month, day).expect("example date is valid")
}

fn example_timezone() -> TimeZonePolicy {
    TimeZonePolicy::fixed_offset(330).expect("example timezone is valid")
}

fn example_datetime(date: Date) -> DateTime {
    DateTime::try_new(
        date,
        TimeOfDay::try_new(9, 30, 0, 0).expect("example time is valid"),
        example_timezone(),
    )
    .expect("example date-time is valid")
}

fn example_bounds() -> DateBounds {
    DateBounds::new(
        Some(example_date(2024, 5, 1)),
        Some(example_date(2024, 6, 30)),
    )
    .expect("example bounds are ordered")
}

fn calendar_model() -> Calendar {
    let start = example_date(2024, 5, 10);
    let end = example_date(2024, 5, 15);
    let mut calendar = Calendar::try_new(
        example_date(2024, 5, 15),
        example_bounds(),
        DateSelectionMode::Range(DateRangePolicy::inclusive()),
    )
    .expect("example calendar policy is valid");

    calendar
        .select(start)
        .expect("example range start is valid");
    calendar
        .select(end)
        .expect("example range end is valid");
    calendar
}

fn date_picker_model() -> DatePicker {
    DatePicker::try_new(Some(example_date(2024, 5, 15)), example_bounds())
        .expect("example date-picker policy is valid")
}

fn date_time_model(value: Option<DateTime>) -> DateTimePicker {
    let timezone = example_timezone();
    let minimum = DateTime::try_new(example_date(2024, 5, 1), TimeOfDay::midnight(), timezone)
        .expect("example minimum is valid");
    let maximum = DateTime::try_new(
        example_date(2024, 6, 30),
        TimeOfDay::try_new(23, 59, 59, 0).expect("example maximum time is valid"),
        timezone,
    )
    .expect("example maximum is valid");
    let policy = DateTimePickerPolicy::try_new(timezone, Some(minimum), Some(maximum))
        .expect("example date-time policy is valid");
    DateTimePicker::try_new(value, policy).expect("example date-time value is in bounds")
}

fn picker_card(
    title: &'static str,
    status: String,
    control: AnyWidget,
    app_theme: ThemeData,
) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(12),
        )
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                .children([
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                    Text::new(status)
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .color(theme::muted_text(&app_theme)),
                        )
                        .boxed(),
                    control,
                ]),
        )
        .boxed()
}
