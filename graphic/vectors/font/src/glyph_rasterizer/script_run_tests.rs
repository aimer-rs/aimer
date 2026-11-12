//! What announcing a run does to the rasterizer's script state.

use super::{GlyphRasterizer, is_kana};
use crate::font::TextLanguage;
use crate::system_fallback::{ScriptRequirement, from_run_calls};

const LANGUAGES: [Option<TextLanguage>; 4] = [
    None,
    Some(TextLanguage::Chinese),
    Some(TextLanguage::Japanese),
    Some(TextLanguage::Korean),
];

/// What announcing `text` has to leave behind, worked out the long way.
fn expected(text: &str, language: Option<TextLanguage>) -> (ScriptRequirement, Option<char>) {
    (
        ScriptRequirement::from_run(text, language),
        text.chars().find(|codepoint| is_kana(*codepoint)),
    )
}

#[track_caller]
fn assert_announces_in_full(text: &str, language: Option<TextLanguage>) {
    let mut rasterizer = GlyphRasterizer::new();
    rasterizer.begin_script_run(text, language);
    let (requirement, companion) = expected(text, language);
    assert_eq!(rasterizer.script_run, requirement, "{text:?} {language:?}");
    assert_eq!(rasterizer.run_companion, companion, "{text:?} {language:?}");
}

#[test]
fn every_ascii_character_announces_what_a_full_scan_finds() {
    // The requirement is built from the ideographs of the run and nothing else,
    // so one character at a time covers every ASCII string.
    for byte in 0..128_u8 {
        let text = char::from(byte).to_string();
        for language in LANGUAGES {
            assert_announces_in_full(&text, language);
            let (requirement, companion) = expected(&text, language);
            assert!(requirement.is_empty() && companion.is_none(), "{byte:#x}");
        }
    }
}

#[test]
fn ascii_strings_announce_what_a_full_scan_finds() {
    for text in [
        "",
        "plain words and 123, numbers.",
        "Detail paragraph 12: The compositor keeps this decorated scrolling row on the live path.",
        "tab\there\nnewline \u{7f}\u{0}",
    ] {
        for language in LANGUAGES {
            assert_announces_in_full(text, language);
        }
    }
}

#[test]
fn an_ascii_run_is_not_scanned_for_ideographs() {
    let mut rasterizer = GlyphRasterizer::new();
    let before = from_run_calls();
    rasterizer.begin_script_run("Plain ASCII text, 123.", None);
    rasterizer.begin_script_run("", Some(TextLanguage::Chinese));
    assert_eq!(from_run_calls(), before, "ASCII cannot carry a script requirement");
}

#[test]
fn text_beyond_ascii_is_still_judged_in_full() {
    for text in [
        "你好吗",
        "あの時は",
        "한국어",
        "abc 你好",
        "caf\u{e9}",
        "emoji \u{1f600}",
        "あ",
    ] {
        for language in LANGUAGES {
            assert_announces_in_full(text, language);
        }
    }
    let mut rasterizer = GlyphRasterizer::new();
    let before = from_run_calls();
    rasterizer.begin_script_run("abc \u{4f60}", None);
    assert_eq!(from_run_calls(), before + 1, "a run with an ideograph is scanned");
    assert!(!rasterizer.script_run.is_empty());
}

#[test]
fn an_ascii_run_clears_what_the_run_before_it_announced() {
    let mut rasterizer = GlyphRasterizer::new();
    rasterizer.begin_script_run("あの時は", Some(TextLanguage::Chinese));
    assert!(!rasterizer.script_run.is_empty());
    assert!(rasterizer.run_companion.is_some());

    rasterizer.begin_script_run("plain", None);
    assert!(rasterizer.script_run.is_empty(), "the earlier requirement leaked");
    assert!(rasterizer.run_companion.is_none(), "the earlier companion leaked");
    assert_eq!(rasterizer.script_run.language(), None);
}
