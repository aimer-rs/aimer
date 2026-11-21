//! A field that lays out a window of its text: what it paints, where it scrolls, and that
//! what the user does with it still lands on the text.

use std::rc::Rc;
use std::sync::Arc;

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_style::TextOverflow;
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, EventElement, LayoutElement};

use super::RawTextField;
use super::test_support::{commit, dummy_build_context, focused_multiline_field, key};
use crate::editable_text::{LineIndex, LineMetrics};
use crate::input_field::controller::TextFieldController;

/// A note of `lines` hard lines: some short, some long enough to wrap in the width used.
fn note(lines: usize) -> String {
    (0..lines)
        .map(|index| match index % 4 {
            0 => format!("Line {index}: a short one\n"),
            1 => format!(
                "Line {index}: this one has enough words in it that it has to wrap across the \
                 width of the pane more than once, which is what long notes do\n"
            ),
            2 => "\n".to_owned(),
            _ => format!("Line {index}: and a middling one, about so long\n"),
        })
        .collect()
}

/// A field that scrolls inside ten lines, laying out a window of its text from
/// `threshold` bytes of it.
fn field(text: &str, threshold: usize) -> RawTextField {
    let controller = TextFieldController::with_initial(text);
    let mut field = focused_multiline_field(controller, 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.max_lines = Some(10);
    field.window_threshold = threshold;
    field
}

/// What `field` paints as text: each run's text, where it starts and the height of its box.
fn painted(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<(String, f32, Option<f32>)> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 800.0, 600.0)).unwrap();
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
            DrawCommand::DrawText { text, position, bounds_height, .. } => {
                Some((text.to_string(), position.y, *bounds_height))
            }
            _ => None,
        })
        .collect()
}

/// A frame: the field is asked to lay out and draw, as it is on each of the app's.
fn frame(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<(String, f32, Option<f32>)> {
    field.update(ctx);
    painted(field, ctx)
}

fn index_of(field: &RawTextField) -> Rc<LineIndex> {
    field.line_index.borrow().as_ref().cloned().expect("the field windows its text")
}

fn ctrl(name: &str) -> ElementEvent {
    ElementEvent::KeyInput {
        key: NamedKey::Other(name.to_owned()),
        action: KeyAction::Pressed,
        modifiers: Modifiers { ctrl: true, ..Modifiers::default() },
    }
}

#[test]
fn a_long_note_is_laid_out_and_painted_as_a_window() {
    let text = note(3000);
    assert!(text.len() > 100_000);
    let field = field(&text, 64 * 1024);
    // The caret is put at the start, so what is painted is the top of the note.
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(800.0, 600.0);

    let runs = frame(&field, &ctx);

    assert!(!runs.is_empty(), "the field paints its text");
    let painted_bytes = runs.iter().map(|run| run.0.len()).sum::<usize>();
    assert!(
        painted_bytes < text.len() / 5,
        "{painted_bytes} bytes were painted of {}: the window, not the note",
        text.len()
    );
    assert!(runs[0].0.starts_with("Line 0:"), "{:?}", &runs[0].0[..20]);
    let index = index_of(&field);
    let window = field.window_lines.get().expect("a window");
    assert_eq!(window.0, 0);
    assert!(window.1 < index.len() / 4, "{window:?} of {} lines", index.len());
}

#[test]
fn a_note_below_the_threshold_is_not_windowed() {
    let text = note(40);
    let field = field(&text, 64 * 1024);
    let ctx = dummy_build_context(800.0, 600.0);
    let runs = frame(&field, &ctx);
    assert!(field.line_index.borrow().is_none());
    assert_eq!(runs.iter().map(|run| run.0.as_str()).collect::<String>(), text);
}

#[test]
fn a_window_that_reaches_the_whole_text_paints_what_the_whole_layout_paints() {
    // Short enough that the margin of the window, a screen or so each way, holds all of it.
    let text = note(16);
    let whole = field(&text, usize::MAX);
    let windowed = field(&text, 0);
    let ctx = dummy_build_context(800.0, 600.0);
    for caret_at_start in [true, false] {
        if caret_at_start {
            whole.cursor.set_offset(0);
            windowed.cursor.set_offset(0);
        }
        let want = frame(&whole, &ctx);
        let got = frame(&windowed, &ctx);
        assert_eq!(got.len(), want.len());
        for (got, want) in got.iter().zip(&want) {
            assert_eq!(got.0, want.0, "the same text");
            assert!((got.1 - want.1).abs() < 0.05, "drawn at {} and at {}", got.1, want.1);
        }
        assert!((windowed.scroll_y.get() - whole.scroll_y.get()).abs() < 0.5);
    }
}

#[test]
fn the_caret_at_the_end_is_revealed_and_its_lines_painted() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    let ctx = dummy_build_context(800.0, 600.0);

    // A caret at the end, where `with_initial` puts it, is scrolled to on the first frames.
    let mut runs = frame(&field, &ctx);
    for _ in 0..3 {
        runs = frame(&field, &ctx);
    }

    let last = text.trim_end().lines().last().unwrap();
    let painted = runs.iter().map(|run| run.0.as_str()).collect::<String>();
    assert!(painted.contains(last), "the last line was not painted: {:?}", &painted[painted.len().saturating_sub(80)..]);
    let index = index_of(&field);
    let viewport_bottom = f64::from(field.scroll_y.get()) + 10.0 * f64::from(index.metrics().line_height);
    assert!(
        viewport_bottom >= index.total_height() - 2.0 * f64::from(index.metrics().line_height),
        "scrolled to {} of {}",
        field.scroll_y.get(),
        index.total_height()
    );
}

#[test]
fn scrolling_to_the_middle_paints_the_middle() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&field, &ctx);
    let index = index_of(&field);
    let middle = index.total_height() / 2.0;

    field.reveal_caret.set(false);
    field.scroll_y.set(middle as f32);
    let runs = frame(&field, &ctx);

    let first = runs[0].0.lines().next().unwrap();
    let number: usize = first.strip_prefix("Line ").unwrap().split(':').next().unwrap().parse().unwrap();
    assert!((1000..2000).contains(&number), "painted from {first:?}");
    let window = field.window_lines.get().unwrap();
    let viewport = 10.0 * f64::from(index.metrics().line_height);
    assert!(index.top(window.0) <= f64::from(field.scroll_y.get()));
    assert!(index.top(window.1.min(index.len() - 1)) >= f64::from(field.scroll_y.get()) + viewport);
}

