use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::Vec2d;
use aimer_cupid::draw_cmd::DrawCommand;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, Element, EventElement, FocusNode, LayoutElement, Rebuildable};

use super::marker_tests::commands;
use super::test_support::{dummy_build_context, field_config};
use super::{LabelHit, RawTextField, RawTextFieldHost};
use crate::TextEditingController;
use crate::input_field::caret::CaretBlink;
use crate::input_field::gutter::LineNumbers;
use crate::input_field::line_labels::{FieldSpot, LineLabel, LineLabels};
use aimer_cupid::font::FontFamily;

const TEXT: &str = "alpha\nbeta\ngamma";
const GAP: u32 = 10;

/// A field of three lines with a label after the second: "27 usages", a bar, and "1 implementation", the buttons of which
/// record where they were pressed.
fn field_with_label(
    wrap: bool,
    text: &str,
) -> (RawTextField, Rc<RefCell<Vec<(&'static str, FieldSpot)>>>, BuildContext<'static>) {
    let pressed = Rc::new(RefCell::new(Vec::new()));
    let (usages, implementations) = (Rc::clone(&pressed), Rc::clone(&pressed));
    let label = Rc::new(
        LineLabel::new(Color::Rgba(120, 124, 135, 255))
            .button("27 usages", move |spot| usages.borrow_mut().push(("usages", spot)))
            .text(" | ")
            .button("1 implementation", move |spot| implementations.borrow_mut().push(("implementations", spot))),
    );
    let mut config = field_config(TextEditingController::with_text(text));
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = wrap;
    config.line_labels = Some(LineLabels::new(GAP, move |line| (line == 1).then(|| Rc::clone(&label))));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    (field, pressed, ctx)
}

fn drawn_text(commands: &[DrawCommand], wanted: &str) -> Option<(f32, f32)> {
    commands.iter().find_map(|command| match command {
        DrawCommand::DrawText { text, position, .. } if text.trim() == wanted => Some((position.x, position.y)),
        _ => None,
    })
}

#[test]
fn a_label_is_drawn_after_the_end_of_its_line_and_the_other_lines_have_none() {
    let (field, _, ctx) = field_with_label(true, TEXT);
    let commands = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    // The end of "beta", which is the tenth character of the text.
    let end = layout.geometry.caret_geometry(10).expect("a caret at the end of the line");
    let expected_x = layout.outline_left + layout.padding_left - layout.scroll_x + end.x + GAP as f32;
    let (x, y) = drawn_text(&commands, "27 usages").expect("the first part is drawn");
    assert!((x - expected_x).abs() < 1.0, "the label starts {GAP} after the text: {x} for {expected_x}");
    // Level with the second row, and not the first or the third.
    let second_row = layout.padding_top + layout.line_height;
    assert!(y > second_row && y < second_row + layout.line_height * 1.5, "{y} is on the second row, which starts at {second_row}");
    let (bar_x, bar_y) = drawn_text(&commands, "|").expect("the text between the buttons is drawn");
    assert!(bar_x > x, "the parts follow one another");
    assert!((bar_y - y).abs() < 0.5, "on one baseline");
    let (second_x, _) = drawn_text(&commands, "1 implementation").expect("the second button is drawn");
    assert!(second_x > bar_x);
    // Nothing is drawn for the lines that have no label: the lines have their own text and nothing else is added.
    let labels = commands
        .iter()
        .filter(|command| matches!(command, DrawCommand::DrawText { text, .. } if text.contains("usages") || text.contains("implementation")))
        .count();
    assert_eq!(labels, 2, "two parts are buttons and the other is not a label of its own");
}

#[test]
fn a_label_is_in_the_font_it_asks_for_and_in_the_fields_own_when_it_asks_for_none() {
    let family_of = |commands: &[DrawCommand], wanted: &str| {
        commands.iter().find_map(|command| match command {
            DrawCommand::DrawText { text, font_family, .. } if text.contains(wanted) => Some(*font_family),
            _ => None,
        })
    };
    let (field, _, ctx) = field_with_label(true, TEXT);
    let plain = commands(&field, &ctx);
    let own = family_of(&plain, "beta").expect("the text of the line is drawn");
    assert_eq!(family_of(&plain, "27 usages"), Some(own), "no font asked for: the label is in the field's own");

    // The same field with a label that is in another family: the text of the line is as it was and the label is not.
    let other = if own == FontFamily::SANS_SERIF { FontFamily::MONOSPACE } else { FontFamily::SANS_SERIF };
    let label = Rc::new(LineLabel::new(Color::Rgba(120, 124, 135, 255)).font_family(other).button("27 usages", |_| {}));
    let mut config = field_config(TextEditingController::with_text(TEXT));
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    config.line_labels = Some(LineLabels::new(GAP, move |line| (line == 1).then(|| Rc::clone(&label))));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let asked = commands(&field, &ctx);
    assert_eq!(family_of(&asked, "27 usages"), Some(other), "the font it asked for");
    assert_eq!(family_of(&asked, "beta"), Some(own), "and the text is left alone");
}

#[test]
fn a_label_is_where_the_line_ends_and_not_beyond_the_field_when_it_is_scrolled_away() {
    let (field, _, ctx) = field_with_label(false, "alpha\nbeta that is long enough to be a longer line\ngamma");
    let commands = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let end = layout.geometry.caret_geometry(5 + 1 + "beta that is long enough to be a longer line".len()).expect("a caret");
    let expected_x = layout.outline_left + layout.padding_left - layout.scroll_x + end.x + GAP as f32;
    let (x, _) = drawn_text(&commands, "27 usages").expect("drawn");
    assert!((x - expected_x).abs() < 1.0, "after the longer line: {x} for {expected_x}");
}

#[test]
fn a_line_that_wraps_has_its_label_after_its_last_row_and_only_there() {
    // The second line is long enough to take three rows of the 400 wide field.
    let long = "word ".repeat(40);
    let text = format!("alpha\n{long}\ngamma");
    let (field, _, ctx) = field_with_label(true, &text);
    let commands = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let labels: Vec<f32> = commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { text, position, .. } if text.trim() == "27 usages" => Some(position.y),
            _ => None,
        })
        .collect();
    assert_eq!(labels.len(), 1, "one label: {labels:?}");
    assert!(labels[0] > layout.padding_top + layout.line_height * 2.5, "after at least the third row: {}", labels[0]);
}

