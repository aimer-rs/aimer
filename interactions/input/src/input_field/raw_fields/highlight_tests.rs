//! A field with a highlighter: the text is styled by range, and everything the
//! user touches — caret, selection, hit testing — keeps working because the
//! layout they read is the layout of the styled text.

use std::cell::Cell;
use std::rc::Rc;

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_style::TextOverflow;
use aimer_text::{SpanStyle, StyledRange};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::Drawable;

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
