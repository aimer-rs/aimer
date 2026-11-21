//! The name the pointer rests on: told to the app after the pointer has been still on it for a moment, and taken back when
//! the pointer goes, or something else is done, so that what the app shows for it is not left behind.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::AnimInstant;
use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind, TouchPhase};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, EventElement, FocusNode, Rebuildable};

use super::marker_tests::commands;
use super::test_support::{dummy_build_context, field_config};
use super::RawTextField;
use crate::input_field::caret::CaretBlink;
use crate::input_field::gutter::LineNumbers;
use crate::input_field::hover::{HoverSpot, TextHover};
use crate::TextEditingController;

const DELAY: Duration = Duration::from_millis(300);

type Told = Rc<RefCell<Vec<Option<HoverSpot>>>>;

/// A field of `text` that tells what it is told of the name the pointer rests on, painted once.
fn field_of(text: &str, range_at: Option<Rc<dyn Fn(usize) -> Option<Range<usize>>>>) -> (RawTextField, Told, BuildContext<'static>) {
    let told: Told = Rc::default();
    let record = Rc::clone(&told);
    let controller = TextEditingController::with_text(text);
    let mut config = field_config(controller);
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    let mut hover = TextHover::new(DELAY, move |spot| record.borrow_mut().push(spot));
    if let Some(range_at) = range_at {
        hover = hover.range_at(move |byte| range_at(byte));
    }
    config.hover = Some(hover);
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    (field, told, ctx)
}

fn at_line(field: &RawTextField, ctx: &BuildContext<'_>, line: f32, x: f32) -> Vec2d {
    let layout = field.local_v2_layout(ctx).expect("laid out");
    Vec2d { x, y: layout.padding_top + layout.line_height * (line + 0.5) }
}

fn frame(field: &RawTextField, ctx: &BuildContext<'_>) {
    field.update(ctx);
    let _ = commands(field, ctx);
}

fn move_to(field: &RawTextField, ctx: &BuildContext<'_>, at: Vec2d) {
    let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(at, PointerButton::Primary)));
    frame(field, ctx);
}

fn wait(field: &RawTextField, ctx: &BuildContext<'_>, by: Duration) {
    field.test_clock.set(Some(field.test_clock.get().expect("the test clock is set") + by));
    frame(field, ctx);
}

fn shown(told: &Told) -> Vec<Option<Range<usize>>> {
    told.borrow().iter().map(|spot| spot.as_ref().map(|spot| spot.range.clone())).collect()
}

#[test]
fn a_name_the_pointer_rests_on_is_told_after_the_delay_and_not_before() {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    assert!(told.borrow().is_empty(), "the pointer has only just come");
    wait(&field, &ctx, DELAY / 2);
    assert!(told.borrow().is_empty(), "and the delay is not over");
    wait(&field, &ctx, DELAY);
    assert_eq!(shown(&told), [Some(0..5)], "the name, once");
    wait(&field, &ctx, DELAY * 5);
    assert_eq!(told.borrow().len(), 1, "not told again while it rests there");
}

#[test]
fn what_is_told_says_where_the_name_is_in_the_field_in_logical_pixels() {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 1.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY * 2);
    let spot = told.borrow()[0].clone().expect("told");
    assert_eq!(spot.range, 11..16);
    assert!(spot.x >= layout.padding_left - 0.5 && spot.x < layout.padding_left + 2.0, "it starts where the text does: {} for padding {}", spot.x, layout.padding_left);
    assert!((spot.y - (layout.padding_top + layout.line_height)).abs() < 1.5, "on the second line: {spot:?}");
    assert!(spot.width > 10.0 && spot.width < 120.0 && (spot.height - layout.line_height).abs() < 1.5, "as wide as five letters, and a line tall: {spot:?}");
    let (right, bottom) = spot.extent.expect("the field has been laid out, and says where its text ends");
    assert!(right > spot.x + spot.width && bottom >= spot.y + spot.height, "what is beside the name has room to be kept inside: {spot:?}");
}

#[test]
fn moving_within_the_name_keeps_it_and_moving_to_another_takes_it_back_and_tells_the_new_one_after_the_delay() {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY * 2);
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 14.0));
    wait(&field, &ctx, DELAY * 2);
    assert_eq!(shown(&told), [Some(0..5)], "another place in the same name is the same name");
    move_to(&field, &ctx, at_line(&field, &ctx, 1.0, layout.padding_left + 6.0));
    assert_eq!(shown(&told), [Some(0..5), None], "gone as the pointer leaves it");
    wait(&field, &ctx, DELAY / 2);
    assert_eq!(told.borrow().len(), 2, "and the next waits its delay");
    wait(&field, &ctx, DELAY);
    assert_eq!(shown(&told), [Some(0..5), None, Some(11..16)]);
}

#[test]
fn a_pointer_that_passes_over_a_name_without_resting_tells_nothing() {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY / 3);
    move_to(&field, &ctx, at_line(&field, &ctx, 1.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY / 3);
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, 380.0));
    wait(&field, &ctx, DELAY * 3);
    assert!(told.borrow().is_empty(), "{:?}", told.borrow());
}