#[test]
fn only_the_buttons_of_a_label_are_hit_and_pressing_one_gives_where_it_is() {
    let (field, pressed, ctx) = field_with_label(true, TEXT);
    let _ = commands(&field, &ctx);
    let hits = field.label_hits.borrow().clone();
    assert_eq!(hits.len(), 2, "the buttons and not the text between them");
    assert!(hits[0].x + hits[0].width <= hits[1].x, "in order, with the bar between");
    let at = |hit: &LabelHit| Vec2d { x: hit.x + hit.width / 2.0, y: hit.y + hit.height / 2.0 };
    // The field is at the origin of the window in this test.
    for (hit, which) in hits.iter().zip(["usages", "implementations"]) {
        let press = field.gutter_marker_at(at(hit)).expect("on a button");
        press();
        let last = pressed.borrow().last().copied().expect("it was called");
        assert_eq!(last.0, which);
        assert_eq!((last.1.x, last.1.y, last.1.width, last.1.height), (hit.x, hit.y, hit.width, hit.height), "with where the button is");
        assert!(last.1.extent.is_some(), "and how far the field goes");
    }
    let bar = Vec2d { x: hits[0].x + hits[0].width + 2.0, y: hits[0].y + hits[0].height / 2.0 };
    assert!(field.gutter_marker_at(bar).is_none(), "the bar between the buttons is not one");
    let before = Vec2d { x: hits[0].x - 3.0, y: hits[0].y + hits[0].height / 2.0 };
    assert!(field.gutter_marker_at(before).is_none(), "nor is the text before the label");
}

#[test]
fn a_press_on_a_button_is_the_buttons_and_leaves_the_caret_and_the_release_alone() {
    let (field, pressed, ctx) = field_with_label(true, TEXT);
    let controller = field.controller.clone();
    let _ = commands(&field, &ctx);
    let hit = field.label_hits.borrow()[0].clone();
    let on_button = Vec2d { x: hit.x + hit.width / 2.0, y: hit.y + hit.height / 2.0 };
    let caret = controller.cursor_position();

    let host = RawTextFieldHost::new(field, super::embed_tests::caret_stub());
    let root = host.boxed();
    root.sync_paint_state(&ctx);
    root.update(&ctx);
    let press = |at: Vec2d, event: ElementEvent| aimer_widget::dispatch_event(root.as_ref(), at, &event);
    let mouse = |at: Vec2d| PointerInfo::mouse(at, PointerButton::Primary);

    assert!(press(on_button, ElementEvent::PointerDown(mouse(on_button))).is_consumed(), "the press is the button's");
    assert_eq!(pressed.borrow().len(), 1);
    assert!(press(on_button, ElementEvent::PointerUp(mouse(on_button))).is_consumed(), "so is the release");
    assert_eq!(pressed.borrow().len(), 1, "once");
    assert_eq!(controller.cursor_position(), caret, "the caret did not move");
}

#[test]
fn a_field_without_labels_has_no_buttons() {
    let mut config = field_config(TextEditingController::with_text(TEXT));
    config.max_lines = Some(8);
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    assert!(field.label_hits.borrow().is_empty());
    assert!(field.gutter_marker_at(Vec2d { x: 50.0, y: 20.0 }).is_none());
}

