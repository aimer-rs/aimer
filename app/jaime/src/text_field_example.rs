
//! A focused showcase for Aimer's single-line text editor.

use aimer::input::TextEditingController;
use aimer::input::input::{InputType, TextField};
use aimer::style::*;
use aimer::*;

/// Starts the single-line text-field showcase.
pub fn start_text_field_example() {
    AimerApp::start(crate::theme::provide(TextFieldExample::new().boxed()))
}

/// Demonstrates a controlled single-line field with submission handling.
pub struct TextFieldExample {
    controller: TextEditingController,
}

impl TextFieldExample {
    /// Creates the showcase with an editable greeting.
    #[inline]
    pub fn new() -> Self {
        Self {
            controller: TextEditingController::with_text("Hello, Aimer!"),
        }
    }
}

impl Default for TextFieldExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for TextFieldExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let theme = ThemeData::copied(ctx);

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(Column::new().children(vec![
                    Text::new("TextField")
                        .text_style(
                            TextStyle::new()
                                .font_size(28)
                                .font_weight(FontWeight::Bold)
                                .color(theme.on_background_color),
                        )
                        .boxed(),
                    Text::new("Single-line input. Press Return to submit.")
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .color(crate::theme::muted_text(&theme)),
                        )
                        .boxed(),
                    TextField::new()
                        .controller(self.controller.clone())
                        .input_type(InputType::Text)
                        .hint("Type a short message")
                        .hint_style(
                            TextStyle::new()
                                .font_size(16)
                                .color(crate::theme::muted_text(&theme)),
                        )
                        .text_style(TextStyle::new().font_size(16).color(theme.on_surface_color))
                        .max_length(Some(80))
                        .decoration(crate::theme::input_decoration(&theme))
                        .hover_decoration(crate::theme::input_hover_decoration(&theme))
                        .focus_decoration(crate::theme::input_focus_decoration(&theme))
                        .cursor_color(crate::theme::input_cursor_color(&theme))
                        .selection_color(theme.primary_color.with_alpha(0.28))
                        .padding(LayoutSpacing::all(Spacing::Px(12)))
                        .on_submitted(|text: String| println!("Submitted: {text}"))
                        .boxed(),
                ]))
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "TextFieldExample"
    }
}

impl aimer::PortableWidget for TextFieldExample {}
