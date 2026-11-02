//! The optional, tree-sitter backed highlighting backend.
//!
//! [`arborium`](https://docs.rs/arborium) parses the code with a real grammar,
//! which gives noticeably better captures than the default `synoptic` backend
//! at the cost of pulling a tree-sitter grammar in per language. It is only
//! compiled when the `arborium` feature is enabled.

use ::arborium::Highlighter;

use super::CaptureSpan;

/// The tree-sitter backed highlighting backend.
///
/// The [`Highlighter`] owns the loaded grammars, so it is kept alive across
/// parses instead of being rebuilt for every code block.
pub(crate) struct ArboriumBackend {
    highlighter: Highlighter,
}

impl ArboriumBackend {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            highlighter: Highlighter::new(),
        }
    }

    /// Highlight `code`, returning the captures of the supported `language`, or
    /// nothing at all when the language is unknown or absent.
    pub(crate) fn highlight(&mut self, code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
        let Some(language) = language.map(str::to_ascii_lowercase) else {
            return Vec::new();
        };
        let language = match language.as_str() {
            "py" => "python",
            "rs" => "rust",
            "js" => "javascript",
            "ts" => "typescript",
            language => language,
        };

        self.highlighter
            .highlight_spans(language, code)
            .map(|spans| spans.into_iter().map(CaptureSpan::from).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight(code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
        ArboriumBackend::new().highlight(code, language)
    }

    #[test]
    fn highlights_rust_with_capture_spans() {
        assert_eq!(
            highlight("fn main(){}", Some("rust")),
            vec![
                CaptureSpan::Keyword { start: 0, end: 2 },
                CaptureSpan::Function { start: 3, end: 7 },
                CaptureSpan::Punctuation { start: 7, end: 8 },
                CaptureSpan::Punctuation { start: 8, end: 9 },
                CaptureSpan::Punctuation { start: 9, end: 10 },
                CaptureSpan::Punctuation { start: 10, end: 11 },
            ]
        );
    }

    #[test]
    fn supports_languages_and_aliases_provided_by_arborium() {
        let toml = highlight("edition = \"2024\"", Some("toml"));
        assert!(
            toml.iter()
                .any(|span| matches!(span, CaptureSpan::String { .. }))
        );
        assert!(
            highlight("def main(): pass", Some("py"))
                .iter()
                .any(|span| matches!(span, CaptureSpan::Keyword { start: 0, end: 3 }))
        );
    }

    #[test]
    fn returns_no_captures_without_a_supported_language() {
        assert!(highlight("plain text", None).is_empty());
        assert!(highlight("plain text", Some("unknown")).is_empty());
    }
}