/// A field that fills the height it is given (`expand`) and scrolls inside it, with no cap on its
/// lines: a code editor's. Nothing outside it says what is shown, so the window follows its own
/// scroll.
fn expanding_field(text: &str) -> RawTextField {
    let controller = TextFieldController::with_initial(text);
    let mut field = focused_multiline_field(controller, 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.max_lines = None;
    field.expand = super::ExpandDirection::Vertical;
    field.window_threshold = 64 * 1024;
    field.cursor.set_offset(0);
    field.reveal_caret.set(false);
    field
}

/// The number in the first painted line that has one (some of the note's lines are empty).
fn first_line_number(runs: &[(String, f32, Option<f32>)]) -> usize {
    runs.iter()
        .flat_map(|run| run.0.lines())
        .find_map(|line| line.strip_prefix("Line ")?.split(':').next()?.parse().ok())
        .expect("a numbered line is painted")
}

#[test]
fn an_expanding_windowed_field_paints_what_it_is_scrolled_to() {
    let text = note(3000);
    let field = expanding_field(&text);
    let ctx = dummy_build_context(800.0, 600.0);
    let top = frame(&field, &ctx);
    assert_eq!(first_line_number(&top), 0);
    let index = index_of(&field);
    assert!(index.total_height() > 30_000.0);

    for fraction in [0.25, 0.5, 0.9] {
        field.scroll_y.set((index.total_height() * fraction) as f32);
        let runs = frame(&field, &ctx);
        let number = first_line_number(&runs);
        assert!(
            (number as f64) > 3000.0 * fraction * 0.8 - 100.0,
            "scrolled to {fraction} of the note, painted from line {number}"
        );
        let window = field.window_lines.get().unwrap();
        assert!(index.top(window.0) <= f64::from(field.scroll_y.get()), "{window:?}");
        assert!(
            index.top(window.1.min(index.len() - 1)) >= f64::from(field.scroll_y.get()) + 600.0,
            "the window {window:?} does not reach the bottom of what is shown"
        );
    }
}

#[test]
fn an_expanding_windowed_field_follows_its_viewport_when_it_grows_and_shrinks() {
    // The window was laid out for a small viewport, which then grows (a window made full screen) and
    // shrinks again: the lines laid out always cover what is shown.
    let text = note(3000);
    let field = expanding_field(&text);
    let small = dummy_build_context(800.0, 300.0);
    frame(&field, &small);
    let big = dummy_build_context(800.0, 3000.0);
    for (ctx, viewport) in [(&big, 3000.0_f64), (&small, 300.0), (&big, 3000.0)] {
        field.scroll_y.set(0.0);
        frame(&field, ctx);
        field.scroll_y.set(2000.0);
        frame(&field, ctx);
        let index = index_of(&field);
        let window = field.window_lines.get().unwrap();
        assert!(
            index.top(window.1.min(index.len() - 1)) >= 2000.0 + viewport,
            "viewport {viewport}: window {window:?} stops at {}",
            index.top(window.1.min(index.len() - 1))
        );
    }
}

#[test]
fn regaining_focus_does_not_scroll_back_to_the_caret() {
    // The window is deactivated and activated again (full screen does it): the field is told it lost
    // and regained focus, and the view stays where the user scrolled it to, not on the caret.
    for threshold in [64 * 1024, usize::MAX] {
        let text = note(3000);
        let field = field(&text, threshold);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(800.0, 600.0);
        frame(&field, &ctx);
        field.reveal_caret.set(false);
        field.scroll_y.set(3000.0);
        frame(&field, &ctx);
        let scrolled = field.scroll_y.get();
        assert!(scrolled > 1000.0, "{scrolled}");

        field.set_focused(false);
        field.set_focused(true);
        frame(&field, &ctx);
        assert_eq!(field.scroll_y.get(), scrolled, "threshold {threshold}: focus brought the view back to the caret");
    }
}

#[test]
fn a_click_or_a_key_still_scrolls_to_the_caret() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&field, &ctx);
    field.reveal_caret.set(false);
    field.scroll_y.set(3000.0);
    frame(&field, &ctx);
    // Typing puts the caret on the line being typed on, wherever the view was.
    let _ = field.on_event(&commit("x"));
    frame(&field, &ctx);
    assert!(field.scroll_y.get() < 3000.0, "{}", field.scroll_y.get());
}

#[test]
fn the_text_stays_put_when_what_was_estimated_above_it_is_corrected() {
    let text = note(1500);
    let field = field(&text, 64 * 1024);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&field, &ctx);
    let before = index_of(&field);

    // An index that thinks graphemes are four times as wide as they are, so every
    // line above the viewport is much taller than it is.
    let metrics = LineMetrics { advance: before.metrics().advance * 4.0, ..before.metrics() };
    let wrong = Rc::new(LineIndex::new(before.text().clone(), metrics));
    *field.line_index.borrow_mut() = Some(Rc::clone(&wrong));
    field.window_lines.set(None);
    field.window_anchor.set(None);

    field.reveal_caret.set(false);
    let far = wrong.total_height() as f32 * 0.6;
    field.scroll_y.set(far);
    frame(&field, &ctx);
    let after_far = wrong.total_height();

    // Scroll up to lines that have never been laid out: their heights are corrected
    // as the window reaches them, and the viewport must not move on the text.
    let target = field.scroll_y.get() - 900.0;
    let line = wrong.line_at_y(f64::from(target));
    let offset_in_line = f64::from(target) - wrong.top(line);
    field.scroll_y.set(target);
    let version = wrong.version();
    frame(&field, &ctx);

    assert!(wrong.version() > version, "lines above were laid out and corrected");
    assert!((wrong.total_height() - after_far).abs() > 50.0, "the estimates were off");
    let now = wrong.top(line) + offset_in_line - f64::from(field.scroll_y.get());
    assert!(now.abs() < 0.5, "the line that was at the top moved by {now}px");
}

#[test]
fn the_scroll_extent_is_the_wholes_once_the_lines_are_laid_out() {
    let text = note(400);
    let whole = field(&text, usize::MAX);
    let windowed = field(&text, 0);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&whole, &ctx);
    frame(&windowed, &ctx);

    // Scroll through the note so every line is laid out.
    windowed.reveal_caret.set(false);
    let mut at = 0.0_f32;
    while at < whole.scroll_y_extent.get() + 2000.0 {
        windowed.scroll_y.set(at);
        frame(&windowed, &ctx);
        at += 500.0;
    }
    frame(&windowed, &ctx);

    assert!(
        (windowed.scroll_y_extent.get() - whole.scroll_y_extent.get()).abs() < 1.0,
        "{} against {}",
        windowed.scroll_y_extent.get(),
        whole.scroll_y_extent.get()
    );
}

#[test]
fn typing_at_the_end_of_a_long_note_lands_at_the_end_and_undoes() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    let ctx = dummy_build_context(800.0, 600.0);
    for _ in 0..4 {
        frame(&field, &ctx);
    }
    let end = field.controller.grapheme_count();
    field.cursor.set_offset(end);

    let _ = field.on_event(&commit("XYZ"));
    let mut runs = frame(&field, &ctx);
    runs = frame(&field, &ctx).into_iter().chain(runs).collect::<Vec<_>>();
    assert!(field.controller.text().ends_with("XYZ"));
    assert!(runs.iter().any(|run| run.0.ends_with("XYZ")), "the typed text is painted");

    let _ = field.on_event(&ctrl("z"));
    frame(&field, &ctx);
    assert!(!field.controller.text().ends_with("XYZ"), "undone");
    assert_eq!(field.controller.text(), text);
}

#[test]
fn select_all_and_end_need_no_layout_of_the_note() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    let ctx = dummy_build_context(800.0, 600.0);
    field.cursor.set_offset(0);
    frame(&field, &ctx);
    let laid_out = field.window_lines.get().unwrap();

    let _ = field.on_event(&ctrl("a"));
    let runs = frame(&field, &ctx);

    assert_eq!(field.cursor.selection_anchor(), Some(0));
    assert_eq!(field.cursor.offset(), field.controller.grapheme_count());
    assert!(!runs.is_empty());
    // The selection reaches the end; the caret is revealed there; the window follows.
    let after = field.window_lines.get().unwrap();
    assert!(after.1 - after.0 < index_of(&field).len() / 4, "{laid_out:?} then {after:?}");
    let _ = key(NamedKey::End);
}

