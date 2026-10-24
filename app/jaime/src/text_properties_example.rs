use aimer::style::{
    FontWeight, LayoutSpacing, LineHeight, Spacing, TextAlign, TextDecoration, TextDecorationLine,
    TextOverflow, TextShadow, TextStyle, TextTransform,
};
use aimer::{
    AimerApp, AnyWidget, Column, Container, RichText, ScrollAxis, Scrollable, SpanStyle, Text,
    TextSpan, Widget,
};

use crate::theme;

const SOURCE_SAMPLE: &str = "Straße / café\u{301} / 你好 / សួស្តី — mixed CASE and spaces\nA second line keeps wrapping visible.";
const PARAGRAPH_SAMPLE: &str = "One paragraph has enough words to wrap onto several lines. Its line boxes, first-line indent, and word gaps are easier to compare when the source stays the same.";

/// Starts the public text-properties showcase used for manual visual checks.
pub fn start_text_properties_example() {
    AimerApp::start(theme::provide(text_properties_example()));
}

/// Builds a constrained, scrollable page covering every supported text property.
pub fn text_properties_example() -> impl Widget {
    let app_theme = theme::app_theme();
    let base = TextStyle::new()
        .font_size(16)
        .font_weight(FontWeight::Normal)
        .color(app_theme.on_background_color)
        .text_overflow(TextOverflow::Wrap);
    let shadow = TextShadow::new()
        .offset_x(3.0)
        .offset_y(3.0)
        .blur(2.0)
        .color(
            app_theme
                .background_color
                .with_alpha(0.85),
        );

    let content = Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(16)))
        .children([
            heading("Text properties"),
            explanation(
                "All samples use the public TextStyle, Text, RichText, and TextSpan builders. Spacing and indentation are logical pixels; line-height is either natural, an absolute pixel value, or a font-size factor.",
            ),
            section(
                "Baseline",
                "Default style for comparison",
                Text::new(SOURCE_SAMPLE).text_style(base).boxed(),
            ),
            transform_samples(base),
            line_height_samples(base),
            spacing_samples(base),
            indent_samples(base),
            section(
                "text-shadow",
                "Glyph paint follows the text; the shadow does not enlarge layout.",
                Text::new("Shadowed glyphs stay aligned with the unshadowed baseline.")
                    .text_style(base)
                    .text_shadow(shadow)
                    .boxed(),
            ),
            rich_text_sample(base, shadow),
        ])
        .boxed();

    Container::new()
        // .color(app_theme.background_color)
        // .padding(LayoutSpacing::all(Spacing::Px(24)))
        .child(
            Scrollable::new()
                .axis(ScrollAxis::Vertical)
                .child(Container::new().child(content)),
        )
}

fn heading(text: &'static str) -> AnyWidget {
    let app_theme = theme::app_theme();

    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(28)
                .font_weight(FontWeight::Bold)
                .color(app_theme.on_background_color),
        )
        .boxed()
}

fn explanation(text: &'static str) -> AnyWidget {
    let app_theme = theme::app_theme();

    Text::new(text)
        .text_style(
            TextStyle::new()
                .font_size(15)
                .color(theme::muted_text(&app_theme)),
        )
        .boxed()
}

fn section(label: &'static str, description: &'static str, sample: AnyWidget) -> AnyWidget {
    let app_theme = theme::app_theme();

    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(6)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(18)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_background_color),
                )
                .boxed(),
            Text::new(description)
                .text_style(
                    TextStyle::new()
                        .font_size(13)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
            Container::new()
                .color(theme::raised_surface(&app_theme))
                .padding(LayoutSpacing::all(Spacing::Px(12)))
                .child(sample)
                .boxed(),
        ])
        .boxed()
}

fn labeled_sample(label: &'static str, sample: AnyWidget) -> AnyWidget {
    let app_theme = theme::app_theme();

    Column::new()
        .gaps(LayoutSpacing::all(Spacing::Px(4)))
        .children([
            Text::new(label)
                .text_style(
                    TextStyle::new()
                        .font_size(13)
                        .color(theme::muted_text(&app_theme)),
                )
                .boxed(),
            Container::new()
                .color(theme::raised_surface(&app_theme))
                .padding(LayoutSpacing::all(Spacing::Px(10)))
                .child(sample)
                .boxed(),
        ])
        .boxed()
}

