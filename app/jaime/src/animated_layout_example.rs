//! A small, opt-in layout-transition showcase for Jaime.
//!
//! W17 registers this module in the central showcase; keeping the example
//! independently constructible lets the public builder be tested without
//! starting an application.

use std::time::Duration;

use aimer::animation::curves;
use aimer::animation::layout::AnimatedLayout;
use aimer::style::*;
use aimer::*;

/// Starts the animated-layout showcase as a standalone Jaime page.
pub fn start_animated_layout_example() {
    AimerApp::start(animated_layout_example().boxed());
}

/// Builds a monochrome layout-transition example.
pub fn animated_layout_example() -> impl Widget {
    AnimatedLayout::new()
            .duration(Duration::from_millis(250))
            .curve(curves::EASE_IN_OUT)
        .child(
            Container::new()
                .color(Colors::White.into())
                .padding(LayoutSpacing::all(Spacing::Px(24)))
                .child(
                    Column::new()
                        .gaps(LayoutSpacing::all(Spacing::Px(12)))
                        .children([
                            Text::new("Animated layout")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(26)
                                        .font_weight(FontWeight::Bold)
                                        .color(Colors::Black),
                                )
                                .boxed(),
                            Text::new(
                                "Resize, wrap, and reorder layouts through an explicit transition.",
                            )
                            .wrapped()
                            .text_style(TextStyle::new().font_size(16).color(Colors::Black))
                            .boxed(),
                            Row::new()
                                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                .children([
                                    SizedBox::new()
                                        .width(88)
                                        .height(48)
                                        .color(Colors::Black)
                                        .boxed(),
                                    SizedBox::new()
                                        .width(144)
                                        .height(48)
                                        .color(Colors::Gray)
                                        .boxed(),
                                    SizedBox::new()
                                        .width(64)
                                        .height(48)
                                        .color(Colors::Black)
                                        .boxed(),
                                ])
                                .boxed(),
                            Text::new(
                                "Stable keys keep list state attached while geometry moves; reduced motion settles immediately.",
                            )
                            .wrapped()
                            .text_style(TextStyle::new().font_size(14).color(Colors::Gray))
                            .boxed(),
                        ]),
                ),
        )
}
