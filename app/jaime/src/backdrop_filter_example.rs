//! Text behind a filtered box stays readable outside its rectangular bounds.

use aimer::decoration::{BackdropFilter, DecorationBox};
use aimer::style::{
    BorderSlice, BorderStyle, BoxBorder, BoxDecoration, Color, FontWeight, LayoutSpacing, Spacing,
    Stroke, TextStyle,
};
use aimer::{
    AimerApp, Column, Container, Positioned, Stack, Text, Widget,
};

use crate::theme;

/// Starts the standalone backdrop-filter example.
pub fn start_backdrop_filter_example() {
    AimerApp::start(theme::provide(backdrop_filter_example()));
}

/// Builds a text backdrop with a translucent, filtered box in front of it.
///
/// The stack paints the large words first. The decoration then filters those
/// words before its child paints the tint, border, and sharp foreground text.
pub fn backdrop_filter_example() -> impl Widget {
    let mut backdrop = Stack::new();
    for (index, line) in [
        "AIMER / TEXT / AIMER / TEXT",
        "TEXT BEHIND THE BOX / AIMER",
        "AIMER / BACKDROP / AIMER",
        "WORDS / AIMER / WORDS / AIMER",
        "AIMER / TEXT / AIMER / TEXT",
        "TEXT BEHIND THE BOX / AIMER",
        "AIMER / BACKDROP / AIMER",
    ]
    .into_iter()
    .enumerate()
    {
        backdrop = backdrop.add_child(
            Positioned::new()
                .left(24)
                .top(24 + index as i32 * 52)
                .child(
                    Text::new(line).text_style(
                        TextStyle::new()
                            .font_size(28)
                            .font_weight(FontWeight::Bold)
                            .color(if index % 2 == 0 {
                                Color::Rgb(28, 81, 126)
                            } else {
                                Color::Rgb(119, 65, 145)
                            }),
                    ),
                ),
        );
    }

    let foreground = DecorationBox::new()
        .decoration(BackdropFilter::new().blur_radius(10.0)
            .saturation(0.65).brightness(1.05).contrast(0.9)
        )
        .child(
            Container::new().width(320).height(208)
                .padding(LayoutSpacing::all(Spacing::Px(24)))
                .box_decoration(BoxDecoration::new()
                    .background_color(Color::Rgba(255, 255, 255, 72))
                    .border(BoxBorder::all(BorderSlice::new()
                        .style(BorderStyle::Solid).stroke(Stroke::Px(1.0))
                        .color(Color::Rgba(255, 255, 255, 190)))))
                .child(Column::new().gaps(LayoutSpacing::all(Spacing::Px(12))).children([
                    Text::new("This text stays sharp")
                        .text_style(TextStyle::new().font_size(20).font_weight(FontWeight::Bold)
                            .color(Color::Rgb(16, 39, 65))).boxed(),
                    Text::new("The large words behind this box are blurred. Its own text is painted after the filter.")
                        .text_style(TextStyle::new().font_size(15).color(Color::Rgb(16, 39, 65)))
                        .wrapped().boxed(),
                    Text::new("Blur + saturation + brightness + contrast")
                        .text_style(TextStyle::new().font_size(12).color(Color::Rgb(41, 64, 85)))
                        .wrapped().boxed(),
                ])),
        );

    Column::new().gaps(LayoutSpacing::all(Spacing::Px(16))).children([
        Text::new("Compare the sharp background words outside the box with the filtered words behind it.")
            .text_style(TextStyle::new().font_size(15).color(theme::app_theme().on_background_color))
            .wrapped().boxed(),
        Container::new().width(640).height(400).color(Color::Rgb(218, 237, 247))
            .child(backdrop.add_child(Positioned::new().left(160).top(92).child(foreground)))
            .boxed(),
    ])
}
