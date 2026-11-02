use aimer_color::prelude::Color;
#[cfg(feature = "arborium")]
use arborium::advanced::Span;

/// A highlight capture category, paired with its byte span.
///
/// This is the backend-independent currency of [`crate::syntax`]: both the
/// default `synoptic` highlighter and the optional tree-sitter based
/// `arborium` one are translated into these categories, so the renderer never
/// has to know which one produced them.
///
/// The names follow the semantic categories tree-sitter grammars normalize
/// captures into (see `HIGHLIGHT_NAMES` / the HTML tag reference: <a-k>,
/// <a-f>, etc.), which `synoptic`'s simpler token names map onto cleanly.
///
/// NOTE: `Other(String)` exists because a capture name is a free-form string
/// in both backends — not every name a grammar or a custom rule might emit is
/// guaranteed to be covered above, so this variant preserves anything
/// unrecognized instead of dropping it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSpan {
    // --- Code ---
    Keyword {
        start: u32,
        end: u32,
    },
    Function {
        start: u32,
        end: u32,
    },
    String {
        start: u32,
        end: u32,
    },
    Comment {
        start: u32,
        end: u32,
    },
    Type {
        start: u32,
        end: u32,
    },
    Variable {
        start: u32,
        end: u32,
    },
    Constant {
        start: u32,
        end: u32,
    },
    Number {
        start: u32,
        end: u32,
    },
    Operator {
        start: u32,
        end: u32,
    },
    Punctuation {
        start: u32,
        end: u32,
    },
    Property {
        start: u32,
        end: u32,
    },
    Attribute {
        start: u32,
        end: u32,
    },
    Tag {
        start: u32,
        end: u32,
    },
    Macro {
        start: u32,
        end: u32,
    },
    Label {
        start: u32,
        end: u32,
    },
    Namespace {
        start: u32,
        end: u32,
    },
    Constructor {
        start: u32,
        end: u32,
    },

    // --- Markup (Markdown, AsciiDoc, etc.) ---
    Title {
        start: u32,
        end: u32,
    },
    Strong {
        start: u32,
        end: u32,
    },
    Emphasis {
        start: u32,
        end: u32,
    },
    Link {
        start: u32,
        end: u32,
    },
    Literal {
        start: u32,
        end: u32,
    },
    Strikethrough {
        start: u32,
        end: u32,
    },

    // --- Diff ---
    DiffAdd {
        start: u32,
        end: u32,
    },
    DiffDelete {
        start: u32,
        end: u32,
    },

    // --- Special ---
    Embedded {
        start: u32,
        end: u32,
    },
    Error {
        start: u32,
        end: u32,
    },

    // --- Fallback for anything not covered above ---
    Other {
        start: u32,
        end: u32,
        capture: String,
    },
}

impl CaptureSpan {
    /// Byte offsets, regardless of variant.
    pub fn range(&self) -> (u32, u32) {
        match self {
            CaptureSpan::Keyword { start, end }
            | CaptureSpan::Function { start, end }
            | CaptureSpan::String { start, end }
            | CaptureSpan::Comment { start, end }
            | CaptureSpan::Type { start, end }
            | CaptureSpan::Variable { start, end }
            | CaptureSpan::Constant { start, end }
            | CaptureSpan::Number { start, end }
            | CaptureSpan::Operator { start, end }
            | CaptureSpan::Punctuation { start, end }
            | CaptureSpan::Property { start, end }
            | CaptureSpan::Attribute { start, end }
            | CaptureSpan::Tag { start, end }
            | CaptureSpan::Macro { start, end }
            | CaptureSpan::Label { start, end }
            | CaptureSpan::Namespace { start, end }
            | CaptureSpan::Constructor { start, end }
            | CaptureSpan::Title { start, end }
            | CaptureSpan::Strong { start, end }
            | CaptureSpan::Emphasis { start, end }
            | CaptureSpan::Link { start, end }
            | CaptureSpan::Literal { start, end }
            | CaptureSpan::Strikethrough { start, end }
            | CaptureSpan::DiffAdd { start, end }
            | CaptureSpan::DiffDelete { start, end }
            | CaptureSpan::Embedded { start, end }
            | CaptureSpan::Error { start, end } => (*start, *end),
            CaptureSpan::Other { start, end, .. } => (*start, *end),
        }
    }

