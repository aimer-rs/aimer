use aimer::style::{
    BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, ThemeData,
};
use aimer::{
    Align, Alignment, AnyWidget, Column, Container, Expanded, Row, ScrollAxis, Scrollable, Text,
    Widget,
};

use crate::theme;

/// Compares all nine alignments in equally sized parent boxes.
pub fn align_example(app_theme: ThemeData) -> impl Widget {
    let mut sections = vec![
        Text::new("Place a child within its parent")
            .text_style(TextStyle::new().font_size(20).font_weight(FontWeight::Bold)
                .color(app_theme.on_background_color)).boxed(),
        Text::new("Each box has the same height and the same 48 × 32 child. Align changes the
       child's position within the available space. Resize the window to compare horizontal placement.")
            .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
            .wrapped().boxed(),
    ];
    for row in [
        [
            ("TopLeft", Alignment::TopLeft),
            ("TopCenter", Alignment::TopCenter),
            ("TopRight", Alignment::TopRight),
        ],
        [
            ("MidLeft", Alignment::MidLeft),
            ("MidCenter", Alignment::MidCenter),
            ("MidRight", Alignment::MidRight),
        ],
        [
            ("BotLeft", Alignment::BotLeft),
            ("BotCenter", Alignment::BotCenter),
            ("BotRight", Alignment::BotRight),
        ],
    ] {
        sections.push(
            Row::new()
                .gaps(LayoutSpacing::all(Spacing::Px(12)))
                .children(row.map(|(label, alignment)| {
                    Expanded::new()
                        .child(sample(label, alignment, app_theme))
                        .boxed()
                }))
                .boxed(),
        );
    }
    sections.push(Text::new("Align::new().alignment(Alignment::MidCenter).child(...) centers a child. The default alignment is TopCenter.")
        .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
        .wrapped().boxed());

    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(
            Container::new().child(
                Column::new()
                    .gaps(LayoutSpacing::all(Spacing::Px(16)))
                    .children(sections),
            ),
        )
}

fn sample(label: &'static str, alignment: Alignment, app_theme: ThemeData) -> AnyWidget {
    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(6)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(14)
                        .color(app_theme.on_background_color),
                )
                .boxed(),
            Container::new()
                .height(112)
                .box_decoration(
                    BoxDecoration::new()
                        .background_color(theme::recessed_surface(&app_theme))
                        .border_radius(10),
                )
                .child(
                    Align::new()
                        .alignment(alignment)
                        .child(
                            Container::new()
                                .width(48)
                                .height(32)
                                .box_decoration(
                                    BoxDecoration::new()
                                        .background_color(app_theme.primary_color)
                                        .border_radius(6),
                                )
                                .child(
                                    Text::new("A")
                                        .text_align(TextAlign::MidCenter)
                                        .text_style(
                                            TextStyle::new()
                                                .font_size(16)
                                                .font_weight(FontWeight::Bold)
                                                .color(app_theme.on_primary_color),
                                        ),
                                ),
                        ),
                )
                .boxed(),
        ])
        .boxed()
}
