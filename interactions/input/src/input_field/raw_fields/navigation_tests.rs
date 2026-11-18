//! The keys of a text editor: Home and End, movement and deletion by word and by line, the ends of
//! the text, pages, and shift extending any of them.

use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_style::TextOverflow;
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, EventElement, LayoutElement};

use super::test_support::{dummy_build_context, focused_multiline_field};
use super::{Navigation, PLATFORM_IS_MAC, RawTextField, Reach, navigation_for, word_edge};
use crate::TextEditingController;
use crate::editable_text::VisualLine;

// ── The table ───────────────────────────────────────────────────────────────────

fn held(ctrl: bool, shift: bool, alt: bool, meta: bool) -> Modifiers {
    Modifiers { ctrl, shift, alt, meta }
}

const NONE: Modifiers = Modifiers { ctrl: false, shift: false, alt: false, meta: false };
const CTRL: Modifiers = Modifiers { ctrl: true, shift: false, alt: false, meta: false };
const ALT: Modifiers = Modifiers { ctrl: false, shift: false, alt: true, meta: false };
const META: Modifiers = Modifiers { ctrl: false, shift: false, alt: false, meta: true };

#[test]
fn the_keys_mean_what_they_do_in_most_editors_on_each_platform() {
    use Navigation::{Delete, Move};
    let left = NamedKey::ArrowLeft;
    // Everywhere: plain keys, and Home and End are the ends of the line.
    for mac in [true, false] {
        assert_eq!(navigation_for(&left, &NONE, mac), Some(Move(Reach::Left)));
        assert_eq!(navigation_for(&NamedKey::ArrowDown, &NONE, mac), Some(Move(Reach::Down)));
        assert_eq!(navigation_for(&NamedKey::Home, &NONE, mac), Some(Move(Reach::LineStart)));
        assert_eq!(navigation_for(&NamedKey::End, &NONE, mac), Some(Move(Reach::LineEnd)));
        assert_eq!(navigation_for(&NamedKey::Home, &CTRL, mac), Some(Move(Reach::DocumentStart)));
        assert_eq!(navigation_for(&NamedKey::End, &META, mac), Some(Move(Reach::DocumentEnd)));
        assert_eq!(navigation_for(&NamedKey::PageUp, &NONE, mac), Some(Move(Reach::PageUp)));
        assert_eq!(navigation_for(&NamedKey::PageDown, &NONE, mac), Some(Move(Reach::PageDown)));
        // Shift is not part of what a key means: it extends it.
        assert_eq!(navigation_for(&NamedKey::End, &held(false, true, false, false), mac), Some(Move(Reach::LineEnd)));
        // Plain Backspace and Delete are ordinary edits, and a letter is typed.
        assert_eq!(navigation_for(&NamedKey::Backspace, &NONE, mac), None);
        assert_eq!(navigation_for(&NamedKey::Delete, &NONE, mac), None);
        assert_eq!(navigation_for(&NamedKey::Other("a".into()), &NONE, mac), None);
    }
    // macOS: Option is by word, Command is to the ends of the line and of the text.
    assert_eq!(navigation_for(&left, &ALT, true), Some(Move(Reach::WordLeft)));
    assert_eq!(navigation_for(&NamedKey::ArrowRight, &ALT, true), Some(Move(Reach::WordRight)));
    assert_eq!(navigation_for(&left, &META, true), Some(Move(Reach::LineStart)));
    assert_eq!(navigation_for(&NamedKey::ArrowRight, &META, true), Some(Move(Reach::LineEnd)));
    assert_eq!(navigation_for(&NamedKey::ArrowUp, &META, true), Some(Move(Reach::DocumentStart)));
    assert_eq!(navigation_for(&NamedKey::ArrowDown, &META, true), Some(Move(Reach::DocumentEnd)));
    assert_eq!(navigation_for(&NamedKey::Backspace, &ALT, true), Some(Delete(Reach::WordLeft)));
    assert_eq!(navigation_for(&NamedKey::Backspace, &META, true), Some(Delete(Reach::LineStart)));
    assert_eq!(navigation_for(&NamedKey::Delete, &ALT, true), Some(Delete(Reach::WordRight)));
    assert_eq!(navigation_for(&NamedKey::Delete, &META, true), Some(Delete(Reach::LineEnd)));
    assert_eq!(navigation_for(&left, &CTRL, true), Some(Move(Reach::Left)), "Control+Left is not a word on macOS");
    // Elsewhere: Control is by word; Alt and the Windows key change nothing.
    assert_eq!(navigation_for(&left, &CTRL, false), Some(Move(Reach::WordLeft)));
    assert_eq!(navigation_for(&NamedKey::ArrowRight, &CTRL, false), Some(Move(Reach::WordRight)));
    assert_eq!(navigation_for(&NamedKey::Backspace, &CTRL, false), Some(Delete(Reach::WordLeft)));
    assert_eq!(navigation_for(&NamedKey::Delete, &CTRL, false), Some(Delete(Reach::WordRight)));
    assert_eq!(navigation_for(&left, &ALT, false), Some(Move(Reach::Left)));
    assert_eq!(navigation_for(&left, &META, false), Some(Move(Reach::Left)));
    assert_eq!(navigation_for(&NamedKey::ArrowUp, &CTRL, false), Some(Move(Reach::Up)));
    assert_eq!(navigation_for(&NamedKey::Backspace, &META, false), None);
}