#[test]
fn a_label_that_is_asked_for_is_asked_only_for_lines_in_view() {
    let asked = Rc::new(RefCell::new(std::collections::BTreeSet::new()));
    let seen = Rc::clone(&asked);
    let long: String = (0..200).map(|line| format!("line {line}\n")).collect();
    let mut config = field_config(TextEditingController::with_text(long));
    config.max_lines = Some(8);
    config.wrap = false;
    config.line_labels = Some(LineLabels::new(8, move |line| {
        seen.borrow_mut().insert(line);
        None
    }));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    let asked = asked.borrow();
    assert!(!asked.is_empty() && asked.len() < 40, "a screenful and not the file: {} lines", asked.len());
    assert!(asked.contains(&0));
    let _ = Cell::new(0);
}

const REST: Color = Color::Rgba(200, 60, 110, 255);
const HOT: Color = Color::Rgba(255, 150, 190, 255);
/// The same colors as the bytes a draw command has them in.
const REST_BYTES: [u8; 4] = [200, 60, 110, 255];
const HOT_BYTES: [u8; 4] = [255, 150, 190, 255];

/// A field with a label of two buttons that go bright when the pointer is on them, and the label's buttons' places.
fn field_with_hover() -> (RawTextField, Vec<LabelHit>, BuildContext<'static>) {
    let label = Rc::new(
        LineLabel::new(REST)
            .hover_color(HOT)
            .button("27 usages", |_| {})
            .text(" | ")
            .button("1 implementation", |_| {}),
    );
    let mut config = field_config(TextEditingController::with_text(TEXT));
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = true;
    config.line_labels = Some(LineLabels::new(GAP, move |line| (line == 1).then(|| Rc::clone(&label))));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    let hits = field.label_hits.borrow().clone();
    (field, hits, ctx)
}

fn center(hit: &LabelHit) -> Vec2d {
    Vec2d { x: hit.x + hit.width / 2.0, y: hit.y + hit.height / 2.0 }
}

fn color_of(commands: &[DrawCommand], wanted: &str) -> Option<[u8; 4]> {
    commands.iter().find_map(|command| match command {
        DrawCommand::DrawText { text, color, .. } if text.trim() == wanted => Some(color.to_rgba8().0),
        _ => None,
    })
}

/// The thin rectangles in `color`: what underlines.
fn underlines(commands: &[DrawCommand], color: [u8; 4]) -> Vec<(f32, f32, f32, f32)> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { rect, color: fill, .. } if fill.to_rgba8().0 == color && rect.height <= 2.0 => {
                Some((rect.x, rect.y, rect.width, rect.height))
            }
            _ => None,
        })
        .collect()
}

fn move_to(field: &RawTextField, at: Vec2d) {
    let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(at, PointerButton::Primary)));
}

#[test]
fn a_button_under_the_pointer_is_in_its_hover_color_and_underlined_and_the_others_are_not() {
    let (field, hits, ctx) = field_with_hover();
    let rest = commands(&field, &ctx);
    assert_eq!(color_of(&rest, "27 usages"), Some(REST_BYTES), "at rest it is in the color of the label");
    assert!(underlines(&rest, HOT_BYTES).is_empty() && underlines(&rest, REST_BYTES).is_empty(), "and not underlined");

    move_to(&field, center(&hits[0]));
    assert!(field.paint_needs_recording(&ctx), "the pointer coming onto a button is a reason to draw it again");
    let hot = commands(&field, &ctx);
    assert_eq!(color_of(&hot, "27 usages"), Some(HOT_BYTES), "the button the pointer is on");
    assert_eq!(color_of(&hot, "1 implementation"), Some(REST_BYTES), "and not the other");
    let lines = underlines(&hot, HOT_BYTES);
    assert_eq!(lines.len(), 1, "one underline: {lines:?}");
    let (x, y, width, _) = lines[0];
    assert!((x - hits[0].x).abs() < 1.0 && (width - hits[0].width).abs() < 1.0, "as wide as the button: {lines:?} for {:?}", (hits[0].x, hits[0].width));
    assert!(y > hits[0].y + hits[0].height * 0.5 && y < hits[0].y + hits[0].height + 4.0, "under its text, not through it: {y}");

    // On the other button instead.
    move_to(&field, center(&hits[1]));
    assert!(field.paint_needs_recording(&ctx));
    let other = commands(&field, &ctx);
    assert_eq!((color_of(&other, "27 usages"), color_of(&other, "1 implementation")), (Some(REST_BYTES), Some(HOT_BYTES)));

    // Off the label, and off the field: back as it was, and drawn again to be.
    move_to(&field, Vec2d { x: 5.0, y: 5.0 });
    assert!(field.paint_needs_recording(&ctx));
    assert_eq!(color_of(&commands(&field, &ctx), "1 implementation"), Some(REST_BYTES));
    move_to(&field, center(&hits[0]));
    let _ = commands(&field, &ctx);
    let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
    assert!(field.paint_needs_recording(&ctx), "the pointer going from the field is too");
    assert_eq!(color_of(&commands(&field, &ctx), "27 usages"), Some(REST_BYTES));
}

