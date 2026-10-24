//! Renderer-neutral fallback painting for picker surfaces.

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_style::{ThemeTokens, apply_state_layer};
use aimer_widget::base::{BuildContext, Color};

use super::{Calendar, ColorChannel, ColorPicker, TimeOfDay};

trait PickerPaintCanvas {
    fn scale(&self) -> f32;
    fn fill_color_rect(
        &self,
        pos: Vec2d,
        size: ResolvedSize,
        color: Color,
        border_radius: [f32; 4],
    );
    fn stroke_rect(
        &self,
        pos: Vec2d,
        size: ResolvedSize,
        color: Color,
        stroke_width: f32,
        border_radius: [f32; 4],
    );
    fn draw_text(&self, text: &str, pos: Vec2d, font_size: f32, color: Color, font_weight: u16);
}

pub(crate) struct RetainedPickerCanvas<'a> {
    canvas: &'a Canvas,
    scale: f32,
}

impl<'a> RetainedPickerCanvas<'a> {
    pub(crate) fn new(canvas: &'a Canvas, scale: f32) -> Self {
        Self {
            canvas,
            scale: scale.max(f32::EPSILON),
        }
    }
}

pub(crate) fn record_local_v2(ctx: &BuildContext, paint: impl FnOnce(&RetainedPickerCanvas<'_>)) {
    let canvas = Canvas::of(ctx);
    let painter = RetainedPickerCanvas::new(&canvas, ctx.scale);
    paint(&painter);
    canvas.finish();
}

impl PickerPaintCanvas for RetainedPickerCanvas<'_> {
    fn scale(&self) -> f32 {
        self.scale
    }

    fn fill_color_rect(
        &self,
        pos: Vec2d,
        size: ResolvedSize,
        color: Color,
        border_radius: [f32; 4],
    ) {
        let (red, green, blue, alpha) = color.to_rgba();
        self.canvas.fill_rect_styled(
            aimer_cupid::utilities::Rect::new(
                pos.x / self.scale,
                pos.y / self.scale,
                size.width / self.scale,
                size.height / self.scale,
            ),
            aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
            border_radius.map(|radius| radius / self.scale),
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
        );
    }

    fn stroke_rect(
        &self,
        pos: Vec2d,
        size: ResolvedSize,
        color: Color,
        stroke_width: f32,
        border_radius: [f32; 4],
    ) {
        let (red, green, blue, alpha) = color.to_rgba();
        self.canvas.fill_rect_styled(
            aimer_cupid::utilities::Rect::new(
                pos.x / self.scale,
                pos.y / self.scale,
                size.width / self.scale,
                size.height / self.scale,
            ),
            aimer_cupid::utilities::Color::transparent(),
            border_radius.map(|radius| radius / self.scale),
            [stroke_width / self.scale; 4],
            aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
            [0.0; 4],
            aimer_cupid::utilities::Color::transparent(),
        );
    }

    fn draw_text(&self, text: &str, pos: Vec2d, font_size: f32, color: Color, font_weight: u16) {
        let (red, green, blue, alpha) = color.to_rgba();
        self.canvas.draw_text_styled(
            std::sync::Arc::<str>::from(text),
            aimer_cupid::utilities::Vec2d::new(pos.x / self.scale, pos.y / self.scale),
            font_size / self.scale,
            aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
            None,
            None,
            aimer_cupid::text_pipeline::TextOverflowMode::Clip,
            aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Left,
            aimer_cupid::font::FontFamily::SANS_SERIF,
            aimer_cupid::font::FontStyle::Normal,
            font_weight,
            None,
            true,
        );
    }
}