// ── Words ───────────────────────────────────────────────────────────────────────

#[test]
fn a_word_edge_is_the_end_of_the_word_going_forward_and_its_start_going_back() {
    let text = "alpha beta, gamma";
    // Forward: the end of the word the caret is in, or of the next.
    assert_eq!(word_edge(text, 0, true), Some(5));
    assert_eq!(word_edge(text, 2, true), Some(5));
    assert_eq!(word_edge(text, 5, true), Some(10), "from a word's end, the next word's end");
    assert_eq!(word_edge(text, 10, true), Some(17), "punctuation and spaces are passed over");
    assert_eq!(word_edge(text, 17, true), None);
    // Back.
    assert_eq!(word_edge(text, 17, false), Some(12));
    assert_eq!(word_edge(text, 14, false), Some(12), "from inside a word, its start");
    assert_eq!(word_edge(text, 12, false), Some(6), "from a word's start, the previous word's start");
    assert_eq!(word_edge(text, 6, false), Some(0));
    assert_eq!(word_edge(text, 0, false), None);
}

#[test]
fn words_are_those_of_unicode_and_line_breaks_are_passed_over() {
    assert_eq!(word_edge("don't stop", 0, true), Some(5), "an apostrophe inside a word does not end it");
    assert_eq!(word_edge("foo_bar baz", 0, true), Some(7), "an underscore joins");
    assert_eq!(word_edge("x = 12.5;", 3, true), Some(8), "a number with its point is one word");
    assert_eq!(word_edge("one\n\n  two", 3, true), Some(10), "blank lines and indentation are passed over");
    assert_eq!(word_edge("one\n\n  two", 10, false), Some(7));
    assert_eq!(word_edge("caf\u{e9} cr\u{e8}me", 0, true), Some(5), "accented letters are letters");
    assert_eq!(word_edge("--- ... !!!", 0, true), None, "punctuation alone holds no word");
    // Ideographs are a word each.
    let han = "\u{4f60}\u{597d}";
    assert_eq!(word_edge(han, 0, true), Some(3));
    assert_eq!(word_edge(han, 6, false), Some(3));
}

// ── Lines ───────────────────────────────────────────────────────────────────────

fn visual(start: usize, end: usize) -> VisualLine {
    VisualLine { byte_start: start, byte_end: end, grapheme_start: start, grapheme_end: end, width: 0.0 }
}

#[test]
fn a_line_edge_is_where_the_visual_line_starts_and_ends() {
    // "hello world\nfoo": two hard lines of 11 and 3 graphemes.
    let hard = [visual(0, 11), visual(12, 15)];
    let none = |_: &VisualLine| 0;
    assert_eq!(super::line_edge(&hard, 5, false, none), Some(0));
    assert_eq!(super::line_edge(&hard, 5, true, none), Some(11));
    assert_eq!(super::line_edge(&hard, 13, false, none), Some(12));
    assert_eq!(super::line_edge(&hard, 13, true, none), Some(15));
    assert_eq!(super::line_edge(&hard, 40, true, none), None, "a caret no line holds");

    // "aaaa bbbb cccc" wrapped after the spaces: three lines that join, the first two ending in one.
    let wrapped = [visual(0, 5), visual(5, 10), visual(10, 14)];
    let one_space = |line: &VisualLine| usize::from(line.grapheme_end < 14);
    assert_eq!(super::line_edge(&wrapped, 2, true, one_space), Some(4), "the end stops before the space it wraps after");
    assert_eq!(super::line_edge(&wrapped, 7, false, one_space), Some(5));
    assert_eq!(super::line_edge(&wrapped, 7, true, one_space), Some(9));
    assert_eq!(super::line_edge(&wrapped, 14, true, one_space), Some(14), "the last line has nothing to stop before");
    // A caret at a join is drawn on the second line: Home goes to its start, not the first's.
    assert_eq!(super::line_edge(&wrapped, 5, false, one_space), Some(5));
    assert_eq!(super::line_edge(&wrapped, 5, true, one_space), Some(9));
    // A line that is only spaces keeps them.
    let blank = [visual(0, 3), visual(3, 6)];
    assert_eq!(super::line_edge(&blank, 1, true, |_| 3), Some(3));
}

