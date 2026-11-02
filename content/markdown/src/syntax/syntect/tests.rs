use std::fs;
use std::path::PathBuf;

use super::*;
use crate::syntax::HighlightCache;

fn highlight(code: &str, language: Option<&str>) -> Vec<CaptureSpan> {
    SyntectBackend::new().highlight(code, language)
}

fn has(spans: &[CaptureSpan], expected: CaptureSpan) -> bool {
    spans.contains(&expected)
}

/// A syntax whose only rule colors ALL-CAPS words, selected by `extension`.
fn shout_syntax(name: &str, extension: &str, scope: &str) -> String {
    format!(
        "%YAML 1.2\n---\nname: {name}\nfile_extensions: [{extension}]\nscope: source.{extension}\n\
         contexts:\n  main:\n    - match: '\\b[A-Z]+\\b'\n      scope: {scope}.{extension}\n"
    )
}

/// A fresh directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("aimer_markdown_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn highlights_rust_keywords_at_their_byte_offsets() {
    let spans = highlight("fn main() {}", Some("rust"));

    assert!(has(&spans, CaptureSpan::Keyword { start: 0, end: 2 }), "{spans:?}");
    assert!(has(&spans, CaptureSpan::Function { start: 3, end: 7 }), "{spans:?}");
}

#[test]
fn spans_of_later_lines_are_offset_by_the_lines_above() {
    let spans = highlight("fn a() {}\nfn b() {}", Some("rust"));

    let keywords: Vec<(u32, u32)> = spans
        .iter()
        .filter(|span| matches!(span, CaptureSpan::Keyword { .. }))
        .map(CaptureSpan::range)
        .collect();
    assert_eq!(keywords, vec![(0, 2), (10, 12)], "{spans:?}");
}

#[test]
fn a_string_and_its_quotes_form_one_span() {
    let spans = highlight("let s = \"hi\";", Some("rust"));

    assert!(has(&spans, CaptureSpan::String { start: 8, end: 12 }), "{spans:?}");
}

#[test]
fn a_block_comment_across_lines_is_one_span() {
    let code = "/* one\ntwo */";
    let spans = highlight(code, Some("rust"));

    assert_eq!(spans, vec![CaptureSpan::Comment { start: 0, end: code.len() as u32 }]);
}

#[test]
fn spans_are_ordered_in_bounds_and_on_char_boundaries() {
    for (code, language) in [
        ("let s = \"héllo wörld\"; // ünï\nfn f() {}\n", "rust"),
        ("fn a() {}\r\nfn b() {}\r\n", "rust"),
        ("def f():\n    return '日本語'", "python"),
        ("# title\n\nsome *text*", "markdown"),
        ("", "rust"),
        ("\n\n", "rust"),
    ] {
        let mut previous_end = 0;
        for span in highlight(code, Some(language)) {
            let (start, end) = span.range();
            let (start, end) = (start as usize, end as usize);
            assert!(previous_end <= start && start < end, "{span:?} in {code:?}");
            assert!(end <= code.len(), "{span:?} in {code:?}");
            assert!(code.is_char_boundary(start) && code.is_char_boundary(end), "{span:?}");
            previous_end = end;
        }
    }
}

#[test]
fn code_without_a_trailing_newline_is_highlighted_to_its_end() {
    let spans = highlight("fn main", Some("rust"));

    assert!(has(&spans, CaptureSpan::Keyword { start: 0, end: 2 }), "{spans:?}");
    assert!(spans.iter().all(|span| span.range().1 <= 7), "{spans:?}");
}

#[test]
fn every_language_the_arborium_backend_bundles_is_available() {
    // PowerShell is the one arborium language no bundled Sublime set has.
    for language in [
        "rust", "python", "javascript", "typescript", "tsx", "c", "cpp", "c#", "java", "kotlin",
        "swift", "go", "ruby", "php", "bash", "lua", "sql", "html", "css", "scss", "json",
        "yaml", "toml", "xml", "markdown", "dockerfile", "diff", "haskell", "scala", "dart",
        "zig", "elixir", "make", "glsl",
    ] {
        let backend = SyntectBackend::new();
        assert!(find_syntax(&backend.syntaxes, language).is_some(), "{language}");
    }
}

#[test]
fn aliases_and_extensions_reach_the_same_syntax() {
    let backend = SyntectBackend::new();
    let name = |language| find_syntax(&backend.syntaxes, language).map(|syntax| syntax.name.clone());

    assert_eq!(name("rs"), name("Rust"));
    assert_eq!(name("py"), name("python"));
    assert_eq!(name("yml"), name("yaml"));
    assert_eq!(name("jsx"), name("js"));
    assert_eq!(name("csharp"), name("c#"));
    assert_eq!(name("shell"), name("sh"));
}

#[test]
fn languages_added_by_two_face_highlight() {
    for (code, language, expected) in [
        ("edition = \"2024\"", "toml", "string"),
        ("let x: number = 1;", "typescript", "number"),
        ("fun main() {}", "kotlin", "keyword"),
    ] {
        let spans = highlight(code, Some(language));
        assert!(
            spans.iter().any(|span| span.color() == CaptureSpan::from_capture(expected, 0, 1).color()),
            "{language}: {spans:?}"
        );
    }
}

#[test]
fn returns_no_captures_without_a_supported_language() {
    assert!(highlight("plain text", None).is_empty());
    assert!(highlight("plain text", Some("no-such-language")).is_empty());
}

#[test]
fn an_added_syntax_is_used_for_its_extension_and_name() {
    add_syntax_from_str(&shout_syntax("Shout Alpha", "shoutalpha", "keyword.control")).unwrap();

    let by_extension = highlight("hello WORLD", Some("shoutalpha"));
    assert_eq!(by_extension, vec![CaptureSpan::Keyword { start: 6, end: 11 }]);
    assert_eq!(highlight("hello WORLD", Some("shout alpha")), by_extension);
}