#[test]
fn where_there_is_no_name_or_the_app_says_there_is_none_nothing_is_told() {
    let (field, told, ctx) = field_of("alpha beta", None);
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, 380.0));
    wait(&field, &ctx, DELAY * 2);
    assert!(told.borrow().is_empty(), "past the end of the line");
    // The app knows `beta` is not a name that has anything to say.
    let (field, told, ctx) = field_of("alpha beta", Some(Rc::new(|byte| (byte < 5).then_some(0..5))));
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 56.0));
    wait(&field, &ctx, DELAY * 2);
    assert!(told.borrow().is_empty(), "the app's say: {:?}", told.borrow());
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY * 2);
    assert_eq!(shown(&told), [Some(0..5)]);
}

/// A field with a name that is told, to be taken back.
fn told_field() -> (RawTextField, Told, BuildContext<'static>, Vec2d) {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let at = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
    move_to(&field, &ctx, at);
    wait(&field, &ctx, DELAY * 2);
    assert_eq!(shown(&told), [Some(0..5)]);
    (field, told, ctx, at)
}

#[test]
fn the_pointer_leaving_the_field_takes_the_name_back() {
    let (field, told, ctx, _) = told_field();
    let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
    frame(&field, &ctx);
    assert_eq!(shown(&told), [Some(0..5), None]);
    wait(&field, &ctx, DELAY * 5);
    assert_eq!(told.borrow().len(), 2, "and it is not told again with no pointer");
}

#[test]
fn a_press_a_key_a_scroll_or_losing_the_focus_takes_the_name_back_and_it_stays_back_until_the_pointer_moves() {
    let press = |field: &RawTextField, at: Vec2d| {
        let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, PointerButton::Primary)));
    };
    let key = |field: &RawTextField, _: Vec2d| {
        let _ = field.on_event(&ElementEvent::KeyInput { key: NamedKey::ArrowDown, action: KeyAction::Pressed, modifiers: Modifiers::default() });
    };
    let scroll = |field: &RawTextField, _: Vec2d| {
        let _ = field.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -10.0 },
            kind: ScrollDeltaKind::Pixel,
            phase: TouchPhase::Moved,
            is_direct_manipulation: false,
        });
    };
    let blur = |field: &RawTextField, _: Vec2d| {
        let _ = field.on_event(&ElementEvent::FocusLost);
    };
    let actions: [(&str, &dyn Fn(&RawTextField, Vec2d)); 4] = [("a press", &press), ("a key", &key), ("a scroll", &scroll), ("losing the focus", &blur)];
    for (what, act) in actions {
        let (field, told, ctx, at) = told_field();
        act(&field, at);
        frame(&field, &ctx);
        assert_eq!(shown(&told), [Some(0..5), None], "{what} takes it back");
        wait(&field, &ctx, DELAY * 5);
        assert_eq!(told.borrow().len(), 2, "{what}: it is not told again while the pointer is still");
        let layout = field.local_v2_layout(&ctx).expect("laid out");
        // The field may have scrolled or lost the focus: put the pointer on the name again by the position it has now.
        move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 8.0));
        wait(&field, &ctx, DELAY * 2);
        assert!(told.borrow().len() >= 2, "{what}");
    }
}

#[test]
fn a_press_is_not_followed_by_the_name_until_the_pointer_has_moved() {
    let (field, told, ctx, at) = told_field();
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
    let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, PointerButton::Primary)));
    frame(&field, &ctx);
    wait(&field, &ctx, DELAY * 5);
    assert_eq!(shown(&told), [Some(0..5), None]);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 12.0));
    wait(&field, &ctx, DELAY * 2);
    assert_eq!(shown(&told), [Some(0..5), None, Some(0..5)], "a move says the pointer is there to be looked at");
}

#[test]
fn an_edit_takes_the_name_back_because_what_was_told_of_is_not_what_is_there() {
    let (field, told, ctx, _) = told_field();
    field.controller.replace_range(0..0, "x");
    frame(&field, &ctx);
    assert_eq!(shown(&told), [Some(0..5), None]);
}

#[test]
fn a_field_built_again_while_a_name_is_told_does_not_tell_it_again() {
    let (field, told, ctx, _) = told_field();
    for round in 0..3 {
        let record = Rc::clone(&told);
        let mut config = field_config(field.controller.clone());
        config.line_numbers = Some(LineNumbers::new());
        config.max_lines = Some(8);
        config.wrap = true;
        config.hover = Some(TextHover::new(DELAY, move |spot| record.borrow_mut().push(spot)));
        let rebuilt = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        rebuilt.test_clock.set(field.test_clock.get());
        rebuilt.adopt_runtime_state_from(&field as &dyn aimer_widget::Element);
        frame(&rebuilt, &ctx);
        wait(&rebuilt, &ctx, DELAY * 2);
        assert_eq!(told.borrow().len(), 1, "round {round}: {:?}", told.borrow());
    }
}

