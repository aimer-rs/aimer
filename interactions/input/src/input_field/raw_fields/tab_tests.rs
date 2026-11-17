//! The Tab key in a field that takes it: it indents, and it is not given away to focus
//! traversal, which the field's focus node says.

use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_widget::{EventElement, FocusNode};

use super::RawTextField;
use super::test_support::{field_config, key};
use crate::TextEditingController;
use crate::input_field::caret::CaretBlink;

fn field(text: &str, configure: impl FnOnce(&mut super::RawFieldConfig)) -> (RawTextField, FocusNode) {
    let mut config = field_config(TextEditingController::with_text(text));
    config.min_lines = Some(3);
    config.max_lines = None;
    configure(&mut config);
    let focus = FocusNode::new();
    let field = RawTextField::new(config, CaretBlink::new(), focus.clone());
    let _ = field.on_event(&ElementEvent::FocusGained);
    (field, focus)
}

#[test]
fn tab_inserts_the_spaces_it_was_given_at_the_caret() {
    let (field, focus) = field("ab", |config| config.tab_spaces = Some(4));
    assert!(focus.claims_tab(), "the field takes Tab from focus traversal");
    field.cursor.set_offset(1);

    assert!(field.on_event(&key(NamedKey::Tab)).is_consumed());
    assert_eq!(field.controller.text(), "a    b");
    assert_eq!(field.cursor.offset(), 5, "the caret is after the spaces");

    let _ = field.on_event(&key(NamedKey::Tab));
    assert_eq!(field.controller.text(), "a        b");
}

#[test]
fn tab_replaces_a_selection_as_typed_text_does() {
    let (field, _) = field("hello world", |config| config.tab_spaces = Some(2));
    field.cursor.set_selection_anchor(Some(0));
    field.cursor.set_offset(5);

    let _ = field.on_event(&key(NamedKey::Tab));

    assert_eq!(field.controller.text(), "   world");
}

#[test]
fn shift_tab_is_not_taken() {
    let (field, _) = field("ab", |config| config.tab_spaces = Some(4));
    let shifted = ElementEvent::KeyInput {
        key: NamedKey::Tab,
        action: KeyAction::Pressed,
        modifiers: Modifiers { shift: true, ..Modifiers::default() },
    };

    assert!(!field.on_event(&shifted).is_consumed());
    assert_eq!(field.controller.text(), "ab");
}

#[test]
fn a_field_that_was_not_asked_leaves_tab_to_focus() {
    let (field, focus) = field("ab", |_| {});
    assert!(!focus.claims_tab());

    assert!(!field.on_event(&key(NamedKey::Tab)).is_consumed());
    assert_eq!(field.controller.text(), "ab");
}

#[test]
fn a_read_only_or_single_line_field_does_not_take_tab() {
    let (read_only, focus) = field("ab", |config| {
        config.tab_spaces = Some(4);
        config.read_only = true;
    });
    assert!(!focus.claims_tab(), "nothing to indent in a read-only field");
    assert!(!read_only.on_event(&key(NamedKey::Tab)).is_consumed());
    assert_eq!(read_only.controller.text(), "ab");

    let (single_line, focus) = field("ab", |config| {
        config.tab_spaces = Some(4);
        config.max_lines = Some(1);
    });
    assert!(!focus.claims_tab(), "nor in a field of one line");
    assert!(!single_line.on_event(&key(NamedKey::Tab)).is_consumed());
}