    /// Build a `CaptureSpan` from a raw capture name and its byte range.
    ///
    /// This is the single translation table shared by every highlighting
    /// backend. It matches on the dotted-prefix convention tree-sitter
    /// grammars use (e.g. `"keyword.function"` still maps to
    /// [`CaptureSpan::Keyword`]) as well as on the flat token names
    /// `synoptic`'s regex rules emit (e.g. `"digit"`, `"struct"`,
    /// `"heading"`), falling back to [`CaptureSpan::Other`] for anything
    /// unrecognized rather than dropping the span.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(
    ///     CaptureSpan::from_capture("keyword.function", 0, 2),
    ///     CaptureSpan::Keyword { start: 0, end: 2 }
    /// );
    /// ```
    pub fn from_capture(capture: &str, start: u32, end: u32) -> Self {
        let mut parts = capture.split('.');
        let base = parts.next().unwrap_or(capture);
        let other = || CaptureSpan::Other {
            start,
            end,
            capture: capture.to_string(),
        };

        match base {
            "keyword" | "kw" | "include" | "conditional" | "repeat" => {
                CaptureSpan::Keyword { start, end }
            }
            "function" | "method" => CaptureSpan::Function { start, end },
            "string" | "character" => CaptureSpan::String { start, end },
            "comment" => CaptureSpan::Comment { start, end },
            "type" | "struct" => CaptureSpan::Type { start, end },
            "variable" | "parameter" | "reference" => CaptureSpan::Variable { start, end },
            "constant" | "boolean" => CaptureSpan::Constant { start, end },
            "number" | "float" | "digit" => CaptureSpan::Number { start, end },
            "operator" => CaptureSpan::Operator { start, end },
            "punctuation" => CaptureSpan::Punctuation { start, end },
            "property" | "field" | "key" => CaptureSpan::Property { start, end },
            "attribute" | "annotation" => CaptureSpan::Attribute { start, end },
            "tag" => CaptureSpan::Tag { start, end },
            "macro" => CaptureSpan::Macro { start, end },
            "label" => CaptureSpan::Label { start, end },
            "namespace" | "module" => CaptureSpan::Namespace { start, end },
            "constructor" => CaptureSpan::Constructor { start, end },
            "heading" | "header" | "title" => CaptureSpan::Title { start, end },
            "bold" | "strong" => CaptureSpan::Strong { start, end },
            "italic" | "emphasis" => CaptureSpan::Emphasis { start, end },
            "link" | "image" => CaptureSpan::Link { start, end },
            "block" | "literal" => CaptureSpan::Literal { start, end },
            "strikethrough" => CaptureSpan::Strikethrough { start, end },
            "insertion" => CaptureSpan::DiffAdd { start, end },
            "deletion" => CaptureSpan::DiffDelete { start, end },
            "markup" => match parts.next() {
                Some("heading" | "title") => CaptureSpan::Title { start, end },
                Some("bold" | "strong") => CaptureSpan::Strong { start, end },
                Some("italic" | "emphasis") => CaptureSpan::Emphasis { start, end },
                Some("link") => CaptureSpan::Link { start, end },
                Some("raw" | "literal") => CaptureSpan::Literal { start, end },
                Some("strikethrough") => CaptureSpan::Strikethrough { start, end },
                _ => other(),
            },
            "diff" => match parts.next() {
                Some("plus" | "add") => CaptureSpan::DiffAdd { start, end },
                Some("minus" | "delete") => CaptureSpan::DiffDelete { start, end },
                _ => other(),
            },
            "embedded" => CaptureSpan::Embedded { start, end },
            "error" => CaptureSpan::Error { start, end },
            _ => other(),
        }
    }

    /// Build a `CaptureSpan` from arborium_highlight's raw `Span`.
    #[cfg(feature = "arborium")]
    #[inline]
    pub fn from_raw(span: &Span) -> Self {
        CaptureSpan::from_capture(&span.capture, span.start, span.end)
    }