// ── In a field ──────────────────────────────────────────────────────────────────

/// The modifiers that make a move a word at a time on this platform.
fn by_word() -> Modifiers {
    if PLATFORM_IS_MAC { ALT } else { CTRL }
}

fn press(field: &RawTextField, key: NamedKey, modifiers: Modifiers) -> bool {
    field
        .on_event(&ElementEvent::KeyInput { key, action: KeyAction::Pressed, modifiers })
        .is_consumed()
}

fn shifted(mut modifiers: Modifiers) -> Modifiers {
    modifiers.shift = true;
    modifiers
}

/// A focused field holding `text`, wrapping to `width` physical pixels, laid out once, with the
/// caret at grapheme `caret`.
fn field_of(text: &str, width: f32, caret: usize) -> (RawTextField, BuildContext<'static>) {
    let mut field = focused_multiline_field(TextEditingController::with_text(text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    let ctx = dummy_build_context(width, 400.0);
    field.cursor.set_offset(caret);
    field.update(&ctx);
    (field, ctx)
}

fn selection(field: &RawTextField) -> (usize, usize) {
    (field.cursor.selection_anchor().unwrap_or(field.cursor.offset()), field.cursor.offset())
}

#[test]
fn home_and_end_go_to_the_ends_of_the_line_and_not_of_the_text() {
    let text = "first line\nsecond line\nthird";
    let (field, _) = field_of(text, 600.0, 14);
    assert!(press(&field, NamedKey::Home, NONE));
    assert_eq!(field.cursor.offset(), 11, "the start of the second line");
    assert!(press(&field, NamedKey::End, NONE));
    assert_eq!(field.cursor.offset(), 22, "the end of the second line, before its line break");
    // Again: nowhere further to go.
    assert!(press(&field, NamedKey::End, NONE));
    assert_eq!(field.cursor.offset(), 22);
    // With Control or Command held they are the ends of the text.
    assert!(press(&field, NamedKey::Home, CTRL));
    assert_eq!(field.cursor.offset(), 0);
    assert!(press(&field, NamedKey::End, META));
    assert_eq!(field.cursor.offset(), text.len());
}

#[test]
fn shift_extends_the_selection_to_the_end_or_the_start_of_the_line() {
    let (field, _) = field_of("first line\nsecond line\nthird", 600.0, 14);
    assert!(press(&field, NamedKey::End, shifted(NONE)));
    assert_eq!(selection(&field), (14, 22));
    assert!(press(&field, NamedKey::Home, shifted(NONE)));
    assert_eq!(selection(&field), (14, 11), "the anchor stays and the end turns around it");
    // Without shift the selection is given up.
    assert!(press(&field, NamedKey::End, NONE));
    assert_eq!(selection(&field), (22, 22));
}

#[test]
fn home_and_end_follow_a_line_that_wraps() {
    // One hard line, wrapped by a narrow field into several visual lines.
    let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
    let (field, _) = field_of(text, 160.0, 0);
    let lines = field.geometry_cache.latest().expect("laid out").visual_lines.clone();
    assert!(lines.len() >= 3, "the text wraps: {} lines", lines.len());
    let second = &lines[1];
    let middle = (second.grapheme_start + second.grapheme_end) / 2;
    field.cursor.set_offset(middle);

    assert!(press(&field, NamedKey::Home, NONE));
    assert_eq!(field.cursor.offset(), second.grapheme_start, "the start of the visual line, not of the hard line");
    assert!(press(&field, NamedKey::End, NONE));
    let end = field.cursor.offset();
    assert!(end <= second.grapheme_end && end > second.grapheme_start);
    assert_ne!(text.chars().nth(end - 1), Some(' '), "the end of the line is where its last word ends");
    // Pressed again it stays; the caret does not slip onto the next line.
    assert!(press(&field, NamedKey::End, NONE));
    assert_eq!(field.cursor.offset(), end);
    assert!(press(&field, NamedKey::Home, NONE));
    assert_eq!(field.cursor.offset(), second.grapheme_start);
}

#[test]
fn a_word_at_a_time_in_either_direction_and_with_shift_a_selection_of_words() {
    let text = "alpha beta, gamma";
    let (field, _) = field_of(text, 600.0, 0);
    assert!(press(&field, NamedKey::ArrowRight, by_word()));
    assert_eq!(field.cursor.offset(), 5, "to the end of the first word");
    assert!(press(&field, NamedKey::ArrowRight, by_word()));
    assert_eq!(field.cursor.offset(), 10, "the comma and the space are passed over");
    assert!(press(&field, NamedKey::ArrowRight, shifted(by_word())));
    assert_eq!(selection(&field), (10, 17), "shift selects the word");
    assert!(press(&field, NamedKey::ArrowLeft, shifted(by_word())));
    assert_eq!(selection(&field), (10, 12), "and back over it, to where that word starts");
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 6, "a plain move gives the selection up and goes on from the caret");
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 0);
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 0, "nothing further");
}

