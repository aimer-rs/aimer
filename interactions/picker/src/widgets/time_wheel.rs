use aimer_events::element::ScrollDeltaKind;
use aimer_widget::PointerKey;

use crate::{TIME_WHEEL_ROW_HEIGHT, TIME_WHEEL_ROWS, TimeOfDay};

/// The pointer state retained while a wheel column is being dragged.
#[derive(Clone, Copy)]
pub(super) struct TimeWheelDrag {
    pub(super) pointer: PointerKey,
    pub(super) column: usize,
    pub(super) start_y: f32,
    pub(super) start_time: TimeOfDay,
}

#[inline]
pub(super) fn column_count(use_24_hours: bool) -> usize {
    if use_24_hours { 3 } else { 4 }
}

#[inline]
pub(super) fn column_at(x: f32, width: f32, use_24_hours: bool) -> Option<usize> {
    let count = column_count(use_24_hours);
    if width <= 0.0 || x < 0.0 || x >= width {
        return None;
    }
    let column = (x / (width / count as f32)).floor() as usize;
    (column < count).then_some(column)
}

#[inline]
pub(super) fn row_at(y: f32, height: f32, header_height: f32) -> Option<usize> {
    let top = header_height + wheel_content_top();
    if y < top || y >= height - crate::PICKER_FOOTER_HEIGHT {
        return None;
    }
    let row = ((y - top) / TIME_WHEEL_ROW_HEIGHT).floor() as usize;
    (row < TIME_WHEEL_ROWS).then_some(row)
}

#[inline]
pub(super) fn wheel_content_top() -> f32 {
    // Keep a small breathing space between a segmented header and the first
    // visible value. The same inset is used by paint and hit testing.
    8.0
}

#[inline]
pub(super) fn selected_row() -> usize {
    TIME_WHEEL_ROWS / 2
}

#[inline]
pub(super) fn time_from_row(
    time: TimeOfDay,
    column: usize,
    row: usize,
    use_24_hours: bool,
) -> Option<TimeOfDay> {
    let offset = if !use_24_hours && column == 3 {
        // The period column intentionally renders one current value instead
        // of repeating AM/PM on every visible row; activating that value
        // toggles the period.
        if row != selected_row() {
            return None;
        }
        1
    } else {
        row as i32 - selected_row() as i32
    };
    time_from_offset(time, column, offset, use_24_hours)
}

#[inline]
pub(super) fn time_from_drag(
    start_time: TimeOfDay,
    column: usize,
    start_y: f32,
    current_y: f32,
    use_24_hours: bool,
) -> Option<TimeOfDay> {
    let offset = ((start_y - current_y) / TIME_WHEEL_ROW_HEIGHT).round() as i32;
    time_from_offset(start_time, column, offset, use_24_hours)
}

#[inline]
pub(super) fn time_from_scroll_steps(
    time: TimeOfDay,
    column: usize,
    steps: i32,
    use_24_hours: bool,
) -> Option<TimeOfDay> {
    time_from_offset(time, column, steps, use_24_hours)
}

/// Converts a platform scroll frame to whole wheel steps, retaining a pixel
/// remainder for smooth trackpad input.
pub(super) fn scroll_steps(
    delta_y: f32,
    kind: ScrollDeltaKind,
    remainder: &std::cell::Cell<f32>,
) -> i32 {
    if !delta_y.is_finite() || delta_y == 0.0 {
        return 0;
    }
    if kind == ScrollDeltaKind::Line {
        return if delta_y < 0.0 { 1 } else { -1 };
    }
    let accumulated = remainder.get() - delta_y;
    let steps = (accumulated / TIME_WHEEL_ROW_HEIGHT).trunc() as i32;
    remainder.set(accumulated - steps as f32 * TIME_WHEEL_ROW_HEIGHT);
    steps
}

fn time_from_offset(
    time: TimeOfDay,
    column: usize,
    offset: i32,
    use_24_hours: bool,
) -> Option<TimeOfDay> {
    let hour = if use_24_hours {
        match column {
            0 => wrap(time.hour(), 24, offset),
            _ => time.hour(),
        }
    } else {
        match column {
            0 => hour_12_in_period(time.hour(), offset),
            3 => period_hour(time.hour(), offset),
            _ => time.hour(),
        }
    };
    let minute = if column == 1 {
        wrap(time.minute(), 60, offset)
    } else {
        time.minute()
    };
    let second = if column == 2 {
        wrap(time.second(), 60, offset)
    } else {
        time.second()
    };
    TimeOfDay::try_new(hour, minute, second, time.nanosecond()).ok()
}

#[inline]
fn wrap(value: u8, limit: i32, offset: i32) -> u8 {
    (i32::from(value) + offset).rem_euclid(limit) as u8
}

#[inline]
fn hour_12_in_period(hour: u8, offset: i32) -> u8 {
    let current = hour_12_value(hour);
    let selected = (i32::from(current - 1) + offset).rem_euclid(12) as u8 + 1;
    if hour >= 12 {
        if selected == 12 { 12 } else { selected + 12 }
    } else if selected == 12 {
        0
    } else {
        selected
    }
}

#[inline]
fn period_hour(hour: u8, offset: i32) -> u8 {
    let selected = hour_12_value(hour);
    let is_pm = if offset.rem_euclid(2) == 0 {
        hour >= 12
    } else {
        hour < 12
    };
    if is_pm {
        if selected == 12 { 12 } else { selected + 12 }
    } else if selected == 12 {
        0
    } else {
        selected
    }
}

#[inline]
fn hour_12_value(hour: u8) -> u8 {
    if hour % 12 == 0 { 12 } else { hour % 12 }
}
