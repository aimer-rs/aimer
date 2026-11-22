//! The boxes round what the caret is next to, such as a bracket and the one it matches: drawn behind the text while the caret
//! is there, and marked for a moment when what they are round changes.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use aimer_animation::AnimInstant;
use aimer_cupid::draw_cmd::DrawCommand;
use aimer_events::element::ElementEvent;
use aimer_text_buffer::TextSnapshot;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, EventElement, FocusNode, Rebuildable};

use super::marker_tests::commands;
use super::test_support::{dummy_build_context, field_config};
use super::RawTextField;
use crate::input_field::caret::CaretBlink;
use crate::input_field::caret_marks::{CaretMark, CaretMarks};
use crate::TextEditingController;

const PINK: Color = Color::Rgba(255, 92, 150, 255);
const TEAL: Color = Color::Rgba(84, 205, 236, 255);

/// Text of `a(b)c`, in which the caret after the `)` is next to the pair `(` at 1 and `)` at 3. The marks are asked for
/// with what they were given and counted.
fn field() -> (RawTextField, TextEditingController, BuildContext<'static>, Rc<Cell<usize>>) {
    let asked = Rc::new(Cell::new(0));
    let counted = Rc::clone(&asked);
    let controller = TextEditingController::with_text("a(b)c");
    let mut config = field_config(controller.clone());
    config.max_lines = Some(4);
    config.wrap = false;
    config.caret_marks = Some(CaretMarks::new(move |text: &TextSnapshot, selection: Range<usize>| {
        counted.set(counted.get() + 1);
        assert!(text.len() >= 5 && text.byte_at(1) == Some(b'('), "the whole text, as a snapshot");
        if selection == (4..4) {
            vec![CaretMark::new(1..2, PINK), CaretMark::new(3..4, TEAL)]
        } else {
            Vec::new()
        }
    }));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    (field, controller, ctx, asked)
}

fn frame(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
    field.update(ctx);
    commands(field, ctx)
}

fn move_caret(field: &RawTextField, controller: &TextEditingController, at: usize) {
    controller.select_bytes(at, at);
    field.sync_cursor_from_controller();
}

/// The rectangles with a border, as `(border color, size)`: what a box or a pulse is drawn as.
fn boxes(commands: &[DrawCommand]) -> Vec<((u8, u8, u8), (f32, f32))> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { rect, border_width, border_color, .. } if border_width.iter().any(|width| *width > 0.0) => {
                Some(((border_color.r, border_color.g, border_color.b), (rect.width, rect.height)))
            }
            _ => None,
        })
        .collect()
}

fn later(field: &RawTextField, by: Duration) {
    field.test_clock.set(Some(field.test_clock.get().expect("the test clock is set") + by));
}

#[test]
fn nothing_is_drawn_where_the_caret_is_next_to_nothing_and_the_closure_is_given_the_text_and_the_selection() {
    let (field, controller, ctx, asked) = field();
    move_caret(&field, &controller, 0);
    assert!(boxes(&frame(&field, &ctx)).is_empty());
    assert!(asked.get() >= 1, "the closure was asked");
    assert_eq!(field.marks_phase(), None, "and there is nothing to mark");
}

#[test]
fn a_box_is_drawn_round_each_range_in_its_color_and_a_mark_goes_over_each_as_they_appear() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    let first = boxes(&frame(&field, &ctx));
    assert_eq!(first.len(), 2, "a mark for each of the two: {first:?}");
    assert_eq!(first.iter().filter(|(color, _)| *color == (255, 92, 150)).count(), 1, "the first range is in its color");
    assert_eq!(first.iter().filter(|(color, _)| *color == (84, 205, 236)).count(), 1, "and the second in its own");
    assert!(field.marks_phase().is_some_and(|phase| phase < 0.1), "the mark has begun");
    later(&field, Duration::from_secs(5));
    let settled = boxes(&frame(&field, &ctx));
    assert_eq!(settled.len(), 2, "only the boxes stay: {settled:?}");
    assert_eq!(field.marks_phase(), None, "the mark is over");
}