pub(crate) fn draw_calendar(
    ctx: &impl PickerPaintCanvas,
    calendar: &Calendar,
    origin: Vec2d,
    width: f32,
    height: f32,
    has_focus: bool,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    ctx.fill_color_rect(
        origin,
        ResolvedSize { width, height },
        tokens.colors.surface,
        [tokens.shape.medium * scale; 4],
    );
    let month = calendar.visible_month();
    ctx.draw_text(
        &format!("{:04}-{:02}", month.year(), month.month()),
        Vec2d {
            x: origin.x + tokens.spacing.medium * scale,
            y: origin.y + 22.0 * scale,
        },
        tokens.typography.title.font_size * scale,
        tokens.colors.on_surface,
        text_weight(tokens.typography.title.font_weight),
    );

    let weekdays = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    let cell_width = width / 7.0;
    for (index, weekday) in weekdays.iter().enumerate() {
        ctx.draw_text(
            weekday,
            Vec2d {
                x: origin.x + index as f32 * cell_width + tokens.spacing.small * scale,
                y: origin.y + (super::CALENDAR_HEADER + 17.0) * scale,
            },
            tokens.typography.label.font_size * scale,
            apply_state_layer(tokens.colors.on_surface, tokens.state.disabled),
            text_weight(tokens.typography.label.font_weight),
        );
    }

    let cells_top = (super::CALENDAR_HEADER + super::CALENDAR_WEEKDAYS) * scale;
    let cell_height = (height - cells_top).max(0.0) / 6.0;
    for (index, cell) in calendar.cells().into_iter().enumerate() {
        let row = index / 7;
        let column = index % 7;
        let cell_origin = Vec2d {
            x: origin.x + column as f32 * cell_width,
            y: origin.y + cells_top + row as f32 * cell_height,
        };
        let focused = has_focus && cell.date() == calendar.focused_date();
        let range_interior =
            cell.is_selected() && calendar.selection().is_range_interior(cell.date());
        let cell_color = if cell.is_selected() {
            if range_interior {
                tokens.colors.primary.with_alpha(0.5)
            } else {
                tokens.colors.primary
            }
        } else if focused {
            apply_state_layer(tokens.colors.surface, tokens.state.selected)
        } else if cell.is_disabled() || !cell.in_visible_month() {
            apply_state_layer(tokens.colors.surface, tokens.state.disabled)
        } else {
            tokens.colors.surface
        };
        ctx.fill_color_rect(
            Vec2d {
                x: cell_origin.x + 1.0 * scale,
                y: cell_origin.y + 1.0 * scale,
            },
            ResolvedSize {
                width: (cell_width - 2.0 * scale).max(0.0),
                height: (cell_height - 2.0 * scale).max(0.0),
            },
            cell_color,
            [tokens.shape.small * scale; 4],
        );
        if focused {
            ctx.stroke_rect(
                Vec2d {
                    x: cell_origin.x + 1.0 * scale,
                    y: cell_origin.y + 1.0 * scale,
                },
                ResolvedSize {
                    width: (cell_width - 2.0 * scale).max(0.0),
                    height: (cell_height - 2.0 * scale).max(0.0),
                },
                tokens.control.focus_ring.ring_color,
                tokens.control.focus_ring.ring_width * scale,
                [tokens.shape.small * scale; 4],
            );
        }
        let text_color = if cell.is_selected() {
            if range_interior {
                tokens.colors.on_primary.with_alpha(0.5)
            } else {
                tokens.colors.on_primary
            }
        } else if cell.is_disabled() || !cell.in_visible_month() {
            apply_state_layer(tokens.colors.on_surface, tokens.state.disabled)
        } else {
            tokens.colors.on_surface
        };
        ctx.draw_text(
            &cell.date().day().to_string(),
            Vec2d {
                x: cell_origin.x + tokens.spacing.small * scale,
                y: cell_origin.y + 20.0 * scale,
            },
            tokens.typography.body.font_size * 0.75 * scale,
            text_color,
            text_weight(tokens.typography.body.font_weight),
        );
    }
}