#[test]
fn the_box_of_a_windowed_text_covers_its_window_and_not_the_note() {
    let text = note(3000);
    let field = field(&text, 64 * 1024);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(800.0, 600.0);
    let runs = frame(&field, &ctx);
    let index = index_of(&field);
    let height = runs[0].2.expect("a wrapped run declares its box");
    assert!(height < index.total_height() as f32 / 4.0, "{height} for a note {}px tall", index.total_height());
    let window = field.window_lines.get().unwrap();
    let window_height = index.top(window.1.min(index.len() - 1)) - index.top(window.0);
    assert!(f64::from(height) >= window_height * 0.9, "{height} for a window {window_height}px tall");
}

/// Scrolls `field` through its whole text, so that every line has been laid out and its
/// heights are exact, then back to the top.
fn lay_everything_out(field: &RawTextField, ctx: &BuildContext<'_>) {
    field.reveal_caret.set(false);
    let mut at = 0.0_f32;
    frame(field, ctx);
    while at < field.scroll_y_extent.get() + 1500.0 {
        field.scroll_y.set(at);
        frame(field, ctx);
        at += 400.0;
    }
    field.scroll_y.set(0.0);
    frame(field, ctx);
}

#[test]
fn clicks_and_drags_select_what_the_whole_layout_selects() {
    use aimer_attribute::Vec2d;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let text = note(500);
    let whole = field(&text, usize::MAX);
    let windowed = field(&text, 0);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&whole, &ctx);
    lay_everything_out(&windowed, &ctx);
    assert!(
        (windowed.scroll_y_extent.get() - whole.scroll_y_extent.get()).abs() < 1.0,
        "the fields must agree on the height before their pointers can"
    );

    for scroll in [0.0_f32, 1234.0, 3000.0, whole.scroll_y_extent.get()] {
        for field in [&whole, &windowed] {
            field.reveal_caret.set(false);
            field.scroll_y.set(scroll);
            frame(field, &ctx);
        }
        for (x, y) in [(4.0, 6.0), (150.0, 40.0), (400.0, 100.0), (60.0, 170.0)] {
            let mut cursors = Vec::new();
            for field in [&whole, &windowed] {
                let down = Vec2d { x, y };
                let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(down, PointerButton::Primary)));
                field.update(&ctx);
                let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(down, PointerButton::Primary)));
                field.update(&ctx);
                cursors.push((field.cursor.offset(), field.cursor.selection_anchor()));
            }
            assert_eq!(cursors[0], cursors[1], "a click at ({x}, {y}) with the note scrolled to {scroll}");
        }

        // A drag from one place to another.
        let mut selections = Vec::new();
        for field in [&whole, &windowed] {
            let from = Vec2d { x: 30.0, y: 20.0 };
            let to = Vec2d { x: 220.0, y: 150.0 };
            let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(from, PointerButton::Primary)));
            field.update(&ctx);
            let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(to, PointerButton::Primary)));
            field.update(&ctx);
            let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(to, PointerButton::Primary)));
            field.update(&ctx);
            selections.push((field.cursor.selection_anchor(), field.cursor.offset()));
        }
        assert_eq!(selections[0], selections[1], "a drag with the note scrolled to {scroll}");
    }
}

#[test]
fn a_field_that_grows_with_its_text_lays_out_what_its_parent_shows() {
    let text = note(3000);
    let controller = TextFieldController::with_initial(&text);
    let mut field = focused_multiline_field(controller, 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = 64 * 1024;
    field.cursor.set_offset(0);
    field.reveal_caret.set(false);

    let mut ctx = dummy_build_context(800.0, 600.0);
    // A scrolling parent gives the field all the height it wants.
    ctx.box_constraint.max_height = 1_000_000.0;
    ctx.parent_size.height = 1_000_000.0;

    ctx.set_visible_rect(Some((0.0, 0.0, 800.0, 600.0)));
    frame(&field, &ctx);
    let total = index_of(&field).total_height() as f32;
    assert!(total > 30_000.0, "{total}");

    let mut windows = Vec::new();
    for shown in [(0.0, 600.0), (total * 0.3, 600.0), (total * 0.8, 600.0)] {
        ctx.set_visible_rect(Some((0.0, shown.0, 800.0, shown.1)));
        let runs = frame(&field, &ctx);
        let index = index_of(&field);
        let window = field.window_lines.get().unwrap();
        // The lines laid out are the lines shown, with room around them.
        assert!(index.top(window.0) <= f64::from(shown.0), "{window:?} for {shown:?}");
        assert!(
            index.top(window.1.min(index.len() - 1)) >= f64::from(shown.0 + shown.1),
            "{window:?} for {shown:?}"
        );
        assert!(!runs.is_empty());
        let painted = runs.iter().map(|run| run.0.len()).sum::<usize>();
        assert!(painted < text.len() / 5, "{painted} bytes painted");
        windows.push((window, index.total_height()));
    }
    assert!(windows[0].0.0 < windows[1].0.0 && windows[1].0.0 < windows[2].0.0, "{windows:?}");
    // The field is as tall as the note, which is what the parent scrolls over.
    assert!(windows[0].1 > 30_000.0, "{windows:?}");
}

/// A field inside a selection area, as the app's pages put them, holding `text`.
fn field_in_an_area(text: &str, threshold: usize) -> (RawTextField, super::SelectionParticipant) {
    use aimer_text_selection::selectable::{SelectionCoordinator, SelectionScope};
    use aimer_text_selection::session::SelectionSession;
    use aimer_widget::FocusNode;

    use super::test_support::field_config;
    use crate::TextEditingController;
    use crate::input_field::caret::{CaretBlink, CaretContext};

    let context = dummy_build_context(400.0, 200.0);
    let session = SelectionSession::new(
        context.window().clone(),
        Rc::new(SelectionCoordinator::default()),
        aimer_widget::base::Color::Rgba(51, 153, 255, 96),
    );
    let participant = context
        .with_state(SelectionScope(session), |inner| super::SelectionParticipant::from_context(inner, Rc::from(text)))
        .expect("the context is inside a selection area");
    let caret = CaretBlink::new();
    let mut config = field_config(TextEditingController::with_text(text));
    config.min_lines = Some(3);
    config.max_lines = None;
    let mut field = RawTextField::with_caret_context(
        config,
        caret.clone(),
        CaretContext::new(caret),
        FocusNode::new(),
        Some(participant.clone()),
        None,
    );
    field.window_threshold = threshold;
    (field, participant)
}

#[test]
fn a_windowed_field_leaves_the_selection_area_alone() {
    let text = note(40);
    // The same note and the same area, windowed and not: only the one that is not
    // follows what the area selects, and only it pays for a copy of its text to do so.
    let (whole, whole_participant) = field_in_an_area(&text, usize::MAX);
    whole.cursor.set_offset(0);
    whole_participant.set_range(8, 2);
    whole.sync_cursor_from_area();
    assert_eq!(whole.cursor.offset(), 2, "a field that is laid out whole follows its area");

    let (windowed, participant) = field_in_an_area(&text, 0);
    windowed.cursor.set_offset(0);
    participant.set_range(8, 2);
    windowed.sync_cursor_from_area();
    assert_eq!(windowed.cursor.offset(), 0, "a windowed field has no layout of its text for an area to select");
}

#[test]
fn a_drag_selects_in_a_windowed_field_inside_a_selection_area_as_it_does_in_one_laid_out_whole() {
    use aimer_attribute::Vec2d;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let text = note(400);
    let (whole, _) = field_in_an_area(&text, usize::MAX);
    let (windowed, _) = field_in_an_area(&text, 0);
    let ctx = dummy_build_context(400.0, 200.0);
    let mut selections = Vec::new();
    for field in [&whole, &windowed] {
        field.cursor.set_offset(0);
        frame(field, &ctx);
        let from = Vec2d { x: 30.0, y: 20.0 };
        let to = Vec2d { x: 220.0, y: 60.0 };
        let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(from, PointerButton::Primary)));
        field.update(&ctx);
        let _ = field.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(to, PointerButton::Primary)));
        field.update(&ctx);
        let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(to, PointerButton::Primary)));
        field.update(&ctx);
        selections.push((field.cursor.selection_anchor(), field.cursor.offset()));
    }
    let (anchor, focus) = selections[1];
    assert!(anchor.is_some() && anchor != Some(focus), "the windowed field selected nothing: {selections:?}");
    assert_eq!(selections[0], selections[1], "a drag selects the same text either way");
}

