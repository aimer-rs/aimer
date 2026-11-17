//! A field with a highlighter: the text is styled by range, and everything the
//! user touches — caret, selection, hit testing — keeps working because the
//! layout they read is the layout of the styled text.

use std::cell::Cell;
use std::rc::Rc;

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_style::TextOverflow;
use aimer_text::{SpanStyle, StyledRange};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, LayoutElement};

use super::test_support::{dummy_build_context, focused_multiline_field};
use super::RawTextField;
use crate::input_field::controller::TextFieldController;
use crate::input_field::highlight::TextHighlighter;

const KEYWORD: [u8; 4] = [200, 30, 30, 255];

/// Colours every `let` red and counts how often it was asked to.
fn keywords() -> (TextHighlighter, Rc<Cell<u32>>) {
    let runs = Rc::new(Cell::new(0));
    let counted = Rc::clone(&runs);
    let highlighter = TextHighlighter::new(move |text| {
        counted.set(counted.get() + 1);
        text.match_indices("let")
            .map(|(start, word)| {
                StyledRange::new(
                    start..start + word.len(),
                    SpanStyle::new().color(Color::Rgba(KEYWORD[0], KEYWORD[1], KEYWORD[2], 255)),
                )
            })
            .collect()
    });
    (highlighter, runs)
}