#[test]
fn the_mark_fades_and_the_boxes_do_not() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    let alpha_of = |commands: &[DrawCommand]| -> Vec<u8> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { border_width, border_color, .. } if border_width.iter().any(|width| *width > 0.0) => Some(border_color.a),
                _ => None,
            })
            .collect()
    };
    let early = alpha_of(&frame(&field, &ctx));
    assert_eq!(early[0], 255, "starts at 1.0 opacity");
    later(&field, Duration::from_millis(500));
    let middle = alpha_of(&frame(&field, &ctx));
    assert!(middle[0] < early[0], "the mark has faded towards 0.3: {early:?} then {middle:?}");
    later(&field, Duration::from_secs(5));
    let settled = alpha_of(&frame(&field, &ctx));
    assert_eq!(settled.len(), 2);
    assert!(settled.iter().all(|alpha| *alpha > 0), "a box is there for as long as the caret is: {settled:?}");
    assert_eq!(settled[0], 77, "the box line has 0.3 opacity");
}

#[test]
fn the_mark_does_not_begin_again_while_the_caret_stays_and_does_when_it_leaves_and_comes_back() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    later(&field, Duration::from_millis(300));
    frame(&field, &ctx);
    let middle = field.marks_phase().expect("going");
    assert!(middle > 0.3, "{middle}: it was not begun again by drawing again");
    later(&field, Duration::from_secs(5));
    frame(&field, &ctx);
    assert_eq!(field.marks_phase(), None);
    // A frame that is the same marks, long after, does not mark them again.
    frame(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "the same marks are not new");
    move_caret(&field, &controller, 0);
    assert!(boxes(&frame(&field, &ctx)).is_empty(), "gone with the caret");
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    assert!(field.marks_phase().is_some_and(|phase| phase < 0.1), "back: marked again");
}

#[test]
fn a_field_without_focus_has_no_boxes() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    let _ = field.on_event(&ElementEvent::FocusLost);
    assert!(boxes(&frame(&field, &ctx)).is_empty());
    assert_eq!(field.marks_phase(), None);
}

#[test]
fn the_closure_is_not_asked_again_for_a_text_and_a_caret_it_was_asked_about() {
    let (field, controller, ctx, asked) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    let after_first = asked.get();
    for _ in 0..5 {
        later(&field, Duration::from_millis(20));
        frame(&field, &ctx);
    }
    assert_eq!(asked.get(), after_first, "frames of the animation, and of the blink, ask nothing");
    controller.replace_range(5..5, "d");
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    assert!(asked.get() > after_first, "an edit asks again");
}

#[test]
fn a_range_past_the_end_of_the_text_is_drawn_as_far_as_the_text_goes_and_none_is_no_box() {
    let controller = TextEditingController::with_text("ab");
    let mut config = field_config(controller.clone());
    config.max_lines = Some(4);
    config.caret_marks = Some(CaretMarks::new(|_, _| vec![CaretMark::new(1..40, PINK), CaretMark::new(30..40, PINK), CaretMark::new(1..1, PINK)]));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let drawn = boxes(&commands(&field, &ctx));
    assert_eq!(drawn.len(), 1, "one mark before 60%, for the one range that has any text: {drawn:?}");
    later(&field, Duration::from_secs(5));
    let settled = boxes(&frame(&field, &ctx));
    assert_eq!(settled.len(), 1, "one box when settled: {settled:?}");
}

#[test]
fn a_painted_field_is_painted_again_while_the_mark_goes_and_for_a_moved_caret_and_not_after() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    assert!(!field.paint_needs_recording(&ctx), "just painted");
    later(&field, Duration::from_millis(200));
    assert!(field.paint_needs_recording(&ctx), "the mark has moved on: what was painted is not what is to be");
    frame(&field, &ctx);
    later(&field, Duration::from_millis(200));
    assert!(field.paint_needs_recording(&ctx), "and on");
    later(&field, Duration::from_secs(5));
    assert!(field.paint_needs_recording(&ctx), "it is over: the field is painted without it");
    frame(&field, &ctx);
    assert!(!field.paint_needs_recording(&ctx), "and then it rests, though time goes by");
    later(&field, Duration::from_secs(5));
    assert!(!field.paint_needs_recording(&ctx));
    move_caret(&field, &controller, 0);
    assert!(field.paint_needs_recording(&ctx), "the caret moved: the boxes are to go");
}