// ── Shift and a click ───────────────────────────────────────────────────────────

/// Presses and releases the primary button at `at`, one frame each, as the event loop does.
fn click_at(field: &RawTextField, ctx: &BuildContext<'_>, at: (f32, f32)) {
    use aimer_attribute::Vec2d;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let at = Vec2d { x: at.0, y: at.1 };
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(at, PointerButton::Primary)));
    field.update(ctx);
    let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(at, PointerButton::Primary)));
    field.update(ctx);
}

fn hold_shift(held: bool) {
    use aimer_events::element::Modifiers;
    Modifiers::set_current(Modifiers { shift: held, ..Modifiers::default() });
}

/// The selection as (anchor, focus), the anchor being the caret when nothing is selected.
fn selection_of(field: &RawTextField) -> (usize, usize) {
    (field.cursor.selection_anchor().unwrap_or(field.cursor.offset()), field.cursor.offset())
}

#[test]
fn a_shift_click_selects_from_the_caret_to_where_it_lands_in_a_field_laid_out_whole_or_windowed() {
    let text = note(400);
    for (what, threshold) in [("laid out whole", usize::MAX), ("windowed", 0)] {
        let field = field(&text, threshold);
        let ctx = dummy_build_context(800.0, 600.0);
        field.cursor.set_offset(0);
        frame(&field, &ctx);

        // A plain click puts the caret, with nothing selected.
        hold_shift(false);
        click_at(&field, &ctx, (60.0, 20.0));
        let caret = field.cursor.offset();
        assert!(field.cursor.selection_range().is_none(), "{what}: a click selects nothing");

        // Shift and a click somewhere else selects from the caret to there.
        hold_shift(true);
        click_at(&field, &ctx, (220.0, 90.0));
        hold_shift(false);
        let (anchor, focus) = selection_of(&field);
        assert_eq!(anchor, caret, "{what}: the selection starts where the caret was");
        assert!(focus > anchor, "{what}: and runs to the click: {anchor}..{focus}");

        // The same selection a drag from the first point to the second makes.
        let dragged = self::field(&text, threshold);
        dragged.cursor.set_offset(0);
        frame(&dragged, &ctx);
        {
            use aimer_attribute::Vec2d;
            use aimer_events::pointer::{PointerButton, PointerInfo};
            let (from, to) = (Vec2d { x: 60.0, y: 20.0 }, Vec2d { x: 220.0, y: 90.0 });
            let _ = dragged.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(from, PointerButton::Primary)));
            dragged.update(&ctx);
            let _ = dragged.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(to, PointerButton::Primary)));
            dragged.update(&ctx);
            let _ = dragged.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(to, PointerButton::Primary)));
            dragged.update(&ctx);
        }
        assert_eq!(selection_of(&field), selection_of(&dragged), "{what}: as a drag does");
    }
}

#[test]
fn a_shift_click_keeps_the_anchor_and_moves_the_end_of_a_selection_either_way() {
    let text = note(400);
    for (what, threshold) in [("laid out whole", usize::MAX), ("windowed", 0)] {
        let field = field(&text, threshold);
        let ctx = dummy_build_context(800.0, 600.0);
        field.cursor.set_offset(0);
        frame(&field, &ctx);
        // A selection made from the keyboard: 40 to 60.
        field.cursor.set_selection_anchor(Some(40));
        field.cursor.set_offset(60);

        hold_shift(true);
        click_at(&field, &ctx, (250.0, 110.0));
        let (anchor, focus) = selection_of(&field);
        assert_eq!(anchor, 40, "{what}: the anchor stays");
        assert!(focus > 60, "{what}: the end moves forward to the click ({focus})");

        // Back before the anchor: the selection turns around it.
        click_at(&field, &ctx, (6.0, 6.0));
        hold_shift(false);
        let (anchor, focus) = selection_of(&field);
        assert_eq!(anchor, 40, "{what}: still the anchor");
        assert!(focus < 40, "{what}: the end is before it now ({focus})");
    }
}

#[test]
fn a_click_without_shift_still_clears_the_selection_and_a_shift_click_is_not_a_double_click() {
    let text = note(400);
    let field = field(&text, 0);
    let ctx = dummy_build_context(800.0, 600.0);
    field.cursor.set_offset(0);
    frame(&field, &ctx);
    field.cursor.set_selection_anchor(Some(10));
    field.cursor.set_offset(30);

    hold_shift(false);
    click_at(&field, &ctx, (120.0, 60.0));
    assert!(field.cursor.selection_range().is_none(), "a plain click clears the selection");

    // Two shift clicks in the same place, quickly, are not a double click that selects a word.
    hold_shift(true);
    click_at(&field, &ctx, (200.0, 100.0));
    let first = selection_of(&field);
    click_at(&field, &ctx, (200.0, 100.0));
    hold_shift(false);
    assert_eq!(selection_of(&field), first, "the second shift click extends to the same place: it selects no word");
}

#[test]
fn shift_does_not_change_what_a_finger_does() {
    use aimer_attribute::Vec2d;
    use aimer_events::pointer::PointerInfo;

    let text = note(400);
    let field = field(&text, 0);
    let ctx = dummy_build_context(800.0, 600.0);
    field.cursor.set_offset(0);
    frame(&field, &ctx);
    field.cursor.set_selection_anchor(Some(10));
    field.cursor.set_offset(30);

    hold_shift(true);
    let at = Vec2d { x: 120.0, y: 60.0 };
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::touch(at, 1)));
    field.update(&ctx);
    let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::touch(at, 1)));
    field.update(&ctx);
    hold_shift(false);
    assert!(field.cursor.selection_range().is_none(), "a touch has no shift: it places the caret");
}