#[test]
fn a_word_move_goes_across_lines() {
    let (field, _) = field_of("one\n\ntwo three", 600.0, 3);
    assert!(press(&field, NamedKey::ArrowRight, by_word()));
    assert_eq!(field.cursor.offset(), 8, "past the line breaks, to the end of the next word");
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 5, "to its start");
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 0);
}

#[test]
fn the_start_and_end_of_the_text_on_the_keys_of_each_platform() {
    let text = "first\nsecond\nthird";
    let (field, _) = field_of(text, 600.0, 8);
    let (start, end) = if PLATFORM_IS_MAC { (NamedKey::ArrowUp, NamedKey::ArrowDown) } else { (NamedKey::Home, NamedKey::End) };
    let mods = if PLATFORM_IS_MAC { META } else { CTRL };
    assert!(press(&field, start, mods));
    assert_eq!(field.cursor.offset(), 0);
    assert!(press(&field, end, shifted(mods)));
    assert_eq!(selection(&field), (0, text.len()), "and with shift, all of it");
}

#[test]
fn deleting_a_word_back_and_forward_is_one_edit_to_undo() {
    let (field, _) = field_of("alpha beta gamma", 600.0, 10);
    assert!(press(&field, NamedKey::Backspace, by_word()));
    assert_eq!(field.controller.text(), "alpha  gamma", "the word before the caret");
    assert_eq!(field.cursor.offset(), 6);
    assert!(press(&field, NamedKey::Delete, by_word()));
    assert_eq!(field.controller.text(), "alpha ", "the word after it, with the spaces up to it");
    assert!(field.controller.undo());
    assert_eq!(field.controller.text(), "alpha  gamma");
    assert!(field.controller.undo());
    assert_eq!(field.controller.text(), "alpha beta gamma");
}

#[test]
fn a_delete_by_word_with_a_selection_deletes_the_selection_alone() {
    let (field, _) = field_of("alpha beta gamma", 600.0, 0);
    field.cursor.set_selection_anchor(Some(2));
    field.cursor.set_offset(8);
    assert!(press(&field, NamedKey::Backspace, by_word()));
    assert_eq!(field.controller.text(), "alta gamma");
}

#[test]
fn a_delete_by_word_at_an_end_does_nothing_and_a_read_only_field_is_not_edited() {
    let (field, _) = field_of("one two", 600.0, 0);
    assert!(press(&field, NamedKey::Backspace, by_word()));
    assert_eq!(field.controller.text(), "one two");
    field.cursor.set_offset(7);
    assert!(press(&field, NamedKey::Delete, by_word()));
    assert_eq!(field.controller.text(), "one two");

    let (mut read_only, _) = field_of("one two", 600.0, 7);
    read_only.read_only = true;
    assert!(!press(&read_only, NamedKey::Backspace, by_word()), "the key is not taken");
    assert_eq!(read_only.controller.text(), "one two");
    // Moving is allowed.
    assert!(press(&read_only, NamedKey::ArrowLeft, by_word()));
    assert_eq!(read_only.cursor.offset(), 4);
}