fn field(text: &str, highlighter: Option<TextHighlighter>) -> RawTextField {
    let mut field = focused_multiline_field(TextFieldController::with_initial(text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.highlighter = highlighter;
    field
}

/// The `(text, rgba)` of every glyph run `field` paints.
fn painted_runs(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<(String, [u8; 4])> {
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
            DrawCommand::DrawText { text, color, .. } => {
                Some((text.to_string(), color.to_rgba8().0))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_highlighted_range_is_painted_in_its_colour() {
    let (highlighter, _) = keywords();
    let field = field("let x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);

    let runs = painted_runs(&field, &ctx);

    assert_eq!(runs.first().map(|run| run.0.as_str()), Some("let"), "{runs:?}");
    assert_eq!(runs[0].1, KEYWORD);
    assert!(
        runs[1..].iter().all(|run| run.1 != KEYWORD),
        "only the keyword is coloured: {runs:?}"
    );
}

#[test]
fn a_field_without_a_highlighter_paints_one_plain_run() {
    let field = field("let x = 1", None);
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);

    let runs = painted_runs(&field, &ctx);

    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].0, "let x = 1");
}

/// Colour must not move a glyph: the caret is placed from the same layout.
#[test]
fn highlighting_does_not_move_the_caret() {
    const TEXT: &str = "let a = 1;\nlet bb = 22;\nlet ccc = 333;";
    let (highlighter, _) = keywords();
    let plain = field(TEXT, None);
    let styled = field(TEXT, Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);

    for offset in 0..=TEXT.chars().count() {
        plain.cursor.set_offset(offset);
        styled.cursor.set_offset(offset);
        let plain_caret = plain.editable_geometry(&ctx, 380.0).caret_geometry(offset).unwrap();
        let styled_caret = styled.editable_geometry(&ctx, 380.0).caret_geometry(offset).unwrap();
        assert_eq!(plain_caret.line_index, styled_caret.line_index, "line at {offset}");
        assert!(
            (plain_caret.x - styled_caret.x).abs() < 0.5,
            "caret x at {offset}: {} vs {}",
            plain_caret.x,
            styled_caret.x
        );
        assert!((plain_caret.y - styled_caret.y).abs() < 0.5, "caret y at {offset}");
    }
}

#[test]
fn highlighting_does_not_change_hit_testing_or_line_breaks() {
    const TEXT: &str = "let alpha = beta;\nlet gamma = delta + epsilon;";
    let (highlighter, _) = keywords();
    let plain = field(TEXT, None);
    let styled = field(TEXT, Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    let plain_geometry = plain.editable_geometry(&ctx, 120.0);
    let styled_geometry = styled.editable_geometry(&ctx, 120.0);

    // Shaping the runs apart loses kerning across a range edge, a fraction of
    // a pixel; where the lines break must not move at all.
    assert_eq!(plain_geometry.visual_lines.len(), styled_geometry.visual_lines.len());
    for (plain_line, styled_line) in plain_geometry
        .visual_lines
        .iter()
        .zip(&styled_geometry.visual_lines)
    {
        assert_eq!(
            (plain_line.byte_start, plain_line.byte_end),
            (styled_line.byte_start, styled_line.byte_end)
        );
        assert!((plain_line.width - styled_line.width).abs() < 0.5);
    }
    for x in (0..120).step_by(7) {
        for y in (0..60).step_by(9) {
            assert_eq!(
                plain_geometry.hit_test(x as f32, y as f32),
                styled_geometry.hit_test(x as f32, y as f32),
                "hit test at ({x}, {y})"
            );
        }
    }
}

#[test]
fn moving_the_caret_does_not_run_the_highlighter_again() {
    let (highlighter, runs) = keywords();
    let field = field("let x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);

    field.update(&ctx);
    let after_first_frame = runs.get();
    for offset in 0..=5 {
        field.cursor.set_offset(offset);
        field.controller.set_selection_graphemes(offset, offset);
        field.update(&ctx);
    }

    assert!(after_first_frame >= 1);
    assert_eq!(runs.get(), after_first_frame);
}

#[test]
fn editing_the_text_restyles_it() {
    let (highlighter, _) = keywords();
    let field = field("x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);
    assert!(painted_runs(&field, &ctx).iter().all(|run| run.1 != KEYWORD));

    field.controller.set_text("let x = 1");
    field.update(&ctx);

    assert_eq!(painted_runs(&field, &ctx)[0], ("let".to_owned(), KEYWORD));
}

#[test]
fn a_secure_field_ignores_its_highlighter() {
    let (highlighter, runs) = keywords();
    let mut field = field("let secret", Some(highlighter));
    field.input_type = super::InputType::Password;
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);

    let painted = painted_runs(&field, &ctx);

    assert_eq!(runs.get(), 0, "bullets are not the text it was written for");
    assert!(painted.iter().all(|run| run.1 != KEYWORD), "{painted:?}");
}

#[test]
fn swapping_the_highlighter_repaints_without_a_text_change() {
    let (keywords, _) = keywords();
    let mut field = field("let x = 1", None);
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);
    painted_runs(&field, &ctx);
    assert!(!field.paint_needs_recording(&ctx), "painted and unchanged");

    field.highlighter = Some(keywords);

    assert!(field.paint_needs_recording(&ctx), "the new styling must be recorded");
    field.update(&ctx);
    assert_eq!(painted_runs(&field, &ctx)[0].1, KEYWORD);
}

#[test]
fn the_highlighter_sees_the_text_without_an_uncommitted_composition() {
    use crate::{TextEditingValue, TextRange, TextSelection};

    let seen = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let recorded = Rc::clone(&seen);
    let highlighter = TextHighlighter::new(move |text| {
        recorded.borrow_mut().push(text.to_owned());
        Vec::new()
    });
    let field = field("", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    field.controller.set_value(
        TextEditingValue::new(
            "ab\u{3042}cd",
            TextSelection::collapsed(5),
            Some(TextRange::new(2, 5)),
        ),
    );

    field.update(&ctx);

    assert_eq!(seen.borrow().last().map(String::as_str), Some("abcd"));
}

/// A frame asks for the display widget several times; a paragraph must be
/// prepared once, not once per ask.
#[test]
fn a_frame_shares_one_widget_for_highlighted_text() {
    let (highlighter, _) = keywords();
    let field = field("let x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    let geometry = field.editable_geometry(&ctx, 380.0);
    let style = field.field_text_style();
    let align = field.horizontal_text_align();

    let first = field.build_display_widget(&geometry, &style, align);
    let second = field.build_display_widget(&geometry, &style, align);

    assert!(Rc::ptr_eq(&first, &second));
}

#[test]
fn plain_text_still_gets_a_fresh_widget() {
    let field = field("let x = 1", None);
    let ctx = dummy_build_context(400.0, 200.0);
    let geometry = field.editable_geometry(&ctx, 380.0);
    let style = field.field_text_style();
    let align = field.horizontal_text_align();

    let first = field.build_display_widget(&geometry, &style, align);
    let second = field.build_display_widget(&geometry, &style, align);

    assert!(!Rc::ptr_eq(&first, &second));
    assert!(first.ranges.is_none());
}

#[test]
fn the_shared_widget_is_rebuilt_when_the_text_or_style_changes() {
    let (highlighter, _) = keywords();
    let mut field = field("let x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    let geometry = field.editable_geometry(&ctx, 380.0);
    let style = field.field_text_style();
    let align = field.horizontal_text_align();
    let before = field.build_display_widget(&geometry, &style, align);

    // New text: a new geometry.
    field.controller.set_text("let y = 2");
    let edited = field.editable_geometry(&ctx, 380.0);
    let after_edit = field.build_display_widget(&edited, &style, align);
    assert!(!Rc::ptr_eq(&before, &after_edit));

    // New style on the same geometry: the colours are baked into the paragraph.
    let mut restyled = style;
    restyled.font_size += 2;
    let after_restyle = field.build_display_widget(&edited, &restyled, align);
    assert!(!Rc::ptr_eq(&after_edit, &after_restyle));

    // The state the old widget was built for stays valid for its own key only.
    field.text_style = restyled;
    assert_eq!(field.field_text_style(), restyled);
}

/// The base colour of the text must reach the glyphs the ranges leave alone,
/// even though the widget that paints them is retained.
#[test]
fn a_new_text_colour_is_painted_after_a_retained_frame() {
    let (highlighter, _) = keywords();
    let mut field = field("let x = 1", Some(highlighter));
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);
    painted_runs(&field, &ctx);

    field.text_style = field.text_style.color(Color::Rgba(9, 99, 199, 255));
    field.update(&ctx);
    let runs = painted_runs(&field, &ctx);

    let plain_run = runs.iter().find(|run| run.0.contains('x')).expect("the rest of the line");
    assert_eq!(plain_run.1, [9, 99, 199, 255], "{runs:?}");
    assert_eq!(runs[0].1, KEYWORD, "the keyword keeps its own colour");
}

/// What highlighting costs per frame, plain against highlighted.
///
/// Run on its own, in a debug build:
/// `cargo nextest run -p aimer_input highlight_cost_profile --run-ignored only --no-capture`
#[test]
#[ignore = "profile harness; prints timings"]
fn highlight_cost_profile() {
    use std::time::Instant;

    // About 6 KB of code-like text: 160 lines of 8 tokens.
    let line = "let value_a = compute(left, right) + 42;\n";
    let text = line.repeat(160);
    // One range per word, alternating two colours, like a real tokenizer.
    let tokens = TextHighlighter::new(|text| {
        let mut ranges = Vec::new();
        let mut start = None;
        for (index, character) in text.char_indices() {
            match (character.is_alphanumeric() || character == '_', start) {
                (true, None) => start = Some(index),
                (false, Some(from)) => {
                    let shade = (ranges.len() % 2) as u8 * 80;
                    ranges.push(StyledRange::new(
                        from..index,
                        SpanStyle::new().color(Color::Rgba(40 + shade, 60, 200, 255)),
                    ));
                    start = None;
                }
                _ => {}
            }
        }
        ranges
    });
    let ctx = dummy_build_context(400.0, 600.0);
    const ROUNDS: u32 = 20;

    let measure = |label: &str, highlighter: Option<TextHighlighter>| {
        // A keystroke: new text, geometry rebuilt, frame updated and painted.
        let field = field(&text, highlighter.clone());
        field.update(&ctx);
        painted_runs(&field, &ctx);
        let started = Instant::now();
        for round in 0..ROUNDS {
            field.controller.set_text(format!("{text}{}", "x".repeat(round as usize % 3 + 1)));
            field.update(&ctx);
            painted_runs(&field, &ctx);
        }
        let keystroke = started.elapsed() / ROUNDS;

        // A caret move: same text, geometry rebuilt, frame updated and painted.
        let started = Instant::now();
        for round in 0..ROUNDS {
            let offset = (round as usize * 37) % 500;
            field.cursor.set_offset(offset);
            field.controller.set_selection_graphemes(offset, offset);
            field.update(&ctx);
            painted_runs(&field, &ctx);
        }
        let caret_move = started.elapsed() / ROUNDS;

        // An idle frame: nothing changed, only the blink ticks. The pieces a
        // frame is made of are timed apart, because the framework only paints
        // when `paint_needs_recording` says so.
        let mut update = std::time::Duration::ZERO;
        let mut can_paint = std::time::Duration::ZERO;
        let mut paint = std::time::Duration::ZERO;
        for _ in 0..ROUNDS {
            let started = Instant::now();
            field.update(&ctx);
            update += started.elapsed();
            let started = Instant::now();
            let _ = field.can_paint(&ctx);
            can_paint += started.elapsed();
            let started = Instant::now();
            painted_runs(&field, &ctx);
            paint += started.elapsed();
        }
        let (update, can_paint, paint) = (update / ROUNDS, can_paint / ROUNDS, paint / ROUNDS);
        println!(
            "{label:>12}: keystroke {keystroke:>10?}  caret move {caret_move:>10?}  | idle update {update:>10?}  can_paint {can_paint:>10?}  paint {paint:>10?}"
        );
    };

    measure("plain", None);
    measure("highlighted", Some(tokens));
}

/// Makes the first line 24 px while the field's own text is 16 px, the way a
/// note editor sets its title apart.
fn headline() -> TextHighlighter {
    TextHighlighter::new(|text| {
        let end = text.find('\n').unwrap_or(text.len());
        let mut style = SpanStyle::new();
        style.font_size = Some(24);
        vec![StyledRange::new(0..end, style)]
    })
}

/// A field that grows without a line limit, the shape of a `TextArea`.
fn growing_field(text: &str, highlighter: TextHighlighter) -> RawTextField {
    let mut field = focused_multiline_field(TextFieldController::with_initial(text), 1);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap).font_size(16);
    field.highlighter = Some(highlighter);
    field
}

/// The height the laid-out lines really stack to; `metrics.line_height` is only
/// the tallest of them.
fn stacked_height(field: &RawTextField, ctx: &BuildContext<'_>, width: f32) -> f32 {
    field
        .editable_geometry(ctx, width)
        .interaction
        .as_ref()
        .expect("the text is laid out")
        .metrics
        .height
}

const TEXT: &str = "Title\nb\nc\nd\ne\nf";

#[test]
fn a_larger_first_line_does_not_inflate_the_height_a_growing_field_asks_for() {
    let field = growing_field(TEXT, headline());
    let mut ctx = dummy_build_context(400.0, 600.0);
    // A scroll view hands its child an unbounded height.
    ctx.box_constraint.max_height = 1_000_000.0;
    let stacked = stacked_height(&field, &ctx, 392.0);

    let asked = field.computed_size(&ctx).height;

    // Four logical pixels of padding on each side.
    assert!(
        (asked - (stacked + 8.0)).abs() < 1.0,
        "asked for {asked}, the lines stack to {stacked} plus padding"
    );
}

#[test]
fn the_scroll_extent_of_a_mixed_size_field_is_the_overflow_of_its_real_height() {
    let field = growing_field(TEXT, headline());
    let ctx = dummy_build_context(400.0, 100.0);
    let stacked = stacked_height(&field, &ctx, 392.0);

    field.update(&ctx);

    let viewport = 100.0 - 8.0;
    assert!(
        (field.scroll_y_extent.get() - (stacked - viewport)).abs() < 1.0,
        "extent {} for lines stacking to {stacked} in a {viewport} viewport",
        field.scroll_y_extent.get()
    );
}

#[test]
fn the_caret_is_as_tall_as_the_line_it_stands_on() {
    let field = growing_field(TEXT, headline());
    let ctx = dummy_build_context(400.0, 600.0);
    let caret_height = |offset: usize| {
        field.cursor.set_offset(offset);
        field.update(&ctx);
        field
            .ime_cursor_area
            .get()
            .expect("a focused field publishes the caret it painted")
            .height
    };

    let on_title = caret_height(5);
    let on_body = caret_height(TEXT.len());

    // A 24 px line is one and a half times as tall as a 16 px one.
    assert!(
        on_title > on_body * 1.3,
        "caret is {on_title} tall on the title line and {on_body} on a body line"
    );
}

/// Hides the `# ` of a heading line while the cursor is not on it, the way a
/// markdown editor shows the rendered page and the source only under the cursor.
fn concealing_headings() -> TextHighlighter {
    TextHighlighter::with_selection(|text, context| {
        let mut ranges = Vec::new();
        let mut start = 0;
        for line in text.split('\n') {
            let end = start + line.len();
            if line.starts_with("# ") && !context.is_active(&(start..end)) {
                ranges.push(StyledRange::new(start..start + 2, SpanStyle::concealed()));
            }
            start = end + 1;
        }
        ranges
    })
}

/// The width of the first visual line of `field`.
fn first_line_width(field: &RawTextField, ctx: &BuildContext<'_>) -> f32 {
    field.editable_geometry(ctx, 380.0).visual_lines[0].width
}

#[test]
fn a_heading_marker_collapses_while_the_cursor_is_on_another_line() {
    let field = field("# Hello\nbody", Some(concealing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);

    field.cursor.set_offset(3);
    let on_the_heading = first_line_width(&field, &ctx);
    field.cursor.set_offset(10);
    let on_the_body = first_line_width(&field, &ctx);

    assert!(
        on_the_body < on_the_heading - 8.0,
        "the marker takes {on_the_heading} - {on_the_body} px only under the cursor"
    );
}

#[test]
fn a_heading_marker_comes_back_when_the_cursor_returns() {
    let field = field("# Hello\nbody", Some(concealing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(3);
    let before = first_line_width(&field, &ctx);

    field.cursor.set_offset(10);
    field.cursor.set_offset(3);

    assert!((first_line_width(&field, &ctx) - before).abs() < 0.01);
}

#[test]
fn every_marker_collapses_in_a_field_that_is_not_focused() {
    let field = field("# Hello\nbody", Some(concealing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(3);
    let focused = first_line_width(&field, &ctx);

    field.set_focused(false);

    assert!(first_line_width(&field, &ctx) < focused - 8.0);
}

#[test]
fn the_caret_stands_where_the_collapsed_text_is() {
    let field = field("# Hello\nbody", Some(concealing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(10);

    let geometry = field.editable_geometry(&ctx, 380.0);
    let start_of_hello = geometry.caret_geometry(2).unwrap();
    let start_of_line = geometry.caret_geometry(0).unwrap();

    assert!(
        start_of_hello.x - start_of_line.x < 3.0,
        "`Hello` begins {} px from the line start once the marker is hidden",
        start_of_hello.x - start_of_line.x
    );
}

/// Whether the `#` of the heading is painted in a visible colour by the frame
/// the framework would run for `field`: apply input, then record if the state
/// says so. Also returns whether that frame records again.
fn heading_marker_visible(field: &RawTextField, ctx: &BuildContext<'_>) -> (bool, bool) {
    assert!(field.sync_paint_state(ctx));
    let recorded = field.paint_needs_recording(ctx);
    let visible = painted_runs(field, ctx)
        .into_iter()
        .any(|(text, rgba)| text.contains('#') && rgba[3] > 0);
    (recorded, visible)
}

#[test]
fn a_click_that_moves_the_cursor_to_another_line_repaints_in_the_same_frame() {
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_widget::EventElement;

    let field = field("# Hello\nbody\nlast", Some(concealing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(14);
    heading_marker_visible(&field, &ctx);
    let (_, visible) = heading_marker_visible(&field, &ctx);
    assert!(!visible, "collapsed while the cursor is elsewhere");

    // A click on the first line, where the heading is.
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
        Vec2d { x: 24.0, y: 12.0 },
        PointerButton::Primary,
    )));
    let (recorded, visible) = heading_marker_visible(&field, &ctx);

    assert!(field.cursor.offset() <= 7, "the click put the cursor on the heading");
    assert!(recorded, "the frame that moved the cursor records again");
    assert!(visible, "the heading shows its marker in the frame of the click");
}

/// A heading that grows to 28 px and loses its `# ` while the cursor is on
/// another line, so moving the cursor shifts every line below it.
fn growing_headings() -> TextHighlighter {
    TextHighlighter::with_selection(|text, context| {
        let mut ranges = Vec::new();
        let mut start = 0;
        for line in text.split('\n') {
            let end = start + line.len();
            if line.starts_with("# ") && !context.is_active(&(start..end)) {
                ranges.push(StyledRange::new(start..start + 2, SpanStyle::concealed()));
                let mut style = SpanStyle::new();
                style.font_size = Some(28);
                ranges.push(StyledRange::new(start + 2..end, style));
            }
            start = end + 1;
        }
        ranges
    })
}

#[test]
fn a_click_lands_on_the_line_the_user_clicked_even_when_it_reflows_the_text() {
    use aimer_attribute::Vec2d;
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_widget::EventElement;

    // The cursor starts on the heading, which then shows as plain 16 px text.
    let field = field("# Hello\nbody\nlast", Some(growing_headings()));
    let ctx = dummy_build_context(400.0, 200.0);
    field.cursor.set_offset(3);
    assert!(field.sync_paint_state(&ctx));
    assert!(field.sync_paint_state(&ctx));
    let seen = field.editable_geometry(&ctx, 392.0);
    let body_top = seen.line_span(1, 18.0).0;
    let body_offset = 8;

    // The user clicks the `body` line as it is drawn.
    let _ = field.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
        Vec2d { x: 24.0, y: 4.0 + body_top + 6.0 },
        PointerButton::Primary,
    )));
    // The shared selection area has already moved the cursor there by the time
    // the frame applies the click, which is what makes the heading grow.
    field.cursor.set_offset(body_offset);
    assert!(field.sync_paint_state(&ctx));

    assert!(
        (body_offset..=12).contains(&field.cursor.offset()),
        "the click was on the body line but the cursor is at {}: it was hit-tested against the layout the click itself produced",
        field.cursor.offset()
    );
}

#[test]
fn the_highlight_context_follows_the_cursor_and_the_text() {
    let field = field("one\ntwo\nthree", Some(concealing_headings()));

    field.cursor.set_offset(5);
    let on_two = field.highlight_context("one\ntwo\nthree", None);
    assert_eq!(on_two.active_lines(), Some(4..7));
    assert_eq!(field.highlight_context("one\ntwo\nthree", None), on_two, "asked again, it is the same");

    field.cursor.set_offset(9);
    assert_eq!(field.highlight_context("one\ntwo\nthree", None).active_lines(), Some(8..13), "a moved cursor is not served from the memo");

    field.cursor.set_selection_anchor(Some(1));
    assert_eq!(field.highlight_context("one\ntwo\nthree", None).active_lines(), Some(0..13), "nor a new selection");

    field.set_focused(false);
    assert_eq!(field.highlight_context("one\ntwo\nthree", None).active_lines(), None, "nor a lost focus");
}

/// Reserves a 48 px tall, 18 px wide box over the first `[ ]`, the way an
/// embedded checkbox would.
fn checkbox_embed() -> TextHighlighter {
    TextHighlighter::new(|text| {
        text.find("[ ]")
            .map(|start| {
                StyledRange::new(start..start + 3, SpanStyle::new().inline_box(aimer_text::InlineBox::new(18.0, 48.0)))
            })
            .into_iter()
            .collect()
    })
}

#[test]
fn a_reserved_box_makes_the_field_as_tall_as_the_box_asks() {
    let plain = growing_field("- [ ] task\nnext", TextHighlighter::new(|_| Vec::new()));
    let boxed = growing_field("- [ ] task\nnext", checkbox_embed());
    let mut ctx = dummy_build_context(400.0, 600.0);
    ctx.box_constraint.max_height = 1_000_000.0;

    let plain_height = plain.computed_size(&ctx).height;
    let boxed_height = boxed.computed_size(&ctx).height;

    assert!(boxed_height >= plain_height + 25.0, "{plain_height} -> {boxed_height}");
    let stacked = stacked_height(&boxed, &ctx, 392.0);
    assert!((boxed_height - (stacked + 8.0)).abs() < 1.0, "the field asks for the real stacked height");
}

#[test]
fn the_box_is_not_painted_as_text_and_the_caret_jumps_across_it() {
    let field = growing_field("- [ ] task\nnext", checkbox_embed());
    let ctx = dummy_build_context(400.0, 600.0);
    field.update(&ctx);

    let runs = painted_runs(&field, &ctx);
    assert!(!runs.iter().any(|run| run.0.contains('[') || run.0.contains(']')), "the source of the box is not drawn: {runs:?}");

    let geometry = field.editable_geometry(&ctx, 392.0);
    let before = geometry.caret_geometry(2).expect("the caret can stand before the box");
    let after = geometry.caret_geometry(5).expect("and after it");
    assert!((after.x - before.x - 18.0).abs() < 0.5, "{} -> {}", before.x, after.x);
}

#[test]
fn the_scroll_extent_counts_the_height_of_a_box() {
    let field = growing_field("- [ ] task\nnext", checkbox_embed());
    let ctx = dummy_build_context(400.0, 40.0);
    let stacked = stacked_height(&field, &ctx, 392.0);

    field.update(&ctx);

    let viewport = 40.0 - 8.0;
    assert!(stacked > 48.0);
    assert!((field.scroll_y_extent.get() - (stacked - viewport)).abs() < 1.0, "{} for {stacked} in {viewport}", field.scroll_y_extent.get());
}