#[test]
fn a_shift_click_extends_the_selection_in_a_field_inside_a_selection_area() {
    let text = note(40);
    let (field, _) = field_in_an_area(&text, usize::MAX);
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(0);
    frame(&field, &ctx);

    hold_shift(false);
    click_at(&field, &ctx, (40.0, 20.0));
    let caret = field.cursor.offset();
    hold_shift(true);
    click_at(&field, &ctx, (200.0, 60.0));
    hold_shift(false);
    let (anchor, focus) = selection_of(&field);
    assert_eq!(anchor, caret);
    assert!(focus > anchor, "{anchor}..{focus}");
    // And it stays so once the area has been asked what it holds.
    field.sync_cursor_from_area();
    assert_eq!(selection_of(&field), (anchor, focus));
}

// ── Styled text and embeds ─────────────────────────────────────────────────────

use std::cell::{Cell, RefCell};
use std::ops::Range;

use aimer_text::{SpanStyle, StyledRange};
use aimer_widget::FocusNode;

use super::embed_tests::Probe;
use super::test_support::field_config;
use crate::TextEditingController;
use crate::input_field::caret::CaretBlink;
use crate::input_field::embed::{TextEmbed, TextEmbedder};
use crate::input_field::highlight::TextHighlighter;

/// A note with headings, which a highlighter makes taller than the text under them, so
/// that the lines of the note are not all one height.
fn markdown(sections: usize) -> String {
    (0..sections)
        .map(|index| {
            format!(
                "## Heading {index}\nSome **bold** words in a paragraph that goes on for long enough \
                 to wrap across the width of the pane, in section {index}.\n- item one\n- item two\n\n"
            )
        })
        .collect()
}

fn markdown_highlighter() -> TextHighlighter {
    TextHighlighter::by_line((), |line: &str, _: &()| {
        let mut ranges = Vec::new();
        if line.starts_with("## ") {
            ranges.push(StyledRange::new(0..line.trim_end().len(), SpanStyle::new().font_size(28)));
        }
        let mut from = 0;
        while let Some(open) = line[from..].find("**") {
            let open = from + open;
            let Some(close) = line[open + 2..].find("**") else { break };
            let end = open + 2 + close + 2;
            ranges.push(StyledRange::new(open..end, SpanStyle::new().color(aimer_widget::base::Color::Rgba(200, 30, 30, 255))));
            from = end;
        }
        (ranges, ())
    })
}

/// A field that scrolls inside ten lines, laying a window out from `threshold` bytes of text,
/// that highlights and embeds as it is asked to.
fn rich_field(
    text: &str,
    threshold: usize,
    highlighter: Option<TextHighlighter>,
    embedder: Option<TextEmbedder>,
) -> RawTextField {
    let mut config = field_config(TextEditingController::with_text(text));
    config.min_lines = Some(3);
    config.max_lines = Some(10);
    config.highlighter = highlighter;
    config.embedder = embedder;
    let mut field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = threshold;
    field.cursor.set_offset(0);
    field
}

#[test]
fn a_highlighted_note_is_laid_out_and_painted_as_a_window_too() {
    let text = markdown(1200);
    assert!(text.len() > 150_000);
    let field = rich_field(&text, 64 * 1024, Some(markdown_highlighter()), None);
    let ctx = dummy_build_context(800.0, 600.0);

    let runs = frame(&field, &ctx);

    assert!(!runs.is_empty());
    let painted = runs.iter().map(|run| run.0.len()).sum::<usize>();
    assert!(painted < text.len() / 5, "{painted} bytes painted of {}", text.len());
    assert!(runs.iter().any(|run| run.0.contains("Heading 0")), "the top of the note");
    let window = field.window_lines.get().expect("a window");
    assert!(window.1 < index_of(&field).len() / 5, "{window:?}");
}

#[test]
fn a_highlighted_window_is_the_wholes_once_the_lines_are_laid_out() {
    use aimer_attribute::Vec2d;
    use aimer_events::pointer::{PointerButton, PointerInfo};

    let text = markdown(100);
    let whole = rich_field(&text, usize::MAX, Some(markdown_highlighter()), None);
    let windowed = rich_field(&text, 0, Some(markdown_highlighter()), None);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&whole, &ctx);
    lay_everything_out(&windowed, &ctx);

    // The headings are taller than the text, and the total is what the whole layout says.
    assert!(
        (windowed.scroll_y_extent.get() - whole.scroll_y_extent.get()).abs() < 1.5,
        "{} against {}",
        windowed.scroll_y_extent.get(),
        whole.scroll_y_extent.get()
    );

    for scroll in [0.0_f32, 900.0, 2500.0, whole.scroll_y_extent.get()] {
        for field in [&whole, &windowed] {
            field.reveal_caret.set(false);
            field.scroll_y.set(scroll);
            frame(field, &ctx);
        }
        for (x, y) in [(4.0, 6.0), (150.0, 40.0), (400.0, 100.0), (60.0, 170.0)] {
            let mut cursors = Vec::new();
            for field in [&whole, &windowed] {
                let down = Vec2d { x, y };
                let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(down, PointerButton::Primary)));
                field.update(&ctx);
                let _ = field.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(down, PointerButton::Primary)));
                field.update(&ctx);
                cursors.push(field.cursor.offset());
            }
            assert_eq!(cursors[0], cursors[1], "a click at ({x}, {y}) with the note scrolled to {scroll}");
        }
    }
}

/// Blocks of `code` between fences, one embed each, with a few lines of text around them.
fn embedded_note(blocks: usize) -> String {
    (0..blocks)
        .map(|index| format!("intro {index} of the note, a line of some length\n```\ncode {index}\n```\nafter {index}\n"))
        .collect()
}

fn fence_embedder(height: f32, updated_at: Rc<Cell<(f32, f32)>>) -> TextEmbedder {
    TextEmbedder::new(move |text, _context| {
        let mut embeds = Vec::new();
        let mut open: Option<usize> = None;
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            if line.starts_with("```") {
                match open.take() {
                    None => open = Some(at),
                    Some(start) => {
                        let probe = Probe::new(height, Rc::clone(&updated_at));
                        embeds.push(TextEmbed::new(start..at + line.trim_end().len(), embeds.len() as u64, move || probe.clone()));
                    }
                }
            }
            at += line.len();
        }
        embeds
    })
}

#[test]
fn embeds_are_laid_out_and_published_only_for_the_window() {
    let text = embedded_note(1500);
    let field = rich_field(&text, 64 * 1024, None, Some(fence_embedder(60.0, Rc::default())));
    let ctx = dummy_build_context(800.0, 600.0);

    frame(&field, &ctx);
    field.embeds.sync(1.0);
    let first = field.embeds.len();
    assert!(first > 0, "the embeds at the top are placed");
    assert!(first < 100, "{first} of 1500 embeds were measured and placed");

    // Scrolled to the middle, the embeds there are the ones placed.
    let index = index_of(&field);
    field.reveal_caret.set(false);
    field.scroll_y.set((index.total_height() / 2.0) as f32);
    frame(&field, &ctx);
    // The host drops what the field no longer shows when it places the rest.
    field.embeds.sync(1.0);
    let window = field.window_lines.get().unwrap();
    let bytes = index.line_bytes(window.0).start..index.line_bytes(window.1 - 1).end;
    let placed = (0..1500u64).filter(|&block| field.embeds.placement_of(block).is_some()).collect::<Vec<_>>();
    let in_window = (0..1500u64)
        .filter(|&block| {
            let start = text.find(&format!("code {block}\n")).unwrap();
            bytes.contains(&start)
        })
        .collect::<Vec<_>>();
    assert!(!in_window.is_empty());
    assert_eq!(placed.first(), in_window.first(), "the placed embeds are those of the window");
    assert!(placed.len() >= in_window.len() && placed.len() <= in_window.len() + 2, "{placed:?} against {in_window:?}");
    assert!(field.embeds.placement_of(0).is_none(), "the embeds at the top are no longer placed");
}