    pub fn color(&self) -> Color {
        match self {
            // --- Code ---
            CaptureSpan::Keyword { .. } => Color::Rgb(198, 120, 221), // purple
            CaptureSpan::Function { .. } => Color::Rgb(97, 175, 239), // blue
            CaptureSpan::String { .. } => Color::Rgb(152, 195, 121),  // green
            CaptureSpan::Comment { .. } => Color::Rgb(92, 99, 112),   // muted gray
            CaptureSpan::Type { .. } => Color::Rgb(229, 192, 123),    // yellow/gold
            CaptureSpan::Variable { .. } => Color::Rgb(224, 108, 117), // soft red
            CaptureSpan::Constant { .. } => Color::Rgb(209, 154, 102), // orange
            CaptureSpan::Number { .. } => Color::Rgb(209, 154, 102),  // orange
            CaptureSpan::Operator { .. } => Color::Rgb(86, 182, 194), // cyan
            CaptureSpan::Punctuation { .. } => Color::Rgb(171, 178, 191), // light gray
            CaptureSpan::Property { .. } => Color::Rgb(224, 108, 117), // soft red
            CaptureSpan::Attribute { .. } => Color::Rgb(209, 154, 102), // orange
            CaptureSpan::Tag { .. } => Color::Rgb(224, 108, 117),     // soft red
            CaptureSpan::Macro { .. } => Color::Rgb(198, 120, 221),   // purple
            CaptureSpan::Label { .. } => Color::Rgb(198, 120, 221),   // purple
            CaptureSpan::Namespace { .. } => Color::Rgb(229, 192, 123), // yellow/gold
            CaptureSpan::Constructor { .. } => Color::Rgb(97, 175, 239), // blue

            // --- Markup ---
            CaptureSpan::Title { .. } => Color::Rgb(224, 108, 117), // soft red
            CaptureSpan::Strong { .. } => Color::Rgb(229, 192, 123), // yellow/gold
            CaptureSpan::Emphasis { .. } => Color::Rgb(198, 120, 221), // purple
            CaptureSpan::Link { .. } => Color::Rgb(97, 175, 239),   // blue
            CaptureSpan::Literal { .. } => Color::Rgb(152, 195, 121), // green
            CaptureSpan::Strikethrough { .. } => Color::Rgb(92, 99, 112), // muted gray

            // --- Diff ---
            CaptureSpan::DiffAdd { .. } => Color::Rgb(152, 195, 121), // green
            CaptureSpan::DiffDelete { .. } => Color::Rgb(224, 108, 117), // soft red

            // --- Special ---
            CaptureSpan::Embedded { .. } => Color::Rgb(171, 178, 191), // light gray
            CaptureSpan::Error { .. } => Color::Rgb(224, 108, 117),    // soft red (bold-worthy)

            // --- Fallback ---
            CaptureSpan::Other { .. } => Color::Rgb(171, 178, 191), // light gray, neutral default
        }
    }
}

#[cfg(feature = "arborium")]
impl From<Span> for CaptureSpan {
    #[inline]
    fn from(value: Span) -> Self {
        CaptureSpan::from_raw(&value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_tree_sitter_captures_keep_their_base_category() {
        assert_eq!(
            CaptureSpan::from_capture("keyword.function", 0, 2),
            CaptureSpan::Keyword { start: 0, end: 2 }
        );
        assert_eq!(
            CaptureSpan::from_capture("markup.heading.1", 0, 3),
            CaptureSpan::Title { start: 0, end: 3 }
        );
        assert_eq!(
            CaptureSpan::from_capture("diff.plus", 0, 1),
            CaptureSpan::DiffAdd { start: 0, end: 1 }
        );
    }

    #[test]
    fn flat_synoptic_token_names_map_onto_the_same_categories() {
        assert_eq!(
            CaptureSpan::from_capture("digit", 0, 1),
            CaptureSpan::Number { start: 0, end: 1 }
        );
        assert_eq!(
            CaptureSpan::from_capture("struct", 0, 3),
            CaptureSpan::Type { start: 0, end: 3 }
        );
        assert_eq!(
            CaptureSpan::from_capture("boolean", 0, 4),
            CaptureSpan::Constant { start: 0, end: 4 }
        );
        assert_eq!(
            CaptureSpan::from_capture("insertion", 0, 4),
            CaptureSpan::DiffAdd { start: 0, end: 4 }
        );
    }

    #[test]
    fn unknown_captures_are_preserved_rather_than_dropped() {
        assert_eq!(
            CaptureSpan::from_capture("tumbleweed", 1, 2),
            CaptureSpan::Other {
                start: 1,
                end: 2,
                capture: "tumbleweed".to_string(),
            }
        );
    }

    #[test]
    fn every_variant_reports_its_byte_range() {
        assert_eq!(CaptureSpan::from_capture("keyword", 3, 9).range(), (3, 9));
        assert_eq!(CaptureSpan::from_capture("nope", 3, 9).range(), (3, 9));
    }
}
