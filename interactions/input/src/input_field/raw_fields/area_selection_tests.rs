//! A field inside a selection area mirrors the area's selection into its cursor.

use std::rc::Rc;

use aimer_text_selection::selectable::{SelectionCoordinator, SelectionScope};
use aimer_text_selection::session::SelectionSession;
use aimer_widget::FocusNode;

use super::test_support::{dummy_build_context, field_config};
use super::{RawTextField, SelectionParticipant};
use crate::TextEditingController;
use crate::input_field::caret::{CaretBlink, CaretContext};

const TEXT: &str = "hello world";

/// A field registered with a selection area, and the participant the area sees.
fn field_in_an_area() -> (RawTextField, SelectionParticipant) {
    let context = dummy_build_context(400.0, 200.0);
    let session = SelectionSession::new(
        context.window().clone(),
        Rc::new(SelectionCoordinator::default()),
        aimer_widget::base::Color::Rgba(51, 153, 255, 96),
    );
    let participant = context
        .with_state(SelectionScope(session), |inner| SelectionParticipant::from_context(inner, Rc::from(TEXT)))
        .expect("the context is inside a selection area");
    let caret = CaretBlink::new();
    let field = RawTextField::with_caret_context(
        field_config(TextEditingController::with_text(TEXT)),
        caret.clone(),
        CaretContext::new(caret),
        FocusNode::new(),
        Some(participant.clone()),
        None,
    );
    (field, participant)
}

/// The area reports a selection as an ordered range, but the cursor has a side: its
/// offset is where the pointer is. A drag that goes backwards must leave the caret
/// at the end the pointer reached, not at the end the press came from.
#[test]
fn a_backwards_selection_leaves_the_caret_at_its_start() {
    let (field, participant) = field_in_an_area();

    participant.set_range(8, 2);
    field.sync_cursor_from_area();

    assert_eq!(field.cursor.offset(), 2, "the caret is where the pointer went");
    assert_eq!(field.cursor.selection_anchor(), Some(8), "and the selection is anchored where it began");
    assert_eq!(field.cursor.selection_range(), Some((2, 8)));
}

#[test]
fn a_forwards_selection_leaves_the_caret_at_its_end() {
    let (field, participant) = field_in_an_area();

    participant.set_range(2, 8);
    field.sync_cursor_from_area();

    assert_eq!((field.cursor.selection_anchor(), field.cursor.offset()), (Some(2), 8));
}

#[test]
fn a_collapsed_selection_has_no_anchor() {
    let (field, participant) = field_in_an_area();

    participant.set_range(4, 4);
    field.sync_cursor_from_area();

    assert_eq!((field.cursor.selection_anchor(), field.cursor.offset()), (None, 4));
}

/// A field is asked to mirror the area on every layout pass. While the area says what the field already
/// has, nothing changes, so the view is not taken back to the caret: a user who has scrolled away from
/// the caret keeps the view where it is when the window is resized or activated again.
#[test]
fn mirroring_a_selection_that_is_already_the_cursors_does_not_scroll_back_to_it() {
    let (field, participant) = field_in_an_area();
    participant.set_range(4, 4);
    field.sync_cursor_from_area();
    assert!(field.reveal_caret.get(), "a selection that moved the caret is shown");

    // The view was scrolled away: the reveal was taken, or cancelled by the wheel.
    field.reveal_caret.set(false);
    field.sync_cursor_from_area();
    field.sync_cursor_from_area();
    assert!(!field.reveal_caret.get(), "the same selection, mirrored again, asked for the caret to be shown");

    // A selection that moves the caret is shown again.
    participant.set_range(2, 8);
    field.sync_cursor_from_area();
    assert!(field.reveal_caret.get());
    assert_eq!((field.cursor.selection_anchor(), field.cursor.offset()), (Some(2), 8));
}