#[test]
fn a_field_without_a_hover_is_not_asked_anything_and_a_pointer_that_drags_a_selection_tells_nothing() {
    let controller = TextEditingController::with_text("alpha beta");
    let mut config = field_config(controller);
    config.max_lines = Some(4);
    let plain = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let ctx = dummy_build_context(400.0, 300.0);
    plain.update(&ctx);
    move_to(&plain, &ctx, Vec2d { x: 20.0, y: 10.0 });
    // A button held down is a drag: what it passes over is not something to say of.
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let start = at_line(&field, &ctx, 1.0, layout.padding_left + 2.0);
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(start, PointerButton::Primary)));
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY * 3);
    assert!(told.borrow().is_empty(), "{:?}", told.borrow());
}

#[test]
fn a_field_that_never_had_the_focus_says_where_it_ends_too() {
    let (field, told, ctx) = field_of("alpha beta\ngamma", None);
    let _ = field.on_event(&ElementEvent::FocusLost);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    move_to(&field, &ctx, at_line(&field, &ctx, 0.0, layout.padding_left + 6.0));
    wait(&field, &ctx, DELAY * 2);
    let spot = told.borrow()[0].clone().expect("told");
    let bounds = field.cached_bounds.get_bounds().expect("painted");
    assert_eq!(spot.extent, Some((bounds.width, bounds.height)), "the box of the field, and not what a caret knows of it");
    assert!(spot.extent.is_some_and(|(right, _)| right >= 399.0), "{spot:?}");
}

#[test]
fn a_key_that_the_app_takes_does_not_dismiss_what_is_shown_and_one_that_it_leaves_to_the_field_does() {
    let dismissed = Rc::new(std::cell::Cell::new(0));
    let counted = Rc::clone(&dismissed);
    let (field, _, ctx) = field_of("alpha beta\ngamma", None);
    let hover = field.hover.clone().expect("a hover").on_dismiss(move || counted.set(counted.get() + 1));
    let mut config = field_config(field.controller.clone());
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    config.hover = Some(hover);
    // The app takes the down arrow, as a list that is open does: it moves in it, and the list is not to go.
    config.key_filter = Some(Rc::new(|key, _| matches!(key, NamedKey::ArrowDown)));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    let key = |key| ElementEvent::KeyInput { key, action: KeyAction::Pressed, modifiers: Modifiers::default() };
    let _ = field.on_event(&key(NamedKey::ArrowDown));
    assert_eq!(dismissed.get(), 0, "taken by the app: what the app shows is for the app to keep or shut");
    let _ = field.on_event(&key(NamedKey::ArrowUp));
    assert_eq!(dismissed.get(), 1, "left to the field, which moves the caret: it is a key in the text");
}

#[test]
fn what_is_shown_is_dismissed_at_once_by_what_the_user_does_and_not_by_the_pointer_leaving() {
    let dismissed = Rc::new(std::cell::Cell::new(0));
    let counted = Rc::clone(&dismissed);
    let (field, _, ctx) = field_of("alpha beta\ngamma", None);
    // The same field, with a hook for it.
    let hover = field.hover.clone().expect("a hover").on_dismiss(move || counted.set(counted.get() + 1));
    let mut config = field_config(field.controller.clone());
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    config.hover = Some(hover);
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let at = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
    move_to(&field, &ctx, at);
    wait(&field, &ctx, DELAY * 2);
    // The pointer going is not a dismissal: what is shown stays for the pointer to get to it.
    let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
    assert_eq!(dismissed.get(), 0);
    // A press in the text is, and so is a key and a wheel, whether or not a name is shown.
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
    assert_eq!(dismissed.get(), 1);
    let _ = field.on_event(&ElementEvent::KeyInput { key: NamedKey::ArrowDown, action: KeyAction::Pressed, modifiers: Modifiers::default() });
    assert_eq!(dismissed.get(), 2);
    // Losing the focus is not: a press on what the app shows, such as a row of a list that is open over the text, takes the focus from
    // the field, and the list is not to go before the press is done.
    let _ = field.on_event(&ElementEvent::FocusLost);
    assert_eq!(dismissed.get(), 2, "the app is not told to take away what it shows because the field lost the focus");
}

#[test]
fn losing_the_focus_takes_back_the_name_that_was_told_without_telling_the_app_to_take_away_what_it_shows() {
    let told = Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = Rc::clone(&told);
    let dismissed = Rc::new(std::cell::Cell::new(0));
    let counted = Rc::clone(&dismissed);
    let (field, _, ctx) = field_of("alpha beta\ngamma", None);
    let hover = TextHover::new(DELAY, move |spot| seen.borrow_mut().push(spot.is_some())).on_dismiss(move || counted.set(counted.get() + 1));
    let mut config = field_config(field.controller.clone());
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    config.hover = Some(hover);
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let at = at_line(&field, &ctx, 0.0, layout.padding_left + 6.0);
    move_to(&field, &ctx, at);
    wait(&field, &ctx, DELAY * 2);
    assert_eq!(told.borrow().last(), Some(&true), "a name is told");
    let _ = field.on_event(&ElementEvent::FocusLost);
    assert_eq!(told.borrow().last(), Some(&false), "and taken back when the focus goes");
    assert_eq!(dismissed.get(), 0);
}