#[test]
fn a_windowed_note_with_embeds_is_as_tall_as_the_whole_one() {
    let text = embedded_note(80);
    let embedder = || fence_embedder(60.0, Rc::default());
    let whole = rich_field(&text, usize::MAX, None, Some(embedder()));
    let windowed = rich_field(&text, 0, None, Some(embedder()));
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&whole, &ctx);
    lay_everything_out(&windowed, &ctx);

    // Source lines inside an embed have no height of their own: the box is the one line.
    assert!(
        (windowed.scroll_y_extent.get() - whole.scroll_y_extent.get()).abs() < 1.5,
        "{} against {}",
        windowed.scroll_y_extent.get(),
        whole.scroll_y_extent.get()
    );
}

// ── Highlighters and embedders made for a window ────────────────────────────────

/// [`markdown_highlighter`] as one that reads a window: the lines from the one that holds the
/// start of the range, to the one that holds its end. `asked` collects the ranges it is given.
fn windowed_markdown_highlighter(asked: Rc<RefCell<Vec<Range<usize>>>>) -> TextHighlighter {
    let lines = markdown_highlighter();
    TextHighlighter::windowed(move |text, range, _context| {
        asked.borrow_mut().push(range.clone());
        let start = text.line_range(text.byte_to_line(range.start)).start;
        let end = text.line_range(text.byte_to_line(range.end.saturating_sub(1))).end;
        let slice = text.slice(start..end);
        lines
            .highlight(&slice)
            .into_iter()
            .map(|styled| StyledRange::new(styled.range.start + start..styled.range.end + start, styled.style))
            .collect()
    })
}

#[test]
fn a_windowed_highlighter_is_asked_for_the_lines_that_are_laid_out_and_no_others() {
    let text = markdown(1200);
    assert!(text.len() > 150_000);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let field = rich_field(&text, 64 * 1024, Some(windowed_markdown_highlighter(Rc::clone(&asked))), None);
    let by_line = rich_field(&text, 64 * 1024, Some(markdown_highlighter()), None);
    let ctx = dummy_build_context(800.0, 600.0);

    let runs = frame(&field, &ctx);

    let asked = asked.borrow().clone();
    assert!(!asked.is_empty(), "it was asked");
    for range in &asked {
        assert!(range.len() < text.len() / 5, "{range:?} of {}: a window, not the note", text.len());
    }
    // The same note styled a line at a time looks the same: the headings are as tall.
    assert_eq!(runs, frame(&by_line, &ctx));
    assert!(runs.iter().any(|run| run.0.contains("Heading 0")));
}

#[test]
fn a_window_that_is_the_same_after_its_layout_taught_the_heights_is_laid_out_once() {
    // The headings are taller than the lines were estimated, so laying the window out changes what
    // the index knows. The lines wanted afterwards are the ones just laid out, and laying them out
    // again would only repeat that.
    let text = markdown(1200);
    let field = rich_field(&text, 64 * 1024, Some(markdown_highlighter()), None);
    let ctx = dummy_build_context(800.0, 600.0);
    let before = super::rich_window_layouts();

    frame(&field, &ctx);

    assert_eq!(super::rich_window_layouts() - before, 1, "the window was laid out again for the same lines");
}

#[test]
fn a_windowed_highlighter_is_not_asked_again_for_a_frame_that_changes_nothing() {
    let text = markdown(1200);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let field = rich_field(&text, 64 * 1024, Some(windowed_markdown_highlighter(Rc::clone(&asked))), None);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&field, &ctx);
    let first = asked.borrow().len();
    for _ in 0..3 {
        frame(&field, &ctx);
    }
    assert_eq!(asked.borrow().len(), first);
}

#[test]
fn a_windowed_highlighter_follows_an_edit_in_the_window() {
    let text = markdown(1200);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let field = rich_field(&text, 64 * 1024, Some(windowed_markdown_highlighter(Rc::clone(&asked))), None);
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&field, &ctx);
    let before = asked.borrow().len();

    field.cursor.set_offset(0);
    let _ = field.on_event(&commit("## "));
    let runs = frame(&field, &ctx);

    assert!(asked.borrow().len() > before, "the edited text is styled again");
    assert!(asked.borrow().iter().all(|range| range.len() < text.len() / 5));
    assert!(runs.iter().any(|run| run.0.contains("## ") || run.0.contains("Heading")));
}

/// [`fence_embedder`] as one that reads a window: blocks start at a line that begins "intro", so
/// the range is read from the last of them before it, and a block is a whole fence. The key of a
/// block is its number, wherever the range starts.
fn windowed_fence_embedder(height: f32, asked: Rc<RefCell<Vec<Range<usize>>>>) -> TextEmbedder {
    TextEmbedder::windowed(move |text, range, _context| {
        asked.borrow_mut().push(range.clone());
        let from = text.slice(range.start.saturating_sub(300)..range.start);
        let start = range.start.saturating_sub(300) + from.rfind("intro").unwrap_or(0);
        let start = if range.start < 300 { 0 } else { start };
        let end = (range.end + 300).min(text.len());
        let end = text.slice(range.end..end).find("after").map_or(end, |after| range.end + after);
        let slice = text.slice(start..end);
        let mut embeds = Vec::new();
        let mut open: Option<usize> = None;
        let mut at = start;
        for line in slice.split_inclusive('\n') {
            if line.starts_with("```") {
                match open.take() {
                    None => open = Some(at),
                    Some(first) => {
                        let number = text.slice(first..at).trim_start_matches("```\ncode ").trim().parse::<u64>().unwrap_or(0);
                        let probe = Probe::new(height, Rc::default());
                        embeds.push(TextEmbed::new(first..at + line.trim_end().len(), number, move || probe.clone()));
                    }
                }
            }
            at += line.len();
        }
        embeds
    })
}

#[test]
fn a_windowed_embedder_is_asked_for_the_lines_that_are_laid_out_and_places_only_those() {
    let text = embedded_note(1500);
    let asked = Rc::new(RefCell::new(Vec::new()));
    let field = rich_field(&text, 64 * 1024, None, Some(windowed_fence_embedder(60.0, Rc::clone(&asked))));
    let ctx = dummy_build_context(800.0, 600.0);

    frame(&field, &ctx);
    field.embeds.sync(1.0);

    let first = field.embeds.len();
    assert!(first > 0, "the embeds at the top are placed");
    assert!(first < 100, "{first} of 1500 embeds were placed");
    assert!(!asked.borrow().is_empty());
    for range in asked.borrow().iter() {
        assert!(range.len() < text.len() / 5, "{range:?}: a window, not the note");
    }
    assert!(field.embeds.placement_of(0).is_some(), "the first block is at the top");

    // Scrolled to the middle, the embeds there are the ones placed, and by the same keys.
    let index = index_of(&field);
    field.reveal_caret.set(false);
    field.scroll_y.set((index.total_height() / 2.0) as f32);
    frame(&field, &ctx);
    field.embeds.sync(1.0);
    let placed = (0..1500u64).filter(|&block| field.embeds.placement_of(block).is_some()).collect::<Vec<_>>();
    assert!(!placed.is_empty() && placed.len() < 100, "{placed:?}");
    assert!(placed.iter().all(|&block| block > 100), "{placed:?}: the middle of the note");
    assert!(field.embeds.placement_of(0).is_none());
}

