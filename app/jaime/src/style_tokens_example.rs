//! Jaime's standalone semantic design-token example.
//!
//! The page installs a token provider locally because the shared showcase
//! currently provides the legacy `ThemeData` palette.

use aimer::style::{
    AnimatedTheme, TextStyle, Theme, ThemeTokenVariants, ThemeTokens, ThemeVariant,
};
use aimer::{
    AimerApp, AnyElement, BoxAlignment, BuildContext, Column, Container, Dimension, SizedBox, Text,
    Widget,
};

/// Starts the semantic token example as a standalone application.
pub fn start_style_tokens_example() {
    AimerApp::start(style_tokens_example());
}

/// Builds the semantic token example and provides animated light/dark tokens.
pub fn style_tokens_example() -> impl Widget {
    AnimatedTheme::new()
        .adaptive(ThemeTokens::light(), ThemeTokens::dark())
        .child(StyleTokensExample::new())
}

/// A small page showing semantic roles, state layers, density, contrast, and
/// high-contrast fallback behavior without importing a platform renderer.
pub struct StyleTokensExample {
    variants: ThemeTokenVariants,
}

impl StyleTokensExample {
    /// Creates a page using the built-in light/dark/high-contrast variants.
    pub fn new() -> Self {
        Self {
            variants: ThemeTokenVariants::default(),
        }
    }
}

impl Default for StyleTokensExample {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for StyleTokensExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let tokens = ThemeTokens::copied(ctx).normalized();
        let high_contrast = self
            .variants
            .resolve(ThemeVariant::HighContrast);
        let body = tokens
            .typography
            .body;
        let control_height = tokens
            .density
            .target_size(
                tokens
                    .control
                    .min_height,
            );
        let hover_surface = tokens
            .state
            .hover
            .apply(
                tokens
                    .colors
                    .surface,
            );
        let contrast = tokens
            .control
            .focus_ring
            .minimum_contrast;

        Container::new()
            .color(
                tokens
                    .colors
                    .background,
            )
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("Semantic design tokens")
                            .text_style(
                                TextStyle::new()
                                    .font_size(
                                        tokens
                                            .typography
                                            .title
                                            .font_size
                                            as u32,
                                    )
                                    .color(
                                        tokens
                                            .colors
                                            .on_background,
                                    ),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(
                                tokens
                                    .spacing
                                    .medium as u32,
                            )
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(320.0))
                            .height(Dimension::Px(control_height))
                            .color(hover_surface)
                            .child(
                                Text::new(format!(
                                    "body {:.0}px · gap {:.0}px · radius {:.0}px",
                                    body.font_size,
                                    tokens
                                        .spacing
                                        .medium,
                                    tokens.shape.medium,
                                ))
                                .text_style(
                                    TextStyle::new().color(
                                        tokens
                                            .colors
                                            .on_surface,
                                    ),
                                ),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(tokens.spacing.small as u32)
                            .boxed(),
                        Text::new(format!(
                            "focus contrast ≥ {:.1}: high-contrast primary {:?}",
                            contrast,
                            high_contrast
                                .colors
                                .primary,
                        ))
                        .text_style(
                            TextStyle::new().color(
                                tokens
                                    .colors
                                    .on_background,
                            ),
                        )
                        .boxed(),
                        SizedBox::new()
                            .height(tokens.spacing.small as u32)
                            .boxed(),
                        Text::new(format!(
                            "motion {:.0}ms · state opacity {:.2}",
                            tokens
                                .motion
                                .effective_duration_ms(aimer::style::MotionDuration::Standard,),
                            tokens
                                .state
                                .pressed
                                .opacity,
                        ))
                        .text_style(
                            TextStyle::new().color(
                                tokens
                                    .colors
                                    .on_background,
                            ),
                        )
                        .boxed(),
                    ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "StyleTokensExample"
    }
}

impl aimer::PortableWidget for StyleTokensExample {}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer::Color;

    #[test]
    fn example_exposes_a_widget_without_global_token_state() {
        fn assert_widget(_widget: impl Widget) {}

        assert_widget(style_tokens_example());
        assert_widget(StyleTokensExample::new());
    }

    #[test]
    fn example_variants_keep_the_high_contrast_fallback_visible() {
        let variants = ThemeTokenVariants::new(ThemeTokens::light());

        assert_eq!(
            variants.resolve(ThemeVariant::HighContrast),
            ThemeTokens::light().high_contrast_fallback()
        );
    }

    #[test]
    fn example_state_preview_uses_the_control_namespace() {
        let tokens = ThemeTokens::light();

        assert_eq!(
            tokens
                .control
                .focus_ring
                .minimum_contrast,
            tokens
                .focus
                .minimum_contrast
        );
        assert_eq!(
            tokens
                .state
                .hover
                .apply(
                    tokens
                        .colors
                        .surface
                ),
            Color::Rgb(255, 235, 235)
        );
    }
}
