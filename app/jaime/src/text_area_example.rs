//! A focused showcase for Aimer's multiline text editor.

use aimer::input::{TextArea, TextEditingController};
use aimer::style::*;
use aimer::*;

use crate::theme;

/// Starts the multiline text-area showcase.
pub fn start_text_area_example() {
    AimerApp::start(theme::provide(TextAreaExample::new().boxed()))
}

/// Demonstrates wrapped multiline input with bounded vertical growth.
#[derive(Clone, StatelessWidget)]
pub struct TextAreaExample {
    controller: TextEditingController,
}

impl TextAreaExample {
    /// Creates the showcase with editable multiline content.
    #[inline]
    pub fn new() -> Self {
        Self {
            controller: TextEditingController::with_text(
                "Write across multiple lines.\nLong lines wrap within the available width.",
            ),
        }
    }
}

impl Default for TextAreaExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl StatelessWidget for TextAreaExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);
        let text_style = TextStyle::new()
            .font_size(16)
            .color(theme.on_surface_color);

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(Column::new().children(vec![
                    Text::new("TextArea")
                        .text_style(
                            TextStyle::new()
                                .font_size(28)
                                .font_weight(FontWeight::Bold)
                                .color(theme.on_background_color),
                        )
                        .boxed(),
                    Text::new("Multiline input with soft wrapping and vertical scrolling.")
                        .text_style(text_style.color(crate::theme::muted_text(&theme)))
                        .boxed(),
                    TextArea::new()
                        .controller(self.controller.clone())
                        .hint("Write a longer message")
                        .hint_style(text_style.color(theme::muted_text(&theme).with_alpha(0.7)))
                        .text_style(text_style.color(theme.on_surface_color))
                        .min_lines(1)
                        .max_lines(Some(10))
                        .decoration(theme::input_decoration(&theme))
                        .hover_decoration(theme::input_hover_decoration(&theme))
                        .focus_decoration(theme::input_focus_decoration(&theme))
                        .cursor_color(theme::input_cursor_color(&theme))
                        .selection_color(theme.primary_color.with_alpha(0.28))
                        .padding(LayoutSpacing::all(Spacing::Px(12)))
                        .on_changed(|text: String| {
                            println!("TextArea contains {} bytes", text.len())
                        })
                        .boxed(),
                ]))
    }
}
