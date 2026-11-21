//! Where a text is scrolled to, remembered by its controller: a view of the text that goes and comes back, as a tab does, is
//! where it was.

use aimer_animation::AnimInstant;
use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, EventElement, FocusNode};

use super::marker_tests::commands;
use super::test_support::{dummy_build_context, field_config};
use super::RawTextField;
use crate::input_field::caret::CaretBlink;
use crate::input_field::gutter::LineNumbers;
use crate::TextEditingController;

fn field_on(controller: &TextEditingController, wrap: bool) -> (RawTextField, BuildContext<'static>) {
    let mut config = field_config(controller.clone());
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = wrap;
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    (field, ctx)
}

fn frame(field: &RawTextField, ctx: &BuildContext<'_>) {
    field.update(ctx);
    let _ = commands(field, ctx);
}

fn wheel(field: &RawTextField, ctx: &BuildContext<'_>, dx: f32, dy: f32) {
    let _ = field.on_event(&ElementEvent::Scroll { delta: Vec2d { x: dx, y: dy }, kind: ScrollDeltaKind::Pixel, phase: TouchPhase::Moved, is_direct_manipulation: false });
    frame(field, ctx);
}

/// A text of 200 lines, with the caret at its start.
fn tall() -> TextEditingController {
    let controller = TextEditingController::with_text((0..200).map(|n| format!("line {n}\n")).collect::<String>());
    controller.select_bytes(0, 0);
    controller
}

#[test]
fn a_controller_remembers_where_its_text_is_scrolled_to_and_that_is_not_an_edit() {
    let controller = TextEditingController::with_text("abc");
    assert_eq!(controller.scroll_offset(), (0.0, 0.0), "at the start until told");
    let revision = controller.revision();
    controller.set_scroll_offset((12.0, 340.0));
    assert_eq!(controller.scroll_offset(), (12.0, 340.0));
    assert_eq!(controller.clone().scroll_offset(), (12.0, 340.0), "a clone is the same controller");
    assert_eq!(controller.revision(), revision, "what is shown is not what is written: no undo, no listener");
    controller.set_scroll_offset((f32::NAN, -5.0));
    assert_eq!(controller.scroll_offset(), (0.0, 0.0), "what is not a place to scroll to is the start");
}

#[test]
fn a_field_that_is_scrolled_tells_its_controller_where() {
    let controller = tall();
    let (field, ctx) = field_on(&controller, false);
    assert_eq!(controller.scroll_offset().1, 0.0);
    wheel(&field, &ctx, 0.0, -300.0);
    let at = field.scroll_y.get();
    assert!(at > 100.0, "the field scrolled: {at}");
    assert_eq!(controller.scroll_offset().1, at, "and its controller knows");
}

#[test]
fn a_new_field_on_the_same_controller_is_scrolled_where_the_one_before_was_and_not_pulled_to_the_caret() {
    let controller = tall();
    let (first, ctx) = field_on(&controller, false);
    wheel(&first, &ctx, 0.0, -600.0);
    let was = first.scroll_y.get();
    assert!(was > 300.0, "{was}");
    assert_eq!(controller.value().selection().range(), 0..0, "the caret is at the start of the text, far from where it is scrolled to");
    drop(first);
    // The tab is shown again: a new field, on the controller that stayed.
    let (second, ctx) = field_on(&controller, false);
    frame(&second, &ctx);
    assert_eq!(second.scroll_y.get(), was, "where it was");
    assert_eq!(controller.scroll_offset().1, was);
}

#[test]
fn a_text_scrolled_sideways_comes_back_scrolled_sideways() {
    let controller = TextEditingController::with_text(format!("{}\nshort", "wide ".repeat(200)));
    controller.select_bytes(0, 0);
    let (first, ctx) = field_on(&controller, false);
    wheel(&first, &ctx, -400.0, 0.0);
    let was = first.scroll_x.get();
    assert!(was > 100.0, "{was}");
    drop(first);
    let (second, ctx) = field_on(&controller, false);
    frame(&second, &ctx);
    assert_eq!(second.scroll_x.get(), was);
}

#[test]
fn an_offset_further_than_the_text_goes_is_held_to_it() {
    let controller = TextEditingController::with_text("one\ntwo");
    controller.set_scroll_offset((5000.0, 9000.0));
    let (field, ctx) = field_on(&controller, false);
    frame(&field, &ctx);
    assert_eq!(field.scroll_y.get(), 0.0, "a text that is not tall has nowhere to scroll to");
    assert_eq!(controller.scroll_offset().1, 0.0, "and the controller is told so");
}

#[test]
fn a_new_text_that_was_never_scrolled_still_shows_its_caret_as_it_always_did() {
    // The caret is at the end of the text, which is far below the first screen.
    let controller = TextEditingController::with_text((0..200).map(|n| format!("line {n}\n")).collect::<String>());
    let (field, ctx) = field_on(&controller, false);
    frame(&field, &ctx);
    assert!(field.scroll_y.get() > 1000.0, "the caret is shown: {}", field.scroll_y.get());
}