#[test]
fn a_windowed_note_with_embeds_is_as_tall_as_the_whole_one_with_windowed_readers() {
    let text = embedded_note(80);
    let whole = rich_field(&text, usize::MAX, None, Some(fence_embedder(60.0, Rc::default())));
    let windowed = rich_field(&text, 0, None, Some(windowed_fence_embedder(60.0, Rc::default())));
    let ctx = dummy_build_context(800.0, 600.0);
    frame(&whole, &ctx);
    lay_everything_out(&windowed, &ctx);
    assert!(
        (windowed.scroll_y_extent.get() - whole.scroll_y_extent.get()).abs() < 1.5,
        "{} against {}",
        windowed.scroll_y_extent.get(),
        whole.scroll_y_extent.get()
    );
}

// ── The caret of a field scrolled away from it ─────────────────────────────────

#[test]
fn a_caret_scrolled_out_of_its_field_is_not_drawn_outside_it() {
    let text = note(400);
    for (what, threshold) in [("laid out whole", usize::MAX), ("windowed", 0)] {
        let field = field(&text, threshold);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(800.0, 600.0);
        frame(&field, &ctx);
        assert!(field.caret_context.is_available(), "{what}: the caret is in view at the top");

        // Scrolled away from the caret, as a wheel does: the caret stays where it is in
        // the text, which is no longer where the field shows.
        field.reveal_caret.set(false);
        field.scroll_y.set(2000.0);
        frame(&field, &ctx);
        assert!(
            !field.caret_context.is_available(),
            "{what}: the caret is above the viewport and must not be drawn below the field"
        );

        // And back.
        field.scroll_y.set(0.0);
        frame(&field, &ctx);
        assert!(field.caret_context.is_available(), "{what}: the caret returns with its line");

        // A caret below the viewport too.
        field.cursor.set_offset(field.controller.grapheme_count());
        field.reveal_caret.set(false);
        field.scroll_y.set(0.0);
        frame(&field, &ctx);
        assert!(!field.caret_context.is_available(), "{what}: the caret is below the viewport");
    }
}

#[test]
fn a_caret_is_told_where_the_visible_part_of_its_field_ends_so_that_what_hangs_from_it_can_stay_on_screen() {
    let text = note(400);
    for (what, threshold) in [("laid out whole", usize::MAX), ("windowed", 0)] {
        let field = field(&text, threshold);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(800.0, 600.0);
        frame(&field, &ctx);
        let (right, bottom) = field
            .caret_context
            .visible_extent()
            .unwrap_or_else(|| panic!("{what}: a field that has been laid out says where it ends"));
        let caret = field.caret_context.geometry();
        assert!(caret.x >= 0.0 && caret.x + caret.width <= right + 0.5, "{what}: the caret is inside {right} across: {caret:?}");
        assert!(caret.y >= 0.0 && caret.y + caret.height <= bottom + 0.5, "{what}: the caret is inside {bottom} down: {caret:?}");
        // The extent is the visible part: four hundred lines are far taller than the 600 the window gives.
        assert!(bottom <= 600.5, "{what}: the visible part ends at {bottom}, not at the end of the text");
        // A caret that has scrolled out of view does not change where the visible part ends.
        field.reveal_caret.set(false);
        field.scroll_y.set(2000.0);
        frame(&field, &ctx);
        assert_eq!(field.caret_context.visible_extent(), Some((right, bottom)), "{what}");
    }
}

#[test]
fn the_wheel_scrolls_a_field_whether_or_not_it_is_windowed() {
    use aimer_attribute::Vec2d;
    use aimer_events::element::{ScrollDeltaKind, TouchPhase};

    let text = note(400);
    for (what, threshold) in [("laid out whole", usize::MAX), ("windowed", 0)] {
        let field = field(&text, threshold);
        field.cursor.set_offset(0);
        let ctx = dummy_build_context(800.0, 600.0);
        frame(&field, &ctx);
        assert!(field.scroll_y_extent.get() > 1000.0, "{what}");
        let before = field.scroll_y.get();
        let _ = field.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -200.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        });
        frame(&field, &ctx);
        assert!(field.scroll_y.get() > before + 100.0, "{what}: scrolled from {before} to {}", field.scroll_y.get());
    }
}

