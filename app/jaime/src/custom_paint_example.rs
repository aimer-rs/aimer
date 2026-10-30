//! A showcase of `CustomPaint` and `CanvasPainter`.

use aimer::canvas::{Canvas, RenderColor};
use aimer::style::*;
use aimer::*;

use crate::theme;

/// Builds the custom-paint page without starting the application.
#[inline]
pub fn custom_paint_example() -> impl Widget {
    CustomPaintExample
}

struct CustomPaintExample;

impl aimer::PortableWidget for CustomPaintExample {}

impl Widget for CustomPaintExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let theme = ThemeData::copied(ctx);
        let heading = TextStyle::new()
            .font_size(18)
            .font_weight(FontWeight::Bold)
            .color(theme.on_background_color);
        let note = TextStyle::new()
            .font_size(14)
            .color(theme::muted_text(&theme));

        Column::new()
            .children(vec![
                Text::new("Fixed size: a bar chart")
                    .text_style(heading)
                    .boxed(),
                Text::new("320 × 160 logical pixels, drawn with rectangles and text.")
                    .text_style(note)
                    .boxed(),
                SizedBox::new().height(10).boxed(),
                CustomPaint::new()
                    .width(320)
                    .height(160)
                    .box_painter(BarChart {
                        values: [0.45, 0.8, 0.3, 0.95, 0.6, 0.7],
                        bar: rgba(theme.primary_color),
                        axis: rgba(theme::muted_text(&theme)),
                    }),
                SizedBox::new().height(26).boxed(),
                Text::new("Fills the width: a color ramp")
                    .text_style(heading)
                    .boxed(),
                Text::new("Resize the window: the cell count follows the width, and the page is recorded again only then.")
                    .text_style(note)
                    .boxed(),
                SizedBox::new().height(10).boxed(),
                CustomPaint::new()
                    .height(56)
                    .box_painter(ColorRamp { cell: 28.0, gap: 4.0 }),
                SizedBox::new().height(26).boxed(),
                Text::new("Rounded, bordered, labelled")
                    .text_style(heading)
                    .boxed(),
                SizedBox::new().height(10).boxed(),
                CustomPaint::new()
                    .width(320)
                    .height(72)
                    .box_painter(Badge {
                        fill: rgba(theme.primary_color),
                        border: rgba(theme.primary_color.lighten(0.2)),
                        label: rgba(theme.on_primary_color),
                    }),
            ])
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "CustomPaintExample"
    }
}

#[inline]
fn rgba(color: Color) -> [u8; 4] {
    let (red, green, blue, alpha) = color.to_rgba();
    [red, green, blue, alpha]
}

#[inline]
fn render_color([red, green, blue, alpha]: [u8; 4]) -> RenderColor {
    RenderColor::rgba8(red, green, blue, alpha)
}

struct BarChart {
    values: [f32; 6],
    bar: [u8; 4],
    axis: [u8; 4],
}

impl CanvasPainter for BarChart {
    fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
        let label_room = 20.0;
        let plot_height = (size.height - label_room).max(0.0);
        let slot = size.width / self.values.len() as f32;
        let width = slot * 0.6;

        for (index, value) in self.values.iter().enumerate() {
            let height = plot_height * value;
            let x = slot * index as f32 + (slot - width) / 2.0;
            canvas.fill_rect_styled(
                aimer::canvas::Rect::new(x, plot_height - height, width, height),
                render_color(self.bar),
                [4.0, 4.0, 0.0, 0.0],
                [0.0; 4],
                RenderColor::transparent(),
                [0.0; 4],
                RenderColor::transparent(),
            );
            canvas.draw_text(
                format!("Q{}", index + 1),
                [x + width / 2.0 - 8.0, plot_height + 3.0],
                13.0,
                self.axis,
            );
        }
        canvas.fill_rect(
            aimer::canvas::Rect::new(0.0, plot_height, size.width, 1.0),
            self.axis,
        );
    }
}

struct ColorRamp {
    cell: f32,
    gap: f32,
}

impl CanvasPainter for ColorRamp {
    fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
        let count = ((size.width + self.gap) / (self.cell + self.gap)).floor().max(1.0) as usize;
        for index in 0..count {
            let hue = index as f32 / count as f32 * 300.0;
            canvas.fill_rect_styled(
                aimer::canvas::Rect::new(
                    index as f32 * (self.cell + self.gap),
                    (size.height - self.cell) / 2.0,
                    self.cell,
                    self.cell,
                ),
                render_color(hue_to_rgba(hue)),
                [self.cell / 4.0; 4],
                [0.0; 4],
                RenderColor::transparent(),
                [0.0; 4],
                RenderColor::transparent(),
            );
        }
    }
}

struct Badge {
    fill: [u8; 4],
    border: [u8; 4],
    label: [u8; 4],
}

impl CanvasPainter for Badge {
    fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
        canvas.fill_rect_with_border(
            aimer::canvas::Rect::new(0.0, 0.0, size.width, size.height),
            render_color(self.fill),
            [size.height / 2.0; 4],
            3.0,
            render_color(self.border),
        );
        canvas.draw_text(
            "Drawn by a CanvasPainter",
            [size.height / 2.0, size.height / 2.0 - 10.0],
            20.0,
            self.label,
        );
    }
}

/// Full-saturation, full-value HSV with `hue` in degrees.
fn hue_to_rgba(hue: f32) -> [u8; 4] {
    let sector = hue / 60.0;
    let ramp = (255.0 * (1.0 - (sector % 2.0 - 1.0).abs())) as u8;
    let (red, green, blue) = match sector as u32 {
        0 => (255, ramp, 0),
        1 => (ramp, 255, 0),
        2 => (0, 255, ramp),
        3 => (0, ramp, 255),
        4 => (ramp, 0, 255),
        _ => (255, 0, ramp),
    };
    [red, green, blue, 255]
}
