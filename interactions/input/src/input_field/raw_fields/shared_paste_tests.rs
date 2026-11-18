//! A paste that goes into the text as the bytes the clipboard delivered, and a copy that is
//! written from the chunks it is in: the same results as the copying paths, without the copies.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use aimer_clipboard::clipboard::ClipboardText;

use super::RawTextField;
use super::test_support::focused_multiline_field;
use crate::TextEditingController;
use crate::editable_text::TextEditingValue;

fn text_of(bytes: usize) -> String {
    "line of a pasted text, with caf\u{e9} and \u{4f60}\u{597d}\n".repeat(bytes / 40 + 1)
}

fn clipboard(text: &str) -> ClipboardText {
    Arc::new(text.to_owned())
}

/// A field holding `text` with the caret at `caret` (a grapheme offset) and every change it reports
/// collected.
fn field(text: &str, caret: usize) -> (RawTextField, Rc<RefCell<Vec<String>>>) {
    let mut field = focused_multiline_field(TextEditingController::with_text(text), 3);
    let reported = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&reported);
    field.on_changed = (move |text: String| sink.borrow_mut().push(text)).into();
    field.cursor.set_offset(caret);
    (field, reported)
}

#[test]
fn a_shared_paste_makes_the_text_a_copying_paste_makes() {
    let pasted = text_of(300_000);
    for (before, caret) in [("", 0), ("hello world", 5), (text_of(200_000).leak() as &str, 1234), (text_of(300_000).leak() as &str, 0)] {
        let (shared, shared_reports) = field(before, caret);
        let (copied, copied_reports) = field(before, caret);

        assert!(shared.insert_shared(&clipboard(&pasted)));
        assert!(copied.insert_text(&pasted));

        assert_eq!(shared.controller.text(), copied.controller.text(), "{} bytes before", before.len());
        assert_eq!(shared.cursor.offset(), copied.cursor.offset());
        assert_eq!(shared.controller.revision(), copied.controller.revision());
        assert_eq!(*shared_reports.borrow(), *copied_reports.borrow(), "what on_changed is given");
    }
}

#[test]
fn a_shared_paste_replaces_the_selection_and_undoes_in_one_step() {
    let before = text_of(120_000);
    let (field, _) = field(&before, 0);
    field.cursor.set_selection_anchor(Some(10));
    field.cursor.set_offset(5000);
    let pasted = text_of(90_000);

    assert!(field.insert_shared(&clipboard(&pasted)));

    use unicode_segmentation::UnicodeSegmentation;
    let graphemes = |text: &str, end: usize| text.grapheme_indices(true).nth(end).map_or(text.len(), |(byte, _)| byte);
    let (from, to) = (graphemes(&before, 10), graphemes(&before, 5000));
    let expected = format!("{}{}{}", &before[..from], pasted, &before[to..]);
    assert_eq!(field.controller.text(), expected);
    assert!(field.controller.undo());
    assert_eq!(field.controller.text(), before, "one undo takes the whole paste back");
    assert!(field.controller.redo());
    assert_eq!(field.controller.text(), expected);
}

#[test]
fn a_shared_paste_is_not_copied() {
    let backing = clipboard(&text_of(300_000));
    let (field, _) = field("start ", 6);
    assert!(field.insert_shared(&backing));
    assert!(Arc::strong_count(&backing) > 10, "the chunks of the text are slices of the paste: {}", Arc::strong_count(&backing));
}

#[test]
fn a_paste_that_must_be_changed_or_is_short_is_copied_in() {
    // Short: copying costs less than the chunks that share.
    let short = clipboard("a short paste");
    let (field_short, _) = field("", 0);
    assert!(field_short.insert_shared(&short));
    assert_eq!(Arc::strong_count(&short), 1);
    assert_eq!(field_short.controller.text(), "a short paste");

    // A length limit changes what goes in.
    let long = clipboard(&text_of(300_000));
    let (limited, _) = field("", 0);
    let mut limited = limited;
    limited.max_length = Some(20);
    assert!(limited.insert_shared(&long));
    assert_eq!(Arc::strong_count(&long), 1);
    assert_eq!(limited.controller.grapheme_count(), 20);

    // One line: the line feeds become spaces.
    let (single, _) = field("", 0);
    let mut single = single;
    single.max_lines = Some(1);
    assert!(single.insert_shared(&long));
    assert_eq!(Arc::strong_count(&long), 1);
    assert!(!single.controller.text().contains('\n'));
}

#[test]
fn an_empty_paste_changes_nothing() {
    let (field, reported) = field("keep", 4);
    assert!(!field.insert_shared(&clipboard("")));
    assert_eq!(field.controller.text(), "keep");
    assert!(reported.borrow().is_empty());
}

#[test]
fn the_pieces_of_a_range_are_the_text_of_the_range() {
    for text in [String::new(), "short".to_owned(), text_of(500_000)] {
        let value = TextEditingValue::with_str(&text);
        let mut ranges = vec![0..text.len(), 0..0, text.len()..text.len()];
        if text.len() > 100 {
            let mut start = text.len() / 3;
            while !text.is_char_boundary(start) {
                start += 1;
            }
            let mut end = text.len() * 2 / 3;
            while !text.is_char_boundary(end) {
                end += 1;
            }
            ranges.push(start..end);
        }
        for range in ranges {
            assert_eq!(value.pieces(range.clone()).concat(), text[range.clone()], "{range:?} of {} bytes", text.len());
        }
    }
}