pub(crate) fn draw_picker_field(
    ctx: &impl PickerPaintCanvas,
    label: &str,
    value: String,
    width: f32,
    open: bool,
    focused: bool,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    let background = if focused {
        apply_state_layer(tokens.colors.surface, tokens.state.selected)
    } else {
        tokens.colors.surface
    };
    ctx.fill_color_rect(
        Vec2d::default(),
        ResolvedSize {
            width,
            height: super::PICKER_FIELD_HEIGHT * scale,
        },
        background,
        [tokens.shape.medium * scale; 4],
    );
    if focused {
        ctx.stroke_rect(
            Vec2d::default(),
            ResolvedSize {
                width: (width - tokens.control.focus_ring.ring_width * scale).max(0.0),
                height: (super::PICKER_FIELD_HEIGHT * scale
                    - tokens.control.focus_ring.ring_width * scale)
                    .max(0.0),
            },
            tokens.control.focus_ring.ring_color,
            tokens.control.focus_ring.ring_width * scale,
            [tokens.shape.medium * scale; 4],
        );
    }
    ctx.draw_text(
        &format!("{label}: {value}"),
        Vec2d {
            x: tokens.spacing.medium * scale,
            y: 26.0 * scale,
        },
        tokens.typography.body.font_size * 0.875 * scale,
        tokens.colors.on_surface,
        text_weight(tokens.typography.body.font_weight),
    );
    ctx.draw_text(
        if open { "▲" } else { "▼" },
        Vec2d {
            x: (width - tokens.spacing.medium * 1.5 * scale).max(0.0),
            y: 26.0 * scale,
        },
        tokens.typography.label.font_size * 0.875 * scale,
        tokens.colors.on_surface,
        text_weight(tokens.typography.label.font_weight),
    );
}

pub(crate) fn draw_overlay_border(
    ctx: &impl PickerPaintCanvas,
    width: f32,
    height: f32,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    ctx.stroke_rect(
        Vec2d {
            x: 0.5 * scale,
            y: 0.5 * scale,
        },
        ResolvedSize {
            width: (width - scale).max(0.0),
            height: (height - scale).max(0.0),
        },
        tokens.colors.outline,
        scale,
        [tokens.shape.medium * scale; 4],
    );
}

pub(crate) fn draw_segmented_picker_header(
    ctx: &impl PickerPaintCanvas,
    width: f32,
    active_date: bool,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    let height = super::PICKER_FIELD_HEIGHT * scale;
    ctx.fill_color_rect(
        Vec2d::default(),
        ResolvedSize { width, height },
        tokens.colors.surface,
        [tokens.shape.medium * scale; 4],
    );
    let half = width / 2.0;
    let active_origin = if active_date { 0.0 } else { half };
    ctx.fill_color_rect(
        Vec2d {
            x: active_origin,
            y: 0.0,
        },
        ResolvedSize {
            width: half,
            height,
        },
        tokens.colors.primary.with_alpha(0.12),
        [tokens.shape.small * scale; 4],
    );
    ctx.fill_color_rect(
        Vec2d {
            x: (half - 0.5 * scale).max(0.0),
            y: 6.0 * scale,
        },
        ResolvedSize {
            width: scale,
            height: (height - 12.0 * scale).max(0.0),
        },
        tokens.colors.outline,
        [0.0; 4],
    );
    let weight = text_weight(tokens.typography.label.font_weight);
    for (index, label) in ["Date", "Time"].into_iter().enumerate() {
        ctx.draw_text(
            label,
            Vec2d {
                x: index as f32 * half + tokens.spacing.medium * scale,
                y: 26.0 * scale,
            },
            tokens.typography.label.font_size * 0.925 * scale,
            if (index == 0) == active_date {
                tokens.colors.primary
            } else {
                tokens.colors.on_surface
            },
            weight,
        );
    }
}

pub(crate) fn draw_footer(
    ctx: &impl PickerPaintCanvas,
    width: f32,
    height: f32,
    focused: bool,
    tokens: &ThemeTokens,
) {
    draw_action_footer(ctx, width, height, focused, "Confirm", tokens);
}

pub(crate) fn draw_done_footer(
    ctx: &impl PickerPaintCanvas,
    width: f32,
    height: f32,
    focused: bool,
    tokens: &ThemeTokens,
) {
    draw_action_footer(ctx, width, height, focused, "Done", tokens);
}

