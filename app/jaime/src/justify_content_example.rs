use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, LayoutSpacing, Spacing,
    TextAlign, TextStyle, ThemeData,
};
use aimer::{AimerApp, AnyWidget, Column, Container, JustifyContent, Row, Text, Widget};

use crate::theme;

const DEMO_WIDTH: f32 = 640.0;
const DEMO_HEIGHT: f32 = 56.0;
const ITEM_SIZE: f32 = 36.0;

/// Builds a showcase of the six [`JustifyContent`] modes.
///
/// Each row has the same available width and the same three children, making
/// the difference between positional alignment and free-space distribution
/// visible at a glance.
pub fn justify_content_example() -> impl Widget {
    let app_theme = theme::app_theme();

    Container::new()
        // .width(700)
        .padding(LayoutSpacing::all(Spacing::Px(28)))
        .color(app_theme.background_color)
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children([
                    Text::new("JustifyContent")
                        .text_style(
                            TextStyle::new()
                                .font_size(28)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_background_color),
                        )
                        .boxed(),
                    Text::new(
                        "The main-axis placement changes while every row keeps the same width.",
                    )
                    .text_style(
                        TextStyle::new()
                            .font_size(16)
                            .color(theme::muted_text(&app_theme)),
                    )
                    .boxed(),
                    demo(JustifyContent::Start, app_theme),
                    demo(JustifyContent::Center, app_theme),
                    demo(JustifyContent::End, app_theme),
                    demo(JustifyContent::SpaceBetween, app_theme),
                    demo(JustifyContent::SpaceAround, app_theme),
                    demo(JustifyContent::SpaceEvenly, app_theme),
                ]),
        )
}

pub fn start_justify_content_example() {
    AimerApp::start(crate::theme::provide(justify_content_example()));
}

fn demo(justify_content: JustifyContent, app_theme: ThemeData) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(4)))
        .children([
            Text::new(justify_content_name(justify_content))
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
                        .justify_content(justify_content)
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
        .width(ITEM_SIZE)
        .height(ITEM_SIZE)
        .color(
            app_theme
                .primary_color
                .darken(0.08),
        )
        .box_decoration(
            BoxDecoration::new().border(BoxBorder::all(
                BorderSlice::new()
                    .stroke(1)
                    .color(app_theme.on_surface_color)
                    .style(BorderStyle::Solid),
            )),
        )
        .child(
            Text::new(label)
                .text_align(TextAlign::MidCenter)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(app_theme.on_primary_color),
                ),
        )
        .boxed()
}

fn justify_content_name(justify_content: JustifyContent) -> &'static str {
    match justify_content {
        JustifyContent::Start => "Start",
        JustifyContent::Center => "Center",
        JustifyContent::End => "End",
        JustifyContent::SpaceBetween => "SpaceBetween",
        JustifyContent::SpaceAround => "SpaceAround",
        JustifyContent::SpaceEvenly => "SpaceEvenly",
    }
}
