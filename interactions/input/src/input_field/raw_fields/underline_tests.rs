//! A field with underlines: lines under ranges of its text, drawn straight with rounded ends, in the color they are given.

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_style::TextOverflow;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, LayoutElement};

use super::RawTextField;
use super::test_support::{dummy_build_context, focused_multiline_field};
use crate::input_field::controller::TextFieldController;
use crate::input_field::underline::{Underline, Underlines};

const RED: Color = Color::Rgba(235, 64, 64, 255);
const YELLOW: Color = Color::Rgba(235, 190, 60, 255);
const TEXT: &str = "let x = 1;\nlet y = 2;\nlet z = 3;";

fn field(text: &str, underlines: Option<Underlines>) -> RawTextField {
    let mut field = focused_multiline_field(TextFieldController::with_initial(text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.underlines = underlines;
    // A field that is made with its text has the caret at the end of it, and scrolls there.
    field.cursor.set_offset(0);
    field
}

fn under(range: std::ops::Range<usize>, color: Color) -> Underlines {
    Underlines::new(vec![Underline::new(range, color)])
}

/// A line that was drawn: where it is, how big, how round and what color.
#[derive(Clone, Debug, PartialEq)]
struct Bar {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    rgba: [u8; 4],
}

/// The bars `field` paints: its rounded rectangles that have no border, in the order they were drawn.
fn bars(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<Bar> {
    field.update(ctx);
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 200.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    tree.draw_list_snapshot(root)
        .unwrap()
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { rect, color, border_radius, border_width, .. } if border_radius[0] > 0.0 && border_width[0] == 0.0 => {
                Some(Bar { x: rect.x, y: rect.y, width: rect.width, height: rect.height, radius: border_radius[0], rgba: color.to_rgba8().0 })
            }
            _ => None,
        })
        .collect()
}

fn rgba(color: Color) -> [u8; 4] {
    let (red, green, blue, alpha) = color.to_rgba();
    [red, green, blue, alpha]
}

#[test]
fn a_range_has_a_straight_line_under_it_with_rounded_ends_in_its_color() {
    let ctx = dummy_build_context(400.0, 200.0);
    let found = bars(&field(TEXT, Some(under(4..5, RED))), &ctx);
    assert_eq!(found.len(), 1, "{found:?}");
    let bar = &found[0];
    assert_eq!(bar.rgba, rgba(RED));
    assert_eq!(bar.height, Underlines::DEFAULT_THICKNESS);
    assert_eq!(bar.radius, Underlines::DEFAULT_RADIUS, "round, and not so round that the ends are a half circle of more than the line");
    assert!(bar.width > 2.0 && bar.width < 12.0, "as wide as `x` is: {}", bar.width);
}

#[test]
fn the_line_is_as_wide_as_the_range_and_under_it_and_not_under_the_rest_of_the_line() {
    let ctx = dummy_build_context(400.0, 200.0);
    let one = bars(&field(TEXT, Some(under(4..5, RED))), &ctx).remove(0);
    let word = bars(&field(TEXT, Some(under(0..3, RED))), &ctx).remove(0);
    assert!(word.width > one.width * 2.0, "`let` is wider than `x`: {} and {}", word.width, one.width);
    assert!(word.x < one.x, "and it begins before it");
    assert!((word.y - one.y).abs() < 0.01, "on the same line");
    let second = bars(&field(TEXT, Some(under(15..16, RED))), &ctx).remove(0);
    assert!(second.y > one.y + 5.0, "a line of text lower: {} and {}", second.y, one.y);
}

#[test]
fn a_range_on_more_than_one_line_has_a_line_under_each_part_of_it() {
    let ctx = dummy_build_context(400.0, 200.0);
    // From `x` of the first line to `y` of the second.
    let found = bars(&field(TEXT, Some(under(4..16, YELLOW))), &ctx);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[1].y > found[0].y, "the second is under the second line");
    assert!(found[1].x < found[0].x, "and begins at the start of it");
    assert!(found.iter().all(|bar| bar.rgba == rgba(YELLOW)));
}

#[test]
fn a_set_of_lines_is_drawn_in_the_order_it_was_given() {
    let ctx = dummy_build_context(400.0, 200.0);
    let set = Underlines::new(vec![Underline::new(4..9, YELLOW), Underline::new(4..9, RED)]);
    let found = bars(&field(TEXT, Some(set)), &ctx);
    assert_eq!(found.iter().map(|bar| bar.rgba).collect::<Vec<_>>(), [rgba(YELLOW), rgba(RED)], "the later is over the earlier");
}

#[test]
fn a_thicker_set_is_drawn_thicker() {
    let ctx = dummy_build_context(400.0, 200.0);
    let found = bars(&field(TEXT, Some(under(4..5, RED).thickness(4.0).radius(10.0))), &ctx);
    assert_eq!(found[0].height, 4.0);
    assert_eq!(found[0].radius, 2.0, "no more than half of the thickness");
}

#[test]
fn what_has_no_range_or_none_in_the_text_has_no_line() {
    let ctx = dummy_build_context(400.0, 200.0);
    assert!(bars(&field(TEXT, None), &ctx).is_empty());
    assert!(bars(&field(TEXT, Some(Underlines::new(Vec::new()))), &ctx).is_empty());
    assert!(bars(&field(TEXT, Some(under(5..5, RED))), &ctx).is_empty(), "an empty range");
    assert!(bars(&field(TEXT, Some(under(500..600, RED))), &ctx).is_empty(), "past the end");
    assert_eq!(bars(&field(TEXT, Some(under(28..600, RED))), &ctx).len(), 1, "and what is past the end is cut off");
}

#[test]
fn a_field_that_hides_its_text_has_no_lines() {
    let ctx = dummy_build_context(400.0, 200.0);
    let mut secret = field("hunter2", Some(under(0..3, RED)));
    secret.input_type = super::InputType::Password;
    assert!(bars(&secret, &ctx).is_empty());
}

#[test]
fn the_lines_stay_under_their_text_while_text_is_typed_before_it_and_a_range_typed_in_is_longer() {
    let ctx = dummy_build_context(400.0, 200.0);
    let made_for = aimer_text_buffer::TextSnapshot::from_text(TEXT);
    let moved = field(TEXT, Some(under(4..5, RED).of_text(made_for)));
    moved.controller.insert_str("ab", 0);
    let after = bars(&moved, &ctx).remove(0);
    let plain = bars(&field(&format!("ab{TEXT}"), Some(under(6..7, RED))), &ctx).remove(0);
    assert!((after.x - plain.x).abs() < 0.01, "{} and {}", after.x, plain.x);
    // Typed in the middle of the range, it is longer.
    let made_for = aimer_text_buffer::TextSnapshot::from_text(TEXT);
    let typed = field(TEXT, Some(under(0..3, RED).of_text(made_for)));
    let before = bars(&typed, &ctx).remove(0);
    typed.controller.insert_str("lll", 1);
    assert!(bars(&typed, &ctx)[0].width > before.width, "`let` of `lllet`");
}