#[test]
fn macos_deletes_to_the_ends_of_the_line_with_command() {
    if !PLATFORM_IS_MAC {
        return;
    }
    let (field, _) = field_of("first line\nsecond line", 600.0, 17);
    assert!(press(&field, NamedKey::Backspace, META));
    assert_eq!(field.controller.text(), "first line\n line", "back to the start of the line");
    field.cursor.set_offset(13);
    assert!(press(&field, NamedKey::Delete, META));
    assert_eq!(field.controller.text(), "first line\n l", "forward to its end");
}

#[test]
fn a_page_moves_the_lines_a_field_shows_less_one_and_stops_at_the_ends() {
    let text: String = (0..40).map(|n| format!("line {n}\n")).collect();
    let mut config = super::test_support::field_config(TextEditingController::with_text(&text));
    config.min_lines = Some(3);
    config.max_lines = Some(6);
    let field = RawTextField::new(config, crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    let ctx = dummy_build_context(600.0, 400.0);
    field.cursor.set_offset(0);
    field.update(&ctx);

    assert!(press(&field, NamedKey::PageDown, NONE));
    assert_eq!(field.cursor.offset(), 5 * 7, "five lines down: line 5 starts after five lines of seven characters");
    assert!(press(&field, NamedKey::PageDown, shifted(NONE)));
    assert_eq!(selection(&field).0, 35, "shift keeps the anchor");
    assert_eq!(field.cursor.offset(), 70);
    assert!(press(&field, NamedKey::PageUp, NONE));
    assert_eq!(field.cursor.offset(), 35);
    for _ in 0..20 {
        press(&field, NamedKey::PageDown, NONE);
    }
    let last = field.controller.grapheme_count();
    assert!(field.cursor.offset() >= last - 8, "stops on the last line: {} of {last}", field.cursor.offset());
    for _ in 0..20 {
        press(&field, NamedKey::PageUp, NONE);
    }
    assert_eq!(field.cursor.offset(), 0, "and on the first");
}

#[test]
fn a_page_key_is_left_to_the_parent_in_a_field_that_grows_with_its_text() {
    let (field, _) = field_of("one\ntwo\nthree", 600.0, 0);
    assert!(!press(&field, NamedKey::PageDown, NONE), "the parent scrolls the page");
    assert_eq!(field.cursor.offset(), 0);
}

#[test]
fn a_secure_field_has_no_words_so_a_word_is_the_line() {
    let mut config = super::test_support::field_config(TextEditingController::with_text("secret words"));
    config.input_type = crate::input_field::InputType::Password;
    let field = RawTextField::new(config, crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.cursor.set_offset(3);
    assert!(press(&field, NamedKey::ArrowRight, by_word()));
    assert_eq!(field.cursor.offset(), 12);
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), 0);
}

#[test]
fn the_same_keys_work_in_a_long_text_laid_out_as_a_window() {
    let text: String = (0..3000).map(|n| format!("line number {n} of a long note\n")).collect();
    let mut field = focused_multiline_field(TextEditingController::with_text(&text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = 0;
    field.max_lines = Some(10);
    let ctx = dummy_build_context(800.0, 600.0);
    let middle = text.find("line number 1500 ").expect("a line");
    field.cursor.set_offset(middle + 5);
    field.update(&ctx);
    assert!(field.windowed_display().is_some(), "the field windows its text");

    assert!(press(&field, NamedKey::Home, NONE));
    assert_eq!(field.cursor.offset(), middle, "the start of the line, in the offsets of the whole text");
    assert!(press(&field, NamedKey::End, NONE));
    assert_eq!(field.cursor.offset(), middle + "line number 1500 of a long note".len());
    assert!(press(&field, NamedKey::ArrowLeft, by_word()));
    assert_eq!(field.cursor.offset(), middle + "line number 1500 of a long ".len());
    assert!(press(&field, NamedKey::Backspace, by_word()));
    assert!(field.controller.text().contains("line number 1500 of a note\n"), "the word before the caret, and the space after it, are deleted in a text of 90 KB");
    // To the ends of the whole text.
    assert!(press(&field, NamedKey::End, CTRL));
    assert_eq!(field.cursor.offset(), field.controller.grapheme_count());
    assert!(press(&field, NamedKey::Home, CTRL));
    assert_eq!(field.cursor.offset(), 0);
}