fn draw_action_footer(
    ctx: &impl PickerPaintCanvas,
    width: f32,
    height: f32,
    focused: bool,
    confirm_label: &str,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    let y = (height - super::PICKER_FOOTER_HEIGHT * scale).max(0.0);
    ctx.fill_color_rect(
        Vec2d { x: 0.0, y },
        ResolvedSize {
            width,
            height: super::PICKER_FOOTER_HEIGHT * scale,
        },
        tokens.colors.background,
        [tokens.shape.small * scale; 4],
    );
    let text_color = if focused {
        tokens.colors.primary
    } else {
        tokens.colors.on_surface
    };
    let weight = text_weight(tokens.typography.label.font_weight);
    ctx.draw_text(
        "Cancel",
        Vec2d {
            x: tokens.spacing.medium * scale,
            y: y + 25.0 * scale,
        },
        tokens.typography.label.font_size * 0.925 * scale,
        text_color,
        weight,
    );
    ctx.draw_text(
        confirm_label,
        Vec2d {
            x: (width / 2.0 + tokens.spacing.medium * scale).min(width),
            y: y + 25.0 * scale,
        },
        tokens.typography.label.font_size * 0.925 * scale,
        text_color,
        weight,
    );
}

pub(crate) fn draw_color_picker(
    ctx: &impl PickerPaintCanvas,
    picker: &ColorPicker,
    width: f32,
    height: f32,
    active_channel: ColorChannel,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    let draft = picker.draft();
    let rgba = draft.to_rgba();
    ctx.fill_color_rect(
        Vec2d {
            x: 0.0,
            y: 48.0 * scale,
        },
        ResolvedSize {
            width,
            height: 30.0 * scale,
        },
        Color::Rgba(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha()),
        [tokens.shape.small * scale; 4],
    );
    for (index, swatch) in picker.swatches().iter().enumerate() {
        let color = swatch.color().to_rgba();
        ctx.fill_color_rect(
            Vec2d {
                x: index as f32 * 32.0 * scale,
                y: 82.0 * scale,
            },
            ResolvedSize {
                width: 26.0 * scale,
                height: 22.0 * scale,
            },
            Color::Rgba(color.red(), color.green(), color.blue(), color.alpha()),
            [tokens.shape.small * scale; 4],
        );
        if swatch.is_disabled() {
            ctx.draw_text(
                "×",
                Vec2d {
                    x: index as f32 * 32.0 * scale + 8.0 * scale,
                    y: 99.0 * scale,
                },
                tokens.typography.body.font_size * 0.875 * scale,
                apply_state_layer(tokens.colors.on_surface, tokens.state.disabled),
                text_weight(tokens.typography.label.font_weight),
            );
        }
    }
    let channels = [
        (ColorChannel::Hue, "H", draft.hue().to_string()),
        (
            ColorChannel::Saturation,
            "S",
            draft.saturation().to_string(),
        ),
        (ColorChannel::Value, "V", draft.value().to_string()),
        (ColorChannel::Alpha, "A", draft.alpha().to_string()),
    ];
    for (index, (channel, label, value)) in channels.into_iter().enumerate() {
        let y = 116.0 + index as f32 * 30.0;
        let disabled = channel == ColorChannel::Alpha && !picker.alpha_enabled();
        ctx.draw_text(
            &format!("{label} {value}"),
            Vec2d {
                x: 2.0 * scale,
                y: (y + 8.0) * scale,
            },
            tokens.typography.label.font_size * 0.8 * scale,
            if disabled {
                apply_state_layer(tokens.colors.on_surface, tokens.state.disabled)
            } else if channel == active_channel {
                tokens.colors.primary
            } else {
                tokens.colors.on_surface
            },
            text_weight(tokens.typography.label.font_weight),
        );
    }
    draw_footer(
        ctx,
        width,
        height,
        active_channel == ColorChannel::Alpha,
        tokens,
    );
}

