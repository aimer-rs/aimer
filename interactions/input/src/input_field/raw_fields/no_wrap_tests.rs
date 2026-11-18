//! A multi-line field that does not wrap: each line is one row however long, and the field scrolls
//! sideways to the caret, as a code editor does.

use aimer_attribute::Vec2d;
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind, TouchPhase};
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, EventElement};

use super::RawTextField;
use super::test_support::{dummy_build_context, focused_multiline_field};
use crate::TextEditingController;

/// Lines of varied length, some far wider than the 300 physical pixels the field is given.
fn code() -> String {
    (0..40)
        .map(|n| match n % 4 {
            0 => format!("fn function_{n}() {{ let a_rather_long_binding_name = some_function_with_a_long_name(argument_one, argument_two); }}\n"),
            1 => format!("    short {n}\n"),
            2 => "\n".to_owned(),
            _ => format!("    // a comment on line {n} that goes on and on and on and on and on and on and on\n"),
        })
        .collect()
}

fn field(text: &str, wrap: bool, caret: usize) -> (RawTextField, BuildContext<'static>) {
    let mut field = focused_multiline_field(TextEditingController::with_text(text), 3);
    field.wrap = wrap;
    field.text_style = field.text_style.text_overflow(aimer_style::TextOverflow::Wrap);
    field.max_lines = Some(8);
    let ctx = dummy_build_context(300.0, 400.0);
    field.cursor.set_offset(caret);
    field.update(&ctx);
    (field, ctx)
}

fn press(field: &RawTextField, key: NamedKey) {
    let _ = field.on_event(&ElementEvent::KeyInput { key, action: KeyAction::Pressed, modifiers: Modifiers::default() });
}

#[test]
fn a_field_that_does_not_wrap_shows_each_line_on_one_row() {
    let text = code();
    let (wrapping, _) = field(&text, true, 0);
    let (plain, _) = field(&text, false, 0);
    let hard_lines = text.matches('\n').count() + 1;

    let rows = |field: &RawTextField| field.geometry_cache.latest().expect("laid out").visual_lines.len();
    assert_eq!(rows(&plain), hard_lines, "one row for each line of the text");
    assert!(rows(&wrapping) > hard_lines, "the same text wraps into more rows: {}", rows(&wrapping));
    let width = plain.geometry_cache.latest().unwrap().text_width;
    assert!(width > 300.0 * 2.0, "the widest line is wider than the field: {width}");
}

#[test]
fn the_field_scrolls_sideways_to_keep_the_caret_in_view() {
    let text = code();
    let first_line_end = text.find('\n').unwrap();
    let (field, ctx) = field(&text, false, 0);
    assert_eq!(field.scroll_x.get(), 0.0);

    // The caret goes to the far end of a long line: the line scrolls to show it.
    field.cursor.set_offset(first_line_end);
    field.reveal_caret.set(true);
    field.update(&ctx);
    assert!(field.scroll_x.get() > 0.0, "scrolled to the end of the line: {}", field.scroll_x.get());
    let scrolled = field.scroll_x.get();

    // And back to the start.
    press(&field, NamedKey::Home);
    field.update(&ctx);
    assert_eq!(field.scroll_x.get(), 0.0, "Home scrolls back to the start of the line (was {scrolled})");
}

#[test]
fn a_field_that_wraps_never_scrolls_sideways() {
    let text = code();
    let (field, ctx) = field(&text, true, text.find('\n').unwrap());
    field.reveal_caret.set(true);
    field.update(&ctx);
    assert_eq!(field.scroll_x.get(), 0.0);
    assert!(!field.on_event(&ElementEvent::Scroll {
        delta: Vec2d { x: -40.0, y: 0.0 },
        kind: ScrollDeltaKind::Pixel,
        phase: TouchPhase::Moved,
        is_direct_manipulation: false,
    }).is_consumed());
}

#[test]
fn a_click_lands_on_the_text_under_the_pointer_when_the_line_is_scrolled() {
    let text = code();
    let (field, ctx) = field(&text, false, text.find('\n').unwrap());
    field.reveal_caret.set(true);
    field.update(&ctx);
    assert!(field.scroll_x.get() > 0.0);
    // The click is at the left edge of the field, which shows text that is far into the line.
    let at = Vec2d { x: 8.0, y: 8.0 };
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
    field.update(&ctx);
    let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, PointerButton::Primary)));
    field.update(&ctx);
    assert!(field.cursor.offset() > 20, "the caret is where the text shows at that edge, not at the start of the line: {}", field.cursor.offset());
}

