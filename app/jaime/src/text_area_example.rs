//! A focused showcase for Aimer's multiline text editor.

use aimer::input::input::{SpanStyle, StyledRange, TextHighlighter};
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
    code: TextEditingController,
    code_highlighter: TextHighlighter,
    message: TextEditingController,
    mention_highlighter: TextHighlighter,
}

/// Colours Rust-like tokens: keywords, strings, numbers and line comments.
fn code_ranges(text: &str) -> Vec<StyledRange> {
    const KEYWORDS: [&str; 9] = ["fn", "let", "mut", "pub", "struct", "impl", "if", "else", "return"];
    let keyword = SpanStyle::new().color(Color::Rgba(198, 120, 221, 255));
    let string = SpanStyle::new().color(Color::Rgba(152, 195, 121, 255));
    let number = SpanStyle::new().color(Color::Rgba(209, 154, 102, 255));
    let comment = SpanStyle::new().color(Color::Rgba(127, 132, 142, 255));
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &text[index..];
        let character = rest.chars().next().unwrap_or(' ');
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(text.len(), |line| index + line);
            ranges.push(StyledRange::new(index..end, comment));
            index = end;
        } else if character == '"' {
            let end = rest[1..].find('"').map_or(text.len(), |close| index + close + 2);
            ranges.push(StyledRange::new(index..end, string));
            index = end;
        } else if character.is_ascii_digit() {
            let length = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            ranges.push(StyledRange::new(index..index + length, number));
            index += length;
        } else if character.is_alphabetic() || character == '_' {
            let length = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if KEYWORDS.contains(&&rest[..length]) {
                ranges.push(StyledRange::new(index..index + length, keyword));
            }
            index += length;
        } else {
            index += character.len_utf8();
        }
    }
    ranges
}

/// Marks every `@name` with a tinted pill-like background.
fn mention_ranges(text: &str) -> Vec<StyledRange> {
    let style = SpanStyle::new()
        .color(Color::Rgba(30, 90, 220, 255))
        .background_color(Color::Rgba(30, 90, 220, 36));
    text.match_indices('@')
        .map(|(start, _)| {
            let length = text[start + 1..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(text.len() - start - 1);
            StyledRange::new(start..start + 1 + length, style)
        })
        .filter(|range| range.range.len() > 1)
        .collect()
}

impl TextAreaExample {
    /// Creates the showcase with editable multiline content.
    #[inline]
    pub fn new() -> Self {
        Self {
            controller: TextEditingController::with_text(
                "Write across multiple lines.\nLong lines wrap within the available width.",
            ),
            code: TextEditingController::with_text(
                "// Highlighted by a TextHighlighter\npub fn answer(mut n: u32) -> u32 {\n    let label = \"the answer\";\n    if n > 41 { return 42; }\n    n + 1\n}",
            ),
            code_highlighter: TextHighlighter::new(code_ranges),
            message: TextEditingController::with_text(
                "Thanks @alice and @bob_2 - can @carol review this?",
            ),
            mention_highlighter: TextHighlighter::new(mention_ranges),
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
        let code_font = crate::custom_font::register_custom_font()
            .expect("the embedded JetBrains Mono font should be valid");

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
                    TextArea::new()
                        .controller(self.code.clone())
                        .highlighter(self.code_highlighter.clone())
                        .text_style(
                            text_style
                                .font_family(code_font)
                                .color(theme.on_surface_color),
                        )
                        .min_lines(4)
                        .max_lines(Some(10))
                        .decoration(theme::input_decoration(&theme))
                        .focus_decoration(theme::input_focus_decoration(&theme))
                        .cursor_color(theme::input_cursor_color(&theme))
                        .selection_color(theme.primary_color.with_alpha(0.28))
                        .padding(LayoutSpacing::all(Spacing::Px(12)))
                        .boxed(),
                    TextArea::new()
                        .controller(self.message.clone())
                        .highlighter(self.mention_highlighter.clone())
                        .text_style(text_style.color(theme.on_surface_color))
                        .min_lines(2)
                        .max_lines(Some(6))
                        .decoration(theme::input_decoration(&theme))
                        .focus_decoration(theme::input_focus_decoration(&theme))
                        .cursor_color(theme::input_cursor_color(&theme))
                        .selection_color(theme.primary_color.with_alpha(0.28))
                        .padding(LayoutSpacing::all(Spacing::Px(12)))
                        .boxed(),
                ]))
    }
}