#[test]
fn a_backend_built_earlier_notices_syntaxes_added_later() {
    let mut backend = SyntectBackend::new();
    assert!(backend.highlight("HELLO", Some("shoutlate")).is_empty());

    add_syntax_from_str(&shout_syntax("Shout Late", "shoutlate", "keyword.control")).unwrap();

    assert_eq!(
        backend.highlight("HELLO", Some("shoutlate")),
        vec![CaptureSpan::Keyword { start: 0, end: 5 }]
    );
}

#[test]
fn the_highlight_cache_drops_entries_computed_before_a_syntax_was_added() {
    let mut cache = HighlightCache::new(4);
    assert!(cache.highlight("HELLO", Some("shoutcache")).is_empty());

    add_syntax_from_str(&shout_syntax("Shout Cache", "shoutcache", "keyword.control")).unwrap();

    assert_eq!(
        &*cache.highlight("HELLO", Some("shoutcache")),
        &[CaptureSpan::Keyword { start: 0, end: 5 }]
    );
}

#[test]
fn a_later_definition_replaces_an_earlier_one_with_the_same_extension() {
    add_syntax_from_str(&shout_syntax("Shout One", "shoutdupe", "keyword.control")).unwrap();
    add_syntax_from_str(&shout_syntax("Shout Two", "shoutdupe", "string.quoted")).unwrap();

    assert_eq!(
        highlight("HELLO", Some("shoutdupe")),
        vec![CaptureSpan::String { start: 0, end: 5 }]
    );
}

#[test]
fn an_added_syntax_can_replace_a_bundled_language() {
    // Same extension as the bundled TOML, so it must win over it.
    add_syntax_from_str(&shout_syntax("Shout Toml", "tml", "keyword.control")).unwrap();
    add_syntax_from_str(&shout_syntax("Loud Toml", "toml", "constant.language")).unwrap();

    assert_eq!(
        highlight("HELLO = 1", Some("toml")),
        vec![CaptureSpan::Constant { start: 0, end: 5 }]
    );
}

#[test]
fn invalid_definitions_are_rejected_and_register_nothing() {
    let revision_before = revision();

    for source in [
        "not: [valid",
        "name: NoContexts\nfile_extensions: [badnone]\nscope: source.bad\n",
        "name: BadRegex\nfile_extensions: [badregex]\nscope: source.bad\ncontexts:\n  main:\n    - match: '('\n      scope: keyword\n",
    ] {
        let error = add_syntax_from_str(source).unwrap_err();
        assert!(error.message().starts_with("Invalid syntax definition"), "{error}");
    }

    assert_eq!(revision(), revision_before);
    assert!(highlight("x", Some("badregex")).is_empty());
}

#[test]
fn a_definition_file_without_a_name_is_named_after_the_file() {
    let dir = TempDir::new("unnamed");
    let path = dir.0.join("Shout Unnamed.sublime-syntax");
    fs::write(
        &path,
        "%YAML 1.2\n---\nfile_extensions: [shoutunnamed]\nscope: source.shoutunnamed\ncontexts:\n  main:\n    - match: '[A-Z]+'\n      scope: keyword.control\n",
    )
    .unwrap();

    add_syntax_from_file(&path).unwrap();

    assert_eq!(
        highlight("ABC", Some("Shout Unnamed")),
        vec![CaptureSpan::Keyword { start: 0, end: 3 }]
    );
}

#[test]
fn a_missing_file_is_reported_with_its_path() {
    let error = add_syntax_from_file("/definitely/not/here.sublime-syntax").unwrap_err();

    assert!(error.message().contains("here.sublime-syntax"), "{error}");
}

#[test]
fn a_directory_adds_every_definition_below_it_and_ignores_other_files() {
    let dir = TempDir::new("tree");
    fs::create_dir_all(dir.0.join("nested")).unwrap();
    fs::write(dir.0.join("a.sublime-syntax"), shout_syntax("Shout Dir A", "shoutdira", "keyword.control")).unwrap();
    fs::write(dir.0.join("nested/b.sublime-syntax"), shout_syntax("Shout Dir B", "shoutdirb", "string.quoted")).unwrap();
    fs::write(dir.0.join("notes.txt"), "not a syntax").unwrap();
    fs::write(dir.0.join("theme.tmTheme"), "<plist/>").unwrap();

    assert_eq!(add_syntax_from_dir(&dir.0).unwrap(), 2);

    assert_eq!(highlight("HI", Some("shoutdira")), vec![CaptureSpan::Keyword { start: 0, end: 2 }]);
    assert_eq!(highlight("HI", Some("shoutdirb")), vec![CaptureSpan::String { start: 0, end: 2 }]);
}

#[test]
fn one_invalid_file_leaves_the_whole_directory_unregistered() {
    let dir = TempDir::new("partial");
    fs::write(dir.0.join("a.sublime-syntax"), shout_syntax("Shout Partial", "shoutpartial", "keyword.control")).unwrap();
    fs::write(dir.0.join("b.sublime-syntax"), "not: [valid").unwrap();
    let revision_before = revision();

    let error = add_syntax_from_dir(&dir.0).unwrap_err();

    assert!(error.message().contains("b.sublime-syntax"), "{error}");
    assert_eq!(revision(), revision_before);
    assert!(highlight("HI", Some("shoutpartial")).is_empty());
}

#[test]
fn an_unreadable_directory_is_an_error() {
    assert!(add_syntax_from_dir("/definitely/not/a/directory").is_err());
}