#[test]
fn a_composition_in_a_long_note_keeps_the_note_windowed() {
    let text = note(3000);
    let controller = crate::TextEditingController::with_text(&text);
    let mut config = super::test_support::field_config(controller.clone());
    config.min_lines = Some(3);
    config.max_lines = Some(10);
    let mut field = RawTextField::new(config, crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = 64 * 1024;
    let ctx = dummy_build_context(800.0, 600.0);
    for _ in 0..3 {
        frame(&field, &ctx);
    }
    let middle = controller.grapheme_count() / 2;
    field.cursor.set_offset(middle);
    field.cursor.set_selection_anchor(None);
    field.reveal_caret.set(true);
    frame(&field, &ctx);

    // The input method starts a composition at the caret.
    assert!(field.set_preedit("XY", None));
    assert!(field.is_composing());
    let runs = frame(&field, &ctx);

    let window = field.window_lines.get().expect("still windowed");
    assert!(window.1 - window.0 < index_of(&field).len() / 4, "{window:?}");
    let painted = runs.iter().map(|run| run.0.len()).sum::<usize>();
    assert!(painted < text.len() / 5, "{painted} bytes painted: the window, not the note");
    // What the field lays out is the text without the composition, which the input method owns.
    assert_eq!(index_of(&field).text().len(), text.len(), "the text of the note, and not the composition in it");
    assert!(controller.text().len() > text.len(), "the controller holds the composition");
}

// ── Enter in a field that scrolls inside a number of visible lines ─────────────

#[test]
fn enter_breaks_the_line_however_many_lines_the_field_shows() {
    // `max_lines` is how many lines are visible; past that the field scrolls. It is not a
    // limit on the lines of the text, so Enter works at the last visible line and beyond.
    for (what, lines, threshold) in [("a short note", 3usize, usize::MAX), ("past max_lines", 40, usize::MAX), ("windowed", 3000, 64 * 1024)] {
        let text = note(lines);
        let field = field(&text, threshold);
        let ctx = dummy_build_context(800.0, 600.0);
        frame(&field, &ctx);
        field.cursor.set_offset(field.controller.grapheme_count());
        let before = field.controller.text().matches('\n').count();

        let _ = field.on_event(&key(NamedKey::Enter));
        let _ = field.on_event(&commit("x"));
        frame(&field, &ctx);

        let after = field.controller.text();
        assert_eq!(after.matches('\n').count(), before + 1, "{what}: Enter inserted a line break");
        assert!(after.ends_with("\nx"), "{what}: and the next text is on the new line");
    }
}

/// A highlighted field that grows with its text, in a parent that shows all of it, at `scale`.
fn growing_rich_field(text: &str, scale: f32) -> (RawTextField, BuildContext<'static>) {
    let mut config = field_config(TextEditingController::with_text(text));
    config.min_lines = Some(3);
    config.max_lines = None;
    config.highlighter = Some(markdown_highlighter());
    let mut field = RawTextField::new(config, CaretBlink::new(), FocusNode::new());
    let _ = field.on_event(&ElementEvent::FocusGained);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = 64 * 1024;
    field.cursor.set_offset(0);
    field.reveal_caret.set(false);

    let mut ctx = dummy_build_context(800.0, 600.0);
    ctx.scale = scale;
    ctx.box_constraint.max_height = 10_000_000.0;
    ctx.parent_size.height = 10_000_000.0;
    ctx.set_visible_rect(Some((0.0, 0.0, 800.0 * scale, 600.0 * scale)));
    (field, ctx)
}

/// The text widget refuses a box taller than a million physical pixels, so a field that gave it
/// the height of a long note, as a screen with a scale of 2 reaches at half the lines, drew no
/// text at all. A windowed text is built in the box of its window.
#[test]
fn a_note_over_a_million_physical_pixels_tall_still_paints() {
    let text = markdown(6000);
    let (field, ctx) = growing_rich_field(&text, 2.0);

    let runs = frame(&field, &ctx);

    let total = index_of(&field).total_height();
    assert!(total > 1_000_000.0, "the note must be taller than the limit: {total}");
    assert!(runs.iter().any(|run| run.0.contains("Heading 0")), "the top of the note: {} runs", runs.len());
    let painted = runs.iter().map(|run| run.0.len()).sum::<usize>();
    assert!(painted < text.len() / 20, "{painted} bytes painted of {}", text.len());
}

/// A field is held to a height far above the million pixels most boxes are, since the note a
/// page scrolls over is as tall as its text and only a window of it is ever drawn.
#[test]
fn a_note_over_a_million_logical_pixels_tall_still_paints() {
    let text = markdown(12000);
    let (field, ctx) = growing_rich_field(&text, 1.0);

    let runs = frame(&field, &ctx);

    let total = index_of(&field).total_height();
    assert!(total > 1_000_000.0, "the note must be taller than a million pixels: {total}");
    assert!(runs.iter().any(|run| run.0.contains("Heading 0")), "the top of the note: {} runs", runs.len());
}

/// How long each frame of scrolling a long highlighted note takes. A timing, not a check:
///
/// ```text
/// cargo test --release -p aimer_input bench_scroll -- --ignored --nocapture --test-threads=1
/// ```
#[test]
#[ignore = "a timing, not a check"]
fn bench_scroll_frames() {
    use std::time::Instant;
    for sections in [1500, 4000, 12000] {
        let text = markdown(sections);
        let (field, mut ctx) = growing_rich_field(&text, 2.0);
        frame(&field, &ctx);
        let mut times = Vec::new();
        for step in 1..=400 {
            let top = step as f32 * 80.0; // physical: 40 logical pixels a frame
            ctx.set_visible_rect(Some((0.0, top, 1600.0, 1200.0)));
            let started = Instant::now();
            let runs = frame(&field, &ctx);
            times.push((started.elapsed(), step));
            if step == 400 {
                println!("  last frame paints {} runs, from {:?}", runs.len(), runs.first().map(|run| run.0.clone()));
            }
        }
        let mut sorted: Vec<_> = times.iter().map(|t| t.0).collect();
        sorted.sort();
        let pick = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
        let mean = sorted.iter().sum::<std::time::Duration>() / sorted.len() as u32;
        let mut worst = times.clone();
        worst.sort_by(|a, b| b.0.cmp(&a.0));
        println!(
            "{:>7} bytes: mean {mean:>9.2?}  p50 {:>9.2?}  p95 {:>9.2?}  max {:>9.2?}   slowest frames: {:?}",
            text.len(),
            pick(0.5),
            pick(0.95),
            pick(1.0),
            worst.iter().take(6).map(|t| (t.1, format!("{:.1?}", t.0))).collect::<Vec<_>>(),
        );
    }
}

/// A pass that does not know what is shown (layout runs before the parent has placed the field)
/// must not lay out the top of the note and put that in the cache in place of the window that is
/// on screen: the next pass, which knows, would have to lay its window out again, and so would
/// every frame after it, twice a frame.
#[test]
fn a_pass_that_does_not_know_what_is_shown_keeps_the_window_that_is_on_screen() {
    let text = markdown(2500);
    let (field, mut ctx) = growing_rich_field(&text, 1.0);
    frame(&field, &ctx);
    let total = index_of(&field).total_height() as f32;
    ctx.set_visible_rect(Some((0.0, total * 0.6, 800.0, 600.0)));
    frame(&field, &ctx);
    let window = field.window_lines.get().expect("a window is laid out");
    let geometry = field.geometry_cache.latest().expect("its geometry is kept");
    assert!(window.0 > 100, "the window is far down the note: {window:?}");

    ctx.set_visible_rect(None);
    frame(&field, &ctx);

    assert_eq!(field.window_lines.get(), Some(window), "the window stays where the viewport was");
    assert!(
        Rc::ptr_eq(&geometry, &field.geometry_cache.latest().unwrap()),
        "and nothing was laid out again in its place"
    );
}

/// Scrolling a long note with embedded blocks (a table, a code block) in a field that grows
/// with its text, as the notes app does: every frame timed, since a spike is what drops a frame.
///
/// ```text
/// cargo nextest run -p aimer_input --run-ignored only -E 'test(bench_scroll_frames_with_embeds)' --no-capture
/// ```
#[test]
#[ignore = "a timing, not a check"]
fn bench_scroll_frames_with_embeds() {
    use std::time::Instant;
    for blocks in [1200, 3500] {
        let text = embedded_note(blocks);
        let (mut field, mut ctx) = growing_rich_field(&text, 2.0);
        field.embedder = Some(fence_embedder(120.0, Rc::default()));
        frame(&field, &ctx);
        let mut times = Vec::new();
        for step in 1..=400 {
            let top = step as f32 * 80.0; // physical: 40 logical pixels a frame
            ctx.set_visible_rect(Some((0.0, top, 1600.0, 1200.0)));
            let started = Instant::now();
            frame(&field, &ctx);
            times.push((started.elapsed(), step));
        }
        let mut sorted: Vec<_> = times.iter().map(|t| t.0).collect();
        sorted.sort();
        let pick = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
        let mean = sorted.iter().sum::<std::time::Duration>() / sorted.len() as u32;
        let slow = sorted.iter().filter(|time| **time > std::time::Duration::from_millis(16)).count();
        println!(
            "{:>7} bytes with embeds: mean {mean:>9.2?}  p50 {:>9.2?}  p95 {:>9.2?}  max {:>9.2?}  over 16 ms: {slow}/400",
            text.len(),
            pick(0.5),
            pick(0.95),
            pick(1.0),
        );
    }
}
