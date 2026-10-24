use aimer::style::{LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, BoxAlignment, Container, Dimension, Flex, FlexDirection, FontError, FontFamily,
    FontRegistration, FontRegistry, FontStyle, FontWeight, SizedBox, Text, Widget,
};

use crate::theme;

const CUSTOM_FONT_FAMILY: &str = "Jaime JetBrains Mono";
const CUSTOM_FONT_BYTES: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

fn register_custom_font() -> Result<FontFamily, FontError> {
    if let Some(family) = FontRegistry::family(CUSTOM_FONT_FAMILY) {
        return Ok(family);
    }

    FontRegistry::register(FontRegistration {
        family: CUSTOM_FONT_FAMILY,
        bytes: CUSTOM_FONT_BYTES,
        weight: FontWeight::Normal,
        style: FontStyle::Normal,
    })
}

/// Builds the custom-font showcase without starting an application.
pub fn custom_font_example() -> impl Widget {
    let app_theme = theme::app_theme();
    let custom_font = register_custom_font().expect("the embedded custom font should be valid");

    Container::new()
        .color(app_theme.background_color)
        .padding(LayoutSpacing::all(Spacing::Px(32)))
        .child(
            Flex::new()
                .direction(FlexDirection::Column)
                .vertical_alignment(BoxAlignment::Center)
                .horizontal_alignment(BoxAlignment::Center)
                .children([
                    Text::new("Text with a custom font")
                        .text_style(TextStyle::new().font_size(30).color(app_theme.on_background_color))
                        .boxed(),
                    SizedBox::new().height(24).boxed(),
                    Text::new("JetBrains Mono\nThe quick brown fox jumps over the lazy dog.\n0123456789  {}[]() =>")
                        .text_style(
                            TextStyle::new()
                                .font_family(custom_font)
                                .font_size(22)
                                .color(app_theme.primary_color),
                        )
                        .boxed(),
                    SizedBox::new().height(24).boxed(),
                    Container::new()
                        .width(Dimension::Px(520.0))
                        .child(
                            Text::new("The font is embedded with include_bytes!, registered before AimerApp::start, and selected through TextStyle::font_family.")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(16)
                                        .color(crate::theme::muted_text(&app_theme)),
                                ),
                        )
                        .boxed(),
                ]),
        )
}

pub fn start_custom_font_example() {
    AimerApp::start(theme::provide(custom_font_example()))
}

#[cfg(test)]
mod tests {
    use aimer::FontRegistry;

    use super::*;

    #[test]
    fn custom_font_example_registers_embedded_family() {
        let family = register_custom_font().expect("the embedded custom font should be valid");

        assert_eq!(FontRegistry::family(CUSTOM_FONT_FAMILY), Some(family));
    }
}