pub(crate) fn draw_time_picker(
    ctx: &impl PickerPaintCanvas,
    time: TimeOfDay,
    width: f32,
    height: f32,
    body_top: f32,
    active_column: usize,
    use_24_hours: bool,
    tokens: &ThemeTokens,
) {
    let scale = ctx.scale().max(f32::EPSILON);
    let wheel_bottom = (height - super::PICKER_FOOTER_HEIGHT * scale).max(0.0);
    let body_top = body_top * scale;
    let wheel_top = body_top + super::TIME_WHEEL_CONTENT_TOP * scale;
    ctx.fill_color_rect(
        Vec2d {
            x: 0.0,
            y: body_top,
        },
        ResolvedSize {
            width,
            height: (wheel_bottom - body_top).max(0.0),
        },
        tokens.colors.surface,
        [tokens.shape.medium * scale; 4],
    );

    let columns: &[&str] = if use_24_hours {
        &["Hour", "Min", "Sec"]
    } else {
        &["Hour", "Min", "Sec", "AM/PM"]
    };
    let column_width = width / columns.len() as f32;
    let selected_row = super::TIME_WHEEL_ROWS / 2;
    for (column, label) in columns.into_iter().enumerate() {
        let column_origin = column as f32 * column_width;
        ctx.draw_text(
            label,
            Vec2d {
                x: column_origin + 8.0 * scale,
                y: wheel_top,
            },
            tokens.typography.label.font_size * 0.8 * scale,
            if column == active_column {
                tokens.colors.primary
            } else {
                tokens.colors.on_surface.with_alpha(0.5)
            },
            text_weight(tokens.typography.label.font_weight),
        );
        for row in 0..super::TIME_WHEEL_ROWS {
            let offset = row as i32 - selected_row as i32;
            let selected = row == selected_row;
            let row_top = (body_top / scale
                + super::TIME_WHEEL_CONTENT_TOP
                + row as f32 * super::TIME_WHEEL_ROW_HEIGHT)
                * scale;
            if selected {
                ctx.fill_color_rect(
                    Vec2d {
                        x: column_origin + 2.0 * scale,
                        y: row_top,
                    },
                    ResolvedSize {
                        width: (column_width - 4.0 * scale).max(0.0),
                        height: super::TIME_WHEEL_ROW_HEIGHT * scale,
                    },
                    tokens.colors.on_surface.with_alpha(0.08),
                    [tokens.shape.small * scale; 4],
                );
            }
            if !use_24_hours && column == 3 && !selected {
                continue;
            }
            let value = time_wheel_value(time, column, offset, use_24_hours);
            let text_color = if selected {
                tokens.colors.primary
            } else {
                tokens.colors.on_surface.with_alpha(0.5)
            };
            ctx.draw_text(
                &value,
                Vec2d {
                    x: column_origin + 8.0 * scale,
                    y: row_top + 21.0 * scale,
                },
                tokens.typography.body.font_size * 0.9 * scale,
                text_color,
                text_weight(tokens.typography.body.font_weight),
            );
        }
    }
}

fn time_wheel_value(time: TimeOfDay, column: usize, offset: i32, use_24_hours: bool) -> String {
    match column {
        0 if use_24_hours => format!("{:02}", wrapped_value(time.hour(), 24, offset)),
        0 => hour12(time.hour(), offset).to_string(),
        1 => format!("{:02}", wrapped_component(time.minute(), offset)),
        2 => format!("{:02}", wrapped_component(time.second(), offset)),
        _ => time_period(time.hour()).to_owned(),
    }
}

fn hour12(hour: u8, offset: i32) -> u8 {
    let current = i32::from(hour12_value(hour)) - 1;
    (current + offset).rem_euclid(12) as u8 + 1
}

fn hour12_value(hour: u8) -> u8 {
    if hour % 12 == 0 { 12 } else { hour % 12 }
}

fn wrapped_component(value: u8, offset: i32) -> u8 {
    wrapped_value(value, 60, offset)
}

fn wrapped_value(value: u8, limit: i32, offset: i32) -> u8 {
    (i32::from(value) + offset).rem_euclid(limit) as u8
}

fn time_period(hour: u8) -> &'static str {
    if hour >= 12 { "PM" } else { "AM" }
}

fn text_weight(weight: f32) -> u16 {
    weight.round().clamp(1.0, 1000.0) as u16
}