fn transform_samples(base: TextStyle) -> AnyWidget {
    section(
        "text-transform",
        "Uppercase expands Straße to STRASSE while selection remains source-based; lowercase and capitalize use the same source.",
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(10)))
            .children([
                labeled_sample(
                    "None",
                    Text::new(SOURCE_SAMPLE)
                        .text_style(base)
                        .boxed(),
                ),
                labeled_sample(
                    "Uppercase",
                    Text::new(SOURCE_SAMPLE)
                        .text_style(base)
                        .text_transform(TextTransform::Uppercase)
                        .boxed(),
                ),
                labeled_sample(
                    "Lowercase",
                    Text::new(SOURCE_SAMPLE)
                        .text_style(base)
                        .text_transform(TextTransform::Lowercase)
                        .boxed(),
                ),
                labeled_sample(
                    "Capitalize",
                    Text::new(SOURCE_SAMPLE)
                        .text_style(base)
                        .text_transform(TextTransform::Capitalize)
                        .boxed(),
                ),
            ])
            .boxed(),
    )
}

fn line_height_samples(base: TextStyle) -> AnyWidget {
    section(
        "line-height",
        "Natural metrics, 32 logical pixels, and a 1.8 font-size factor change baseline distance without changing glyph size.",
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(10)))
            .children([
                labeled_sample(
                    "Normal",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .line_height(LineHeight::Normal)
                        .boxed(),
                ),
                labeled_sample(
                    "Px(32)",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .line_height(LineHeight::Px(32.0))
                        .boxed(),
                ),
                labeled_sample(
                    "Factor(1.8)",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .line_height(LineHeight::Factor(1.8))
                        .boxed(),
                ),
            ])
            .boxed(),
    )
}

fn spacing_samples(base: TextStyle) -> AnyWidget {
    section(
        "letter-spacing and word-spacing",
        "Letter spacing changes every rendered gap; word spacing changes whitespace separators, including the repeated spaces in this sample.",
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(10)))
            .children([
                labeled_sample(
                    "0.0 / 0.0",
                    Text::new("Spacing  between words, punctuation, and café\u{301}.")
                        .text_style(base)
                        .boxed(),
                ),
                labeled_sample(
                    "letter 1.0 / word 0.0",
                    Text::new("Spacing  between words, punctuation, and café\u{301}.")
                        .text_style(base)
                        .letter_spacing(1.0)
                        .boxed(),
                ),
                labeled_sample(
                    "letter -0.35 / word 2.0",
                    Text::new("Spacing  between words, punctuation, and café\u{301}.")
                        .text_style(base)
                        .letter_spacing(-0.35)
                        .word_spacing(2.0)
                        .boxed(),
                ),
            ])
            .boxed(),
    )
}

fn indent_samples(base: TextStyle) -> AnyWidget {
    section(
        "text-indent",
        "Only the first line moves. The negative value is a hanging indent, and the centered case shows alignment after indentation.",
        Column::new()
            .gaps(LayoutSpacing::all(Spacing::Px(10)))
            .children([
                labeled_sample(
                    "0.0",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .boxed(),
                ),
                labeled_sample(
                    "positive 28px",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .text_indent(28.0)
                        .boxed(),
                ),
                labeled_sample(
                    "negative -18px, centered",
                    Text::new(PARAGRAPH_SAMPLE)
                        .text_style(base)
                        .text_indent(-18.0)
                        .text_align(TextAlign::TopCenter)
                        .boxed(),
                ),
            ])
            .boxed(),
    )
}

fn rich_text_sample(base: TextStyle, shadow: TextShadow) -> AnyWidget {
    let app_theme = theme::app_theme();
    let transformed = TextSpan::new(" transformed Straße ").style(
        SpanStyle::new()
            .text_transform(TextTransform::Uppercase)
            .letter_spacing(0.8)
            .text_decoration(TextDecoration::new().line(TextDecorationLine::UNDERLINE))
            .color(app_theme.primary_color),
    );
    let linked = TextSpan::new("selectable link")
        .style(
            SpanStyle::new()
                .word_spacing(2.0)
                .text_shadow(shadow)
                .color(
                    app_theme
                        .primary_color
                        .lighten(0.10),
                ),
        )
        .link("https://aimer.dev/text-properties");

    section(
        "RichText and selectable spans",
        "The base style supplies font attributes; the paragraph supplies line-height and indentation. Two spans override transformation, spacing, decoration, and shadow independently; the transformed span and link remain selectable by source ranges.",
        RichText::new(TextSpan::root([
            TextSpan::new("Base text with "),
            transformed,
            TextSpan::new(" plus a "),
            linked,
            TextSpan::new(". 你好 — សួស្តី\nA second source line keeps the link sample wrapped."),
        ]))
        .text_style(base)
        .line_height(LineHeight::Factor(1.5))
        .text_indent(18.0)
        .text_align(TextAlign::TopLeft)
        .wrapped()
        .selectable()
        .on_link(|target| println!("open {target}"))
        .boxed(),
    )
}

#[cfg(test)]
mod tests {
    use aimer::Widget;

    use super::text_properties_example;

    #[test]
    fn text_properties_example_builds_a_public_api_demo() {
        fn assert_widget(_: impl Widget) {}

        assert_widget(text_properties_example());
    }
}
