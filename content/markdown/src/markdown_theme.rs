use std::sync::OnceLock;

use aimer_assets::{FontFamily, FontRegistration, FontRegistry, FontStyle, FontWeight};
use aimer_color::prelude::Color;
use aimer_style::{TextDecoration, TextStyle};
use aimer_text::SpanStyle;

const CODE_FONT_FAMILY_NAME: &str = "Aimer Markdown JetBrains Mono";
const CODE_FONT_BYTES: &[u8] =
    include_bytes!("../JetBrainsMono-2.304/fonts/ttf/JetBrainsMono-Regular.ttf");

fn code_font_family() -> FontFamily {
    static FAMILY: OnceLock<FontFamily> = OnceLock::new();
    *FAMILY.get_or_init(|| {
        FontRegistry::family(CODE_FONT_FAMILY_NAME).unwrap_or_else(|| {
            FontRegistry::register(FontRegistration {
                family: CODE_FONT_FAMILY_NAME,
                bytes: CODE_FONT_BYTES,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            })
            .expect("the embedded Markdown code font should be valid")
        })
    })
}

#[derive(Clone, Copy, Debug)]
/// Visual styles, colors, and spacing used by [`crate::MarkdownViewer`].
pub struct MarkdownTheme {
    pub icon_color: Color,
    pub body: TextStyle,
    pub headings: [TextStyle; 6],
    pub blockquote: TextStyle,
    pub code_block: TextStyle,
    pub inline_code: SpanStyle,
    pub link: SpanStyle,
    pub link_hover_color: Color,
    pub code_background: Color,
    pub quote_background: Color,
    pub rule_color: Color,
    pub table_header_background: Color,
    pub table_cell_background: Color,
    pub keyword_color: Color,
    pub string_color: Color,
    pub comment_color: Color,
    pub number_color: Color,
    pub block_spacing: u32,
}

impl MarkdownTheme {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn body(mut self, style: TextStyle) -> Self {
        self.body = style;
        self
    }

    /// Set an individual heading level's style (0 = h1 ... 5 = h6).
    pub fn heading(mut self, level: usize, style: TextStyle) -> Self {
        assert!(level < 6, "heading level must be between 0 and 5");
        self.headings[level] = style;
        self
    }

    /// Replace all heading styles at once.
    pub fn headings(mut self, headings: [TextStyle; 6]) -> Self {
        self.headings = headings;
        self
    }

    pub fn h1(mut self, style: TextStyle) -> Self {
        self.headings[0] = style;
        self
    }

    pub fn h2(mut self, style: TextStyle) -> Self {
        self.headings[1] = style;
        self
    }

    pub fn h3(mut self, style: TextStyle) -> Self {
        self.headings[2] = style;
        self
    }

    pub fn h4(mut self, style: TextStyle) -> Self {
        self.headings[3] = style;
        self
    }

    pub fn h5(mut self, style: TextStyle) -> Self {
        self.headings[4] = style;
        self
    }

    pub fn h6(mut self, style: TextStyle) -> Self {
        self.headings[5] = style;
        self
    }

    pub fn blockquote(mut self, style: TextStyle) -> Self {
        self.blockquote = style;
        self
    }

    pub fn code_block(mut self, style: TextStyle) -> Self {
        self.code_block = style;
        self
    }

    pub fn inline_code(mut self, style: SpanStyle) -> Self {
        self.inline_code = style;
        self
    }

    pub fn link(mut self, style: SpanStyle) -> Self {
        self.link = style;
        self
    }

    pub fn link_hover_color(mut self, color: impl Into<Color>) -> Self {
        self.link_hover_color = color.into();
        self
    }

    pub fn code_background(mut self, color: impl Into<Color>) -> Self {
        self.code_background = color.into();
        self
    }

    pub fn quote_background(mut self, color: impl Into<Color>) -> Self {
        self.quote_background = color.into();
        self
    }

    pub fn rule_color(mut self, color: impl Into<Color>) -> Self {
        self.rule_color = color.into();
        self
    }

    pub fn table_header_background(mut self, color: impl Into<Color>) -> Self {
        self.table_header_background = color.into();
        self
    }

    pub fn table_cell_background(mut self, color: impl Into<Color>) -> Self {
        self.table_cell_background = color.into();
        self
    }

    pub fn keyword_color(mut self, color: impl Into<Color>) -> Self {
        self.keyword_color = color.into();
        self
    }

    pub fn string_color(mut self, color: impl Into<Color>) -> Self {
        self.string_color = color.into();
        self
    }

    pub fn comment_color(mut self, color: impl Into<Color>) -> Self {
        self.comment_color = color.into();
        self
    }

    pub fn number_color(mut self, color: impl Into<Color>) -> Self {
        self.number_color = color.into();
        self
    }

    pub fn block_spacing(mut self, spacing: u32) -> Self {
        self.block_spacing = spacing;
        self
    }
}

impl Default for MarkdownTheme {
    fn default() -> Self {
        let body = TextStyle::new().font_size(16).color(Color::Hex(0x24292F));
        let code_font = code_font_family();
        Self {
            body,
            headings: [
                body.font_size(32).font_weight(FontWeight::Bolder),
                body.font_size(28).font_weight(FontWeight::Bolder),
                body.font_size(24).font_weight(FontWeight::Bolder),
                body.font_size(20).font_weight(FontWeight::Bolder),
                body.font_size(18).font_weight(FontWeight::Bolder),
                body.font_size(16).font_weight(FontWeight::Bolder),
            ],
            blockquote: body.color(Color::Hex(0x57606A)),
            code_block: body.font_family(code_font).font_size(14),
            inline_code: SpanStyle::new()
                .font_family(code_font)
                .background_color(Color::Hex(0xEFF1F3)),
            link: SpanStyle::new()
                .color(Color::Hex(0x0969DA))
                .text_decoration(TextDecoration::Underline),
            link_hover_color: Color::Hex(0x0969DA).lighten(0.48),
            code_background: Color::Hex(0xF6F8FA),
            quote_background: Color::Hex(0xF6F8FA),
            rule_color: Color::Hex(0xD0D7DE),
            table_header_background: Color::Hex(0xF6F8FA),
            table_cell_background: Color::Hex(0xFFFFFF),
            keyword_color: Color::Hex(0xCF222E),
            string_color: Color::Hex(0x0A3069),
            comment_color: Color::Hex(0x6E7781),
            number_color: Color::Hex(0x0550AE),
            block_spacing: 8,
            icon_color: Color::Hex(0x0969DA),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MarkdownTheme;
    use aimer_assets::{FontRegistry, FontStyle, FontWeight};

    #[test]
    fn default_code_styles_use_the_markdown_code_font() {
        let theme = MarkdownTheme::default();
        let family = FontRegistry::family(super::CODE_FONT_FAMILY_NAME)
            .expect("the Markdown code font should be registered");

        assert_eq!(theme.code_block.font_family, family);
        assert_eq!(theme.inline_code.font_family, Some(family));

        let face = FontRegistry::resolve(family, FontWeight::Normal, FontStyle::Normal)
            .expect("the Markdown code font face should be registered");
        assert_eq!(face.bytes.as_ref(), super::CODE_FONT_BYTES);
    }
}
