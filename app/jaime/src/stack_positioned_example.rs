use aimer::style::{BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, ThemeData};
use aimer::{AnyWidget, Column, Container, Dimension, Positioned, ScrollAxis, Scrollable, Stack, Text, Widget};

use crate::theme;

/// Demonstrates edge offsets, percentage positioning, and explicit stack layers.
pub fn stack_positioned_example(app_theme: ThemeData) -> impl Widget {
    Scrollable::new().axis(ScrollAxis::Vertical).child(
        Container::new().child(Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
            Text::new("Overlapping layers")
                .text_style(TextStyle::new().font_size(20).font_weight(FontWeight::Bold)
                    .color(app_theme.on_background_color)).boxed(),
            Text::new("Stack gives its children the same area. Positioned places each card relative to that area using logical pixels or percentages.")
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .wrapped().boxed(),
            Container::new().width(520).height(300)
                .box_decoration(BoxDecoration::new()
                    .background_color(theme::recessed_surface(&app_theme)).border_radius(12))
                .child(Stack::new().children([
                    // Declared first, but painted last because its layer is higher.
                    Positioned::new().left(Dimension::Percent(30.0)).top(128).layer(2)
                        .child(card("Layer 2", 208.0, 44.0, 0.0, app_theme)).boxed(),
                    Positioned::new().left(24).top(24)
                        .child(card("Top / left", 260.0, 150.0, 0.30, app_theme)).boxed(),
                    Positioned::new().right(24).bottom(24).layer(1)
                        .child(card("Bottom / right", 260.0, 150.0, 0.16, app_theme)).boxed(),
                ])).boxed(),
            Text::new("Top / left: left(24), top(24). Bottom / right: right(24), bottom(24). Both cards are 260 × 150 inside a 520 × 300 stack, so their edges overlap.")
                .text_style(TextStyle::new().font_size(15).color(app_theme.on_background_color))
                .wrapped().boxed(),
            Text::new("The middle card uses a 30% left offset, top(128), and layer(2). Higher layers paint later: this card stays in front even though it is added first.")
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .wrapped().boxed(),
        ])),
    )
}

fn card(label: &'static str, width: f32, height: f32, darken: f32, app_theme: ThemeData) -> AnyWidget {
    Container::new().width(width).height(height)
        .box_decoration(BoxDecoration::new()
            .background_color(app_theme.primary_color.darken(darken)).border_radius(10))
        .child(Text::new(label).text_align(TextAlign::MidCenter)
            .text_style(TextStyle::new().font_size(16).font_weight(FontWeight::Bold)
                .color(app_theme.on_primary_color)))
        .boxed()
}