#[test]
fn a_label_that_asks_for_no_hover_color_is_not_changed_by_the_pointer() {
    let (field, _, ctx) = field_with_label(true, TEXT);
    let _ = commands(&field, &ctx);
    let hits = field.label_hits.borrow().clone();
    move_to(&field, center(&hits[0]));
    let drawn = commands(&field, &ctx);
    assert_eq!(color_of(&drawn, "27 usages"), Some([120, 124, 135, 255]));
    assert!(underlines(&drawn, [120, 124, 135, 255]).is_empty());
}

#[test]
fn the_pointer_is_a_pointer_over_a_button_of_a_label_and_the_text_cursor_over_the_text() {
    use aimer_utils::cursor::{Cursor, CursorIcon, restore_thread_cursor_handler, set_thread_cursor_handler};
    let requests = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&requests);
    let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));

    let (field, hits, _ctx) = field_with_hover();
    move_to(&field, center(&hits[0]));
    let over_button = requests.borrow().last().cloned();
    // The text of the first line, a little in from the left of the text.
    move_to(&field, Vec2d { x: hits[0].x - 60.0, y: hits[0].y + hits[0].height / 2.0 });
    let over_text = requests.borrow().last().cloned();
    restore_thread_cursor_handler(previous);

    assert_eq!(over_button, Some(Cursor::Icon(CursorIcon::Pointer)), "a button is pointed at");
    assert_eq!(over_text, Some(Cursor::Icon(CursorIcon::Text)), "and the text is not");
}

#[test]
fn the_hover_color_is_kept_by_the_label_and_a_label_that_has_none_says_so() {
    assert_eq!(LineLabel::new(REST).hover_color(HOT).hover(), Some(HOT));
    assert_eq!(LineLabel::new(REST).hover(), None);
}

#[test]
fn a_label_is_placed_after_an_end_of_line_inlay_hint_without_overlapping_it() {
    use crate::input_field::inlay::{Inlay, Inlays};

    let pressed = Rc::new(Cell::new(false));
    let pressed_in = Rc::clone(&pressed);
    let label = Rc::new(
        LineLabel::new(Color::Rgba(120, 124, 135, 255))
            .button("2 usages", move |_| pressed_in.set(true)),
    );
    let mut config = field_config(TextEditingController::with_text(TEXT));
    config.line_numbers = Some(LineNumbers::new());
    config.max_lines = Some(8);
    config.wrap = false;
    config.line_labels = Some(LineLabels::new(GAP, move |line| (line == 1).then(|| Rc::clone(&label))));
    // End of line 1 ("beta") is byte 10 ("alpha\nbeta").
    let eol_offset = 10;
    config.inlays = Some(Inlays::new(vec![
        Inlay::new(eol_offset, "&AsciiSet", Color::Rgba(238, 190, 212, 255))
            .padded(true, false)
            .background(Color::Rgba(255, 92, 150, 76)),
    ]));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);

    let drawn = commands(&field, &ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let place = layout
        .geometry
        .inlays
        .iter()
        .find(|p| p.at == eol_offset && p.eol)
        .expect("end-of-line inlay is placed");
    let (gap_left, _) = layout.geometry.inlay_origin(place, layout.scale).expect("inlay origin");
    let inlay_right = (layout.outline_left + layout.padding_left - layout.scroll_x + gap_left) / layout.scale + place.gap;
    let (label_x, _) = drawn_text(&drawn, "2 usages").expect("label is drawn");
    assert!(
        label_x >= inlay_right + GAP as f32 - 1.0,
        "label at {label_x} overlaps end-of-line inlay ending at {inlay_right} (gap {GAP})"
    );

    let hits = field.label_hits.borrow().clone();
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].x >= inlay_right + GAP as f32 - 1.0,
        "label hit box at {} should match shifted label position past {inlay_right}",
        hits[0].x
    );
    let over_inlay = Vec2d {
        x: inlay_right - place.gap * 0.5,
        y: hits[0].y + hits[0].height / 2.0,
    };
    assert!(
        field.gutter_marker_at(over_inlay).is_none(),
        "the inlay hint itself is not covered by the label button"
    );
    let press = field
        .gutter_marker_at(center(&hits[0]))
        .expect("shifted label button is hit");
    press();
    assert!(pressed.get(), "pressing the shifted label button fires its callback");
}
