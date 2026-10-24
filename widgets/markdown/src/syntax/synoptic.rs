//! The default, lightweight highlighting backend.
//!
//! [`synoptic`](https://docs.rs/synoptic) is a regex based highlighter with a
//! handful of dependencies, which keeps the markdown crate cheap to build. It
//! understands code less deeply than the tree-sitter backed `arborium` backend
//! (enable the `arborium` feature for that), but it is more than enough for the
//! fenced code blocks a document renders.

use ::synoptic::{Highlighter, TokOpt, from_extension};

use super::CaptureSpan;

/// Tabs are expanded to a single space so the byte offsets `synoptic` reports
/// stay identical to the ones in the original source, which is what the
/// renderer slices the code with.
const TAB_WIDTH: usize = 1;

/// The fenced-code languages the backend understands, mapped onto the file
/// extension `synoptic`'s built-in rule sets are keyed by.
///
/// Comparisons are ASCII case-insensitive, so `Rust`, `rust` and `RS` all
/// resolve to the same rules. A language that is absent here yields no spans
/// at all, which renders the block as plain text.
const LANGUAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("rust", "rs"),
    ("rs", "rs"),
    ("python", "py"),
    ("py", "py"),
    ("ruby", "rb"),
    ("rb", "rb"),
    ("perl", "pm"),
    ("pm", "pm"),
    ("lua", "lua"),
    ("r", "r"),
    ("go", "go"),
    ("golang", "go"),
    ("javascript", "js"),
    ("js", "js"),
    ("jsx", "js"),
    ("typescript", "ts"),
    ("ts", "ts"),
    ("tsx", "ts"),
    ("dart", "dart"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("c++", "cpp"),
    ("cxx", "cpp"),
    ("cc", "cpp"),
    ("hpp", "cpp"),
    ("csharp", "cs"),
    ("cs", "cs"),
    ("c#", "cs"),
    ("swift", "swift"),
    ("json", "json"),
    ("kotlin", "kt"),
    ("kt", "kt"),
    ("java", "java"),
    ("vb", "vb"),
    ("visualbasic", "vb"),
    ("matlab", "m"),
    ("php", "php"),
    ("scala", "scala"),
    ("prolog", "prolog"),
    ("haskell", "hs"),
    ("hs", "hs"),
    ("css", "css"),
    ("html", "html"),
    ("htm", "html"),
    ("xhtml", "html"),
    ("markdown", "md"),
    ("md", "md"),
    ("toml", "toml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("csv", "csv"),
    ("shell", "sh"),
    ("sh", "sh"),
    ("bash", "sh"),
    ("zsh", "sh"),
    ("sql", "sql"),
    ("xml", "xml"),
    ("nushell", "nu"),
    ("nu", "nu"),
    ("tex", "tex"),
    ("latex", "tex"),
    ("assembly", "asm"),
    ("asm", "asm"),
    ("diff", "diff"),
    ("patch", "diff"),
];

/// The extension `synoptic` keys its built-in rules by, for a fenced-code
/// language tag, or `None` when the language is not supported.
fn extension_of(language: &str) -> Option<&'static str> {
    LANGUAGE_EXTENSIONS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(language))
        .map(|(_, extension)| *extension)
}

/// The default highlighting backend.
///
/// `synoptic`'s highlighters are line based and carry per-run state, so a fresh
/// one is built for every parse. Results are cached by [`crate::syntax`], so
/// this only happens on a cache miss.
pub(crate) struct SynopticBackend;

impl SynopticBackend {
    #[inline]
    pub(crate) fn new() -> Self {
        Self
    }

    /// Highlight `code`, returning the captures of the supported `language`, or
    /// nothing at all when the language is unknown or absent.
    pub(crate) fn highlight(&mut self, code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
        let Some(extension) = language.and_then(extension_of) else {
            return Vec::new();
        };
        let Some(mut highlighter) = from_extension(extension, TAB_WIDTH) else {
            return Vec::new();
        };

        let lines: Vec<String> = code.split('\n').map(str::to_string).collect();
        highlighter.run(&lines);
        collect_spans(&highlighter, &lines)
    }
}

/// Walk the highlighted lines, turning `synoptic`'s per-line text chunks back
/// into byte spans over the original `code`.
///
/// Each line restarts from its own offset, so a rule that reports a chunk of an
/// unexpected width can never shift the spans of the lines below it.
fn collect_spans(highlighter: &Highlighter, lines: &[String]) -> Vec<CaptureSpan> {
    let mut spans = Vec::new();
    let mut line_start = 0u32;

    for (index, line) in lines.iter().enumerate() {
        let mut offset = line_start;
        for token in highlighter.line(index, line) {
            match token {
                TokOpt::Some(text, kind) => {
                    let end = offset + text.len() as u32;
                    spans.push(CaptureSpan::from_capture(&kind, offset, end));
                    offset = end;
                }
                TokOpt::None(text) => offset += text.len() as u32,
            }
        }
        // `+ 1` for the newline `split` consumed.
        line_start += line.len() as u32 + 1;
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight(code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
        SynopticBackend::new().highlight(code, language)
    }

    #[test]
    fn language_tags_and_extensions_resolve_to_the_same_rules() {
        assert_eq!(extension_of("rust"), Some("rs"));
        assert_eq!(extension_of("Rust"), Some("rs"));
        assert_eq!(extension_of("rs"), Some("rs"));
        assert_eq!(extension_of("py"), Some("py"));
        assert_eq!(extension_of("c++"), Some("cpp"));
        assert_eq!(extension_of("unknown"), None);
    }

    #[test]
    fn highlights_rust_keywords_at_their_byte_offsets() {
        let spans = highlight("fn main() {}", Some("rust"));

        assert!(
            spans.contains(&CaptureSpan::Keyword { start: 0, end: 2 }),
            "{spans:?}"
        );
    }

    #[test]
    fn spans_of_later_lines_are_offset_by_the_lines_above() {
        let spans = highlight("let a = 1;\nlet b = 2;", Some("rust"));

        let keywords: Vec<(u32, u32)> = spans
            .iter()
            .filter(|span| matches!(span, CaptureSpan::Keyword { .. }))
            .map(CaptureSpan::range)
            .collect();
        assert_eq!(keywords, vec![(0, 3), (11, 14)], "{spans:?}");
    }

    #[test]
    fn spans_never_run_past_the_end_of_the_code() {
        let code = "# heading\n\nsome *text* here\n";
        for span in highlight(code, Some("markdown")) {
            let (start, end) = span.range();
            assert!(start <= end, "{span:?}");
            assert!(
                end as usize <= code.len(),
                "{span:?} in {} bytes",
                code.len()
            );
        }
    }

    #[test]
    fn returns_no_captures_without_a_supported_language() {
        assert!(highlight("plain text", None).is_empty());
        assert!(highlight("plain text", Some("unknown")).is_empty());
    }
}
