//! The caret and offset lookups of a long text are indexed, and have to answer what
//! a search from the start of the text would.

use aimer_style::TextOverflow;
use aimer_text::{SpanStyle, StyledRange};
use unicode_segmentation::UnicodeSegmentation;

use super::RawTextField;
use super::test_support::{dummy_build_context, focused_multiline_field};
use crate::input_field::controller::TextFieldController;
use crate::input_field::highlight::TextHighlighter;

/// Text that exercises the lookups: lines that wrap, hard breaks and blank lines, a
/// line that ends the text, and graphemes of more than one scalar or byte.
fn text() -> String {
    let line = "The quick brown fox \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} jumps over e\u{301} the lazy \
        \u{4F60}\u{597D} dog, and keeps running across the width of the pane so that it wraps";
    let mut text = String::new();
    for index in 0..40 {
        text.push_str(line);
        text.push_str(if index % 7 == 3 { "\n\n" } else { "\n" });
    }
    text.push_str("last");
    text
}

/// Plain ASCII, where a grapheme is a byte, apart from a carriage return before a
/// line feed, which with it is one.
fn ascii_text(crlf: bool) -> String {
    let line = "The quick brown fox jumps over the lazy dog, and keeps running across the width of the pane so that it wraps\t!";
    let ending = if crlf { "\r\n" } else { "\n" };
    let mut text = String::new();
    for index in 0..12 {
        text.push_str(line);
        text.push_str(if index % 5 == 3 { "\n\n" } else { ending });
    }
    text.push_str("last");
    text
}

fn field(highlighted: bool) -> RawTextField {
    field_with(text(), highlighted)
}

fn field_with(text: String, highlighted: bool) -> RawTextField {
    let controller = TextFieldController::with_initial(text);
    let mut field = focused_multiline_field(controller, 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    if highlighted {
        // Headings of a different size make lines of different heights, and the
        // rich-text layout, which appends the line breaks after the other clusters.
        field.highlighter = Some(TextHighlighter::new(|text| {
            text.match_indices("quick")
                .map(|(start, word)| {
                    StyledRange::new(start..start + word.len(), SpanStyle::new().font_size(30))
                })
                .collect()
        }));
    }
    field
}

#[test]
fn indexed_lookups_answer_what_scanning_the_text_does() {
    check_lookups(|highlighted| field(highlighted));
}

#[test]
fn ascii_lookups_answer_what_scanning_the_text_does() {
    for crlf in [false, true] {
        check_lookups(|highlighted| field_with(ascii_text(crlf), highlighted));
    }
}

fn check_lookups(make: impl Fn(bool) -> RawTextField) {
    for highlighted in [false, true] {
        let field = make(highlighted);
        let ctx = dummy_build_context(300.0, 600.0);
        let geometry = field.editable_geometry(&ctx, 300.0);
        let display = geometry.display.clone();
        let graphemes = display.graphemes(true).count();

        for offset in 0..=graphemes + 2 {
            assert_eq!(
                geometry.display_byte_offset(offset),
                geometry.display_byte_offset_scanning(offset),
                "the byte of grapheme {offset}, highlighted: {highlighted}"
            );
            assert_eq!(
                geometry.caret_geometry(offset),
                geometry.caret_geometry_scanning(offset),
                "the caret at grapheme {offset}, highlighted: {highlighted}"
            );
        }
        for byte in (0..=display.len() + 2).filter(|&byte| display.is_char_boundary(byte.min(display.len()))) {
            assert_eq!(
                geometry.display_grapheme_offset(byte),
                geometry.display_grapheme_offset_scanning(byte),
                "the grapheme at byte {byte}, highlighted: {highlighted}"
            );
        }
    }
}
