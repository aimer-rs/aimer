use aimer::style::{FontWeight, LayoutSpacing, Spacing, TextStyle, ThemeData};
use aimer::{AimerApp, AnyWidget, Column, Container, OverflowBehavior, Row, Text, Widget};

use crate::theme;

const DEMO_WIDTH: f32 = 360.0;
const DEMO_HEIGHT: f32 = 150.0;
const ITEM_WIDTH: f32 = 150.0;
const ITEM_HEIGHT: f32 = 44.0;

/// Builds a showcase of the three [`OverflowBehavior`] modes.
///
/// Every sample uses the same constrained row and children. `Hidden` clips the
/// overflowing child, `Wrap` moves it to a second line, and `Visible` paints it
/// beyond the row's width.
pub fn overflow_behavior_example() -> impl Widget {
    let app_theme = theme::app_theme();

    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(28)))
        .color(app_theme.background_color)
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children([
                    Text::new("OverflowBehavior")
                        .text_style(
                            TextStyle::new()
                                .font_size(28)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_background_color),
                        )
                        .boxed(),
                    Text::new(
                        "The same children are clipped, wrapped, or allowed outside the row.",
                    )
                    .text_style(
                        TextStyle::new()
                            .font_size(16)
                            .color(theme::muted_text(&app_theme)),
                    )
                    .boxed(),
                    demo("Hidden", OverflowBehavior::Hidden, app_theme),
                    demo("Wrap", OverflowBehavior::Wrap, app_theme),
                    demo("Visible", OverflowBehavior::Visible, app_theme),
                ]),
        )
}

pub fn start_overflow_behavior_example() {
    AimerApp::start(crate::theme::provide(overflow_behavior_example()));
}

fn demo(label: &'static str, overflow: OverflowBehavior, app_theme: ThemeData) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(4)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
            Container::new()
                // .width(DEMO_WIDTH)
                // .height(DEMO_HEIGHT)
                .padding(LayoutSpacing::all(Spacing::Px(10)))
                .color(theme::raised_surface(&app_theme))
                .child(
                    Row::new()
                        .overflow(overflow)
                        .gaps(LayoutSpacing::all(Spacing::Px(8)))
                        .children([
                            item("A", app_theme),
                            item("B", app_theme),
                            item("C", app_theme),
                        ]),
                )
                .boxed(),
        ])
        .boxed()
}

fn item(label: &'static str, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .width(ITEM_WIDTH)
        .height(ITEM_HEIGHT)
        .color(app_theme.primary_color)
        .child(
            Text::new(label).text_style(
                TextStyle::new()
                    .font_size(16)
                    .color(app_theme.on_primary_color),
            ),
        )
        .boxed()
}