/// A pointer over the field, and then off it, as a mouse that goes by does.
fn pointer_in(field: &RawTextField, ctx: &BuildContext<'_>) {
    use aimer_attribute::position::Vec2d;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    field.cached_bounds.save(1.0, 0.0, 0.0, 400.0, 300.0);
    let at = Vec2d { x: 40.0, y: 10.0 };
    let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(at, PointerButton::Primary)));
    let _ = frame(field, ctx);
}

fn pointer_out(field: &RawTextField, ctx: &BuildContext<'_>) {
    use aimer_events::pointer::PointerSource;
    let _ = field.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
    let _ = frame(field, ctx);
}

#[test]
fn a_pointer_that_enters_and_leaves_the_field_does_not_mark_what_was_marked_already() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    later(&field, Duration::from_secs(5));
    frame(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "the mark is over");
    pointer_in(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "the pointer came in");
    later(&field, Duration::from_millis(50));
    pointer_out(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "the pointer went out");
}

#[test]
fn a_field_that_is_built_again_keeps_the_mark_it_was_showing_and_does_not_begin_it_again() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    later(&field, Duration::from_secs(5));
    frame(&field, &ctx);
    // The widget is built again, as it is when something around it changes, and takes what the field before it knew.
    let mut config = field_config(controller.clone());
    config.max_lines = Some(4);
    config.wrap = false;
    config.caret_marks = field.caret_marks.clone();
    let rebuilt = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    rebuilt.test_clock.set(field.test_clock.get());
    rebuilt.adopt_runtime_state_from(&field as &dyn aimer_widget::Element);
    rebuilt.update(&ctx);
    let drawn = boxes(&commands(&rebuilt, &ctx));
    assert_eq!(rebuilt.marks_phase(), None, "no mark: it was shown already");
    assert_eq!(drawn.len(), 2, "only the boxes: {drawn:?}");
}

#[test]
fn a_box_and_a_mark_at_the_edge_of_the_text_are_kept_inside_it_and_not_cut_by_the_clip() {
    // The `(` is the first character of the first line: a box round it, and a mark that begins larger, reach past the text's
    // edge on the left and above, where the clip of the content would cut them.
    let controller = TextEditingController::with_text("(a)");
    let mut config = field_config(controller.clone());
    config.max_lines = Some(4);
    config.wrap = false;
    config.line_numbers = Some(crate::input_field::gutter::LineNumbers::new());
    config.caret_marks = Some(CaretMarks::new(|_, selection| if selection == (1..1) { vec![CaretMark::new(0..1, PINK)] } else { Vec::new() }));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    let _ = commands(&field, &ctx);
    move_caret(&field, &controller, 1);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let (left, top) = (layout.padding_left, layout.padding_top);
    let rects = |commands: &[DrawCommand]| -> Vec<(f32, f32)> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { rect, border_width, .. } if border_width.iter().any(|width| *width > 0.0) => Some((rect.x, rect.y)),
                _ => None,
            })
            .collect()
    };
    let early = rects(&frame(&field, &ctx));
    assert_eq!(early.len(), 1, "a mark during pulse");
    for (x, y) in early {
        assert!(x >= left - 0.01 && y >= top - 0.01, "({x}, {y}) is inside the content, which begins at ({left}, {top})");
    }
    later(&field, Duration::from_secs(5));
    for (x, y) in rects(&frame(&field, &ctx)) {
        assert!(x >= left - 0.01 && y >= top - 0.01, "the box that stays: ({x}, {y})");
    }
}