#[test]
fn the_wheel_scrolls_a_field_that_does_not_wrap_sideways_and_shift_turns_up_and_down_into_left_and_right() {
    let text = code();
    let (field, _) = field(&text, false, 0);
    let scroll = |dx: f32, dy: f32| {
        field
            .on_event(&ElementEvent::Scroll { delta: Vec2d { x: dx, y: dy }, kind: ScrollDeltaKind::Pixel, phase: TouchPhase::Moved, is_direct_manipulation: false })
            .is_consumed()
    };
    assert!(scroll(-60.0, 0.0), "a sideways swipe scrolls");
    assert_eq!(field.scroll_x.get(), 60.0);
    assert!(scroll(20.0, 0.0), "and back");
    assert_eq!(field.scroll_x.get(), 40.0);
    assert!(scroll(1000.0, 0.0));
    assert_eq!(field.scroll_x.get(), 0.0, "it stops at the start");
    Modifiers::set_current(Modifiers { shift: true, ..Modifiers::default() });
    assert!(scroll(0.0, -50.0), "shift and a wheel moves sideways");
    Modifiers::set_current(Modifiers::default());
    assert_eq!(field.scroll_x.get(), 50.0);
}

#[test]
fn a_wheel_scroll_away_from_the_caret_stays_until_the_caret_moves() {
    let text = code();
    let (field, ctx) = field(&text, false, 0);
    let scroll = |dx: f32| {
        field.on_event(&ElementEvent::Scroll { delta: Vec2d { x: dx, y: 0.0 }, kind: ScrollDeltaKind::Pixel, phase: TouchPhase::Moved, is_direct_manipulation: false })
    };
    assert!(scroll(-60.0).is_consumed());
    assert_eq!(field.scroll_x.get(), 60.0);

    // The caret is at the start of the line, off screen to the left. Painting must not drag the
    // text back to it: the wheel is how the rest of the line is looked at without moving the caret.
    field.update(&ctx);
    assert_eq!(field.scroll_x.get(), 60.0, "a repaint keeps the scroll the wheel made");
    assert!(scroll(-60.0).is_consumed());
    field.update(&ctx);
    assert_eq!(field.scroll_x.get(), 120.0, "and the wheel goes on from there");

    // Moving the caret does bring it back into view.
    press(&field, NamedKey::ArrowRight);
    field.update(&ctx);
    assert!(field.scroll_x.get() < 20.0, "the caret is revealed once it moves: {}", field.scroll_x.get());
}

#[test]
fn a_long_text_that_does_not_wrap_is_laid_out_as_a_window_of_single_rows() {
    let text: String = (0..3000).map(|n| format!("let value_{n} = compute_something_long({n}, \"a string argument\", another_function_call(n));\n")).collect();
    let mut field = focused_multiline_field(TextEditingController::with_text(&text), 3);
    field.wrap = false;
    field.text_style = field.text_style.text_overflow(aimer_style::TextOverflow::Wrap);
    field.window_threshold = 0;
    field.max_lines = Some(10);
    let ctx = dummy_build_context(300.0, 400.0);
    field.cursor.set_offset(0);
    field.update(&ctx);

    assert!(field.windowed_display().is_some(), "a long text that does not wrap is windowed too");
    let geometry = field.geometry_cache.latest().expect("laid out");
    let window = geometry.window.as_ref().expect("a window");
    assert_eq!(geometry.visual_lines.len(), window.lines.len(), "one row for each line of the window");
    assert!(geometry.visual_lines.len() < 100, "{} rows, not all 3000", geometry.visual_lines.len());
    assert!(geometry.text_width > 300.0 * 1.3, "the widest line is wider than the field: {}", geometry.text_width);
    // The text is as tall as its lines, one row each.
    let index = field.line_index.borrow().as_ref().cloned().expect("an index");
    let line_height = geometry.interaction.as_ref().unwrap().metrics.line_height;
    assert!((index.total_height() as f32 - 3001.0 * line_height).abs() < 2.0 * line_height, "{} for {} lines", index.total_height(), 3001);
}

#[test]
fn the_text_of_a_line_scrolled_into_view_is_drawn_and_not_cut_at_the_width_of_the_field() {
    use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};

    let text = code();
    let end = text.find('\n').unwrap();
    let (field, ctx) = field(&text, false, end);
    field.reveal_caret.set(true);
    field.update(&ctx);
    assert!(field.scroll_x.get() > 0.0);

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 300.0, 400.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    let widest = field.geometry_cache.latest().expect("laid out").text_width;
    let runs: Vec<Option<f32>> = tree
        .draw_list_snapshot(root)
        .unwrap()
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { bounds_width, .. } => Some(*bounds_width),
            _ => None,
        })
        .collect();
    assert!(!runs.is_empty());
    // The text is laid out as wide as its widest line, so what the scroll brings into view is there
    // to be drawn: a width of the field's own would cut every line at the field's right edge.
    assert!(runs.iter().all(|width| width.is_none_or(|width| width >= widest)), "{runs:?} against a widest line of {widest}");
}
