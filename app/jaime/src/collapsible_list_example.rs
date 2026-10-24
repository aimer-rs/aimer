use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, FontWeight, LayoutSpacing, Spacing,
    TextStyle, ThemeData,
};
use aimer::{
    AnyWidget, BoxAlignment, CollapsibleList, Color, Column, Container, Dimension, ListBody,
    ListHeader, Row, SizedBox, Text, Widget,
};

use crate::theme;

/// Builds the native CollapsibleList showcase.
///
/// The list starts expanded so the colored tag rows are visible. Clicking or
/// keyboard-activating the header collapses the retained body while keeping
/// the header visible.
pub fn collapsible_list_example() -> impl Widget {
    let app_theme = theme::app_theme();

    Container::new()
        .width(Dimension::Px(420.0))
        .padding(LayoutSpacing::all(Spacing::Px(20)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(12),
        )
        .child(
            CollapsibleList::new()
                .header(
                    ListHeader::new().child(
                        Text::new("Tags").text_style(
                            TextStyle::new()
                                .font_size(20)
                                .font_weight(FontWeight::Bold)
                                .color(theme::muted_text(&app_theme)),
                        ),
                    ),
                )
                .body(
                    ListBody::new().child(
                        Column::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(10)))
                            .children([
                                tag_row("Red", Color::Rgb(255, 82, 82), app_theme),
                                tag_row("no-weapon-slot", Color::Transparent, app_theme),
                                tag_row("Orange", Color::Rgb(255, 173, 0), app_theme),
                                tag_row("Yellow", Color::Rgb(255, 240, 0), app_theme),
                                tag_row("Green", Color::Rgb(0, 240, 86), app_theme),
                                tag_row("Blue", Color::Rgb(0, 161, 240), app_theme),
                                tag_row("Purple", Color::Rgb(219, 82, 240), app_theme),
                            ]),
                    ),
                ),
        )
}

fn tag_row(label: &'static str, color: Color, app_theme: ThemeData) -> AnyWidget {
    Row::new()
        .vertical_alignment(BoxAlignment::Center)
        .gaps(LayoutSpacing::all(Spacing::Px(14)))
        .children([
            tag_swatch(color, app_theme),
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(17)
                        .color(app_theme.on_surface_color),
                )
                .boxed(),
        ])
        .boxed()
}

fn tag_swatch(color: Color, app_theme: ThemeData) -> AnyWidget {
    let mut decoration = BoxDecoration::new().border_radius(12);
    if color == Color::Transparent {
        decoration = decoration.border(BoxBorder::all(
            BorderSlice::new()
                .style(BorderStyle::Solid)
                .stroke(2.0)
                .color(theme::muted_text(&app_theme)),
        ));
    } else {
        decoration = decoration.background_color(color);
    }

    Container::new()
        .width(Dimension::Px(24.0))
        .height(Dimension::Px(24.0))
        .box_decoration(decoration)
        .child(SizedBox::new())
        .boxed()
}