#[test]
fn what_a_field_knows_of_its_marks_is_copied_to_the_next_one_and_not_taken_from_the_one_before() {
    let (field, controller, ctx, _) = field();
    move_caret(&field, &controller, 4);
    frame(&field, &ctx);
    later(&field, Duration::from_secs(5));
    frame(&field, &ctx);
    // A field is built again more than once from the same one before it, as it is while the pointer goes over what holds it:
    // the second takes the same as the first, which is that the marks are not new.
    for round in 0..3 {
        let mut config = field_config(controller.clone());
        config.max_lines = Some(4);
        config.wrap = false;
        config.caret_marks = field.caret_marks.clone();
        let rebuilt = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
        rebuilt.test_clock.set(field.test_clock.get());
        rebuilt.adopt_runtime_state_from(&field as &dyn aimer_widget::Element);
        rebuilt.update(&ctx);
        let _ = commands(&rebuilt, &ctx);
        assert_eq!(rebuilt.marks_phase(), None, "round {round}: the marks are not new");
    }
}

#[test]
fn typing_near_a_bracket_does_not_begin_the_mark_pulse() {
    let controller = TextEditingController::with_text("a(b)c");
    let mut config = field_config(controller.clone());
    config.max_lines = Some(4);
    config.wrap = false;
    config.caret_marks = Some(CaretMarks::new(move |text: &TextSnapshot, selection: Range<usize>| {
        if (1..=text.len()).contains(&selection.start) {
            vec![CaretMark::new(1..2, PINK)]
        } else {
            Vec::new()
        }
    }));
    let field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    field.test_clock.set(Some(AnimInstant::now()));
    let _ = field.on_event(&ElementEvent::FocusGained);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    move_caret(&field, &controller, 3);
    frame(&field, &ctx);
    assert!(field.marks_phase().is_some(), "moving caret onto bracket triggers mark pulse");
    later(&field, Duration::from_secs(5));
    frame(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "mark pulse settled");

    // Typing near the bracket edits the text:
    controller.insert_str("d", 3);
    field.sync_cursor_from_controller();
    frame(&field, &ctx);
    assert_eq!(field.marks_phase(), None, "typing near bracket does not trigger mark pulse");
}

#[test]
fn brace_highlight_starts_transition_at_60_percent_progress_and_settles_at_point_three_opacity() {
    let (field, controller, ctx, _) = field();
    let border_alphas = |commands: &[DrawCommand]| -> Vec<u8> {
        commands
            .iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { border_width, border_color, .. } if border_width.iter().any(|width| *width > 0.0) => Some(border_color.a),
                _ => None,
            })
            .collect()
    };
    move_caret(&field, &controller, 4);

    // First frame anchors pulse at t0. At 0ms (0% progress), 2 pulse marks at 1.0 opacity (alpha 255).
    let at_start = border_alphas(&frame(&field, &ctx));
    assert_eq!(at_start.len(), 2);
    assert_eq!(at_start[0], 255, "1.0 opacity at start");

    // DURATION = 650ms. 60% is 390ms.
    // At 300ms (~46% progress), phase < 0.6: still at 1.0 opacity (alpha 255).
    later(&field, Duration::from_millis(300));
    let pre_60 = border_alphas(&frame(&field, &ctx));
    assert_eq!(pre_60[0], 255, "still at 1.0 opacity before 60%");

    // At 500ms (~77% progress), phase >= 0.6: transitioning down towards 0.3.
    later(&field, Duration::from_millis(200));
    let during = border_alphas(&frame(&field, &ctx));
    assert!(during[0] < 255 && during[0] > 77, "transitioning from 1.0 down to 0.3: {}", during[0]);

    // Once settled (after 650ms): pulse is over, boxes settle at 0.3 opacity (77).
    later(&field, Duration::from_secs(2));
    let settled = border_alphas(&frame(&field, &ctx));
    assert_eq!(settled.len(), 2, "only the 2 highlight boxes stay");
    assert_eq!(settled[0], 77, "box line has 0.3 opacity after animation end");
    assert_eq!(settled[1], 77, "box line has 0.3 opacity after animation end");
}

