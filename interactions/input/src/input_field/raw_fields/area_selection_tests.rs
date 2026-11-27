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

/// A jump that lands while the press that asked for it is still held, as a force click's does: the area keeps what the
/// press selected, and mirrors it back into the field on every frame, which put the caret back where the press was.
#[test]
fn a_jump_ends_the_press_the_area_holds_and_the_caret_stays_where_it_went() {
    use aimer_events::pointer::PointerSource;
    use aimer_widget::{Drawable, PointerKey};

    let (field, participant) = field_in_an_area();
    let pointer = PointerKey::new(PointerSource::Mouse, 0);
    participant.begin(8, pointer);
    field.sync_cursor_from_area();
    assert_eq!(field.cursor.offset(), 8, "the press put the caret where it was");
    assert_eq!(participant.active_pointer(), Some(pointer), "and the area is holding it");

    field.controller.locate(2..4);
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);
    let _ = super::marker_tests::commands(&field, &ctx);

    assert_eq!(participant.active_pointer(), None, "the jump ended the press");
    assert_eq!(field.controller.value().selection().range(), 2..2, "the caret is on the name");
    assert_eq!((field.cursor.offset(), field.cursor.selection_anchor()), (2, None), "and so is the field's, with nothing selected");
    // The area says the same, so that mirroring it into the field on a later frame keeps the caret where it is.
    field.update(&ctx);
    field.sync_cursor_from_area();
    assert_eq!((field.cursor.offset(), field.cursor.selection_anchor()), (2, None));
}

/// Scrolling or repainting without changing the area's selection must not copy the
/// field's text and recount graphemes from byte zero on every frame.
#[test]
fn unchanged_area_selection_does_not_rescan_graphemes_on_repeat_sync() {
    let (field, participant) = field_in_an_area();
    participant.set_range(4, 4);
    field.sync_cursor_from_area();
    assert_eq!(field.area_sync_scans.get(), 1);

    field.sync_cursor_from_area();
    field.sync_cursor_from_area();
    assert_eq!(
        field.area_sync_scans.get(),
        1,
        "repeat sync with unchanged area endpoints should skip scanning the text"
    );

    // Changing the area's selection triggers a new scan and updates the cursor.
    participant.set_range(2, 8);
    field.sync_cursor_from_area();
    assert_eq!(field.area_sync_scans.get(), 2);
    assert_eq!((field.cursor.selection_anchor(), field.cursor.offset()), (Some(2), 8));
}

#[test]
fn default_window_threshold_windows_medium_large_source_files() {
    let text = "fn sample_line_of_rust_code() -> usize { 42 }\n".repeat(400); // ~18.4 KiB
    let caret = CaretBlink::new();
    let mut config = field_config(TextEditingController::with_text(&text));
    config.max_lines = Some(20);
    let field = RawTextField::with_caret_context(
        config,
        caret.clone(),
        CaretContext::new(caret),
        FocusNode::new(),
        None,
        None,
    );
    assert!(
        field.windowed_display().is_some(),
        "a 400-line (~18 KiB) source file should use windowed layout by default"
    );
}
