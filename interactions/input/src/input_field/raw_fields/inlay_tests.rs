//! A field with inlays: notes that take room in a line, and are drawn in it, while the text stays what it is.

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_style::TextOverflow;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, LayoutElement};

use super::test_support::{dummy_build_context, focused_multiline_field};
use super::RawTextField;
use crate::input_field::controller::TextFieldController;
use crate::input_field::inlay::{Inlay, Inlays};

const GREY: Color = Color::Rgba(120, 124, 135, 255);
const TEXT: &str = "let x = 1;\nlet y = 2;";

fn field(text: &str, inlays: Option<Inlays>) -> RawTextField {
    let mut field = focused_multiline_field(TextFieldController::with_initial(text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.inlays = inlays;
    field
}

fn hint(offset: usize, label: &str) -> Inlays {
    Inlays::new(vec![Inlay::new(offset, label, GREY)])
}

/// Where the caret stands at each offset, from the start to `text.len()`.
fn carets(field: &RawTextField, ctx: &BuildContext<'_>, count: usize) -> Vec<f32> {
    let geometry = field.editable_geometry(ctx, 380.0);
    (0..=count).map(|offset| geometry.caret_geometry(offset).expect("a laid out caret").x).collect()
}

fn gap_of(field: &RawTextField, ctx: &BuildContext<'_>) -> f32 {
    field.editable_geometry(ctx, 380.0).inlays.first().expect("a place for the hint").gap
}

#[test]
fn a_hint_makes_room_in_its_line_after_the_character_before_it_and_moves_what_follows() {
    let ctx = dummy_build_context(400.0, 200.0);
    let plain = carets(&field(TEXT, None), &ctx, TEXT.len());
    let hinted = field(TEXT, Some(hint(5, ": i32")));
    let moved = carets(&hinted, &ctx, TEXT.len());
    let gap = gap_of(&hinted, &ctx);

    assert!(gap > 10.0, "five characters are wider than {gap}");
    for offset in 0..=5 {
        assert!((moved[offset] - plain[offset]).abs() < 0.01, "before the room, at {offset}: {} and {}", moved[offset], plain[offset]);
    }
    for offset in 6..=10 {
        assert!(
            (moved[offset] - plain[offset] - gap).abs() < 0.01,
            "after the room, at {offset}: {} and {} and {gap}",
            moved[offset],
            plain[offset]
        );
    }
    // The next line is not touched.
    for offset in 11..=TEXT.len() {
        assert!((moved[offset] - plain[offset]).abs() < 0.01, "on the next line, at {offset}");
    }
}

#[test]
fn the_room_is_in_physical_pixels_on_a_screen_that_has_more_of_them() {
    let mut ctx = dummy_build_context(400.0, 200.0);
    ctx.scale = 2.0;
    let plain = carets(&field(TEXT, None), &ctx, 8);
    let hinted = field(TEXT, Some(hint(5, ": i32")));
    let moved = carets(&hinted, &ctx, 8);
    let gap = gap_of(&hinted, &ctx);
    assert!(
        (moved[7] - plain[7] - gap * 2.0).abs() < 0.05,
        "{} px of room for {gap} logical ones, at a scale of 2: {} and {}",
        moved[7] - plain[7],
        moved[7],
        plain[7]
    );
}

#[test]
fn the_text_is_the_text_it_was_and_the_hint_is_not_in_it() {
    let ctx = dummy_build_context(400.0, 200.0);
    let hinted = field(TEXT, Some(hint(5, ": i32")));
    let _ = hinted.editable_geometry(&ctx, 380.0);
    assert_eq!(hinted.controller.text(), TEXT);
    assert_eq!(hinted.display_text(), TEXT);
    assert_eq!(hinted.editable_geometry(&ctx, 380.0).display.as_ref(), TEXT);
}

#[test]
fn a_hint_that_has_no_place_takes_no_room() {
    let ctx = dummy_build_context(400.0, 200.0);
    let plain = carets(&field(TEXT, None), &ctx, TEXT.len());
    // The start of the text, the start of a line, and past the end.
    for offset in [0, 11, TEXT.len() + 5] {
        let hinted = field(TEXT, Some(hint(offset, ": i32")));
        assert_eq!(carets(&hinted, &ctx, TEXT.len()), plain, "a hint at {offset}");
        assert!(hinted.editable_geometry(&ctx, 380.0).inlays.is_empty());
    }
}

#[test]
fn a_field_that_hides_its_text_shows_no_hints() {
    let ctx = dummy_build_context(400.0, 200.0);
    let mut secret = field("hunter2", Some(hint(3, ": i32")));
    secret.input_type = super::InputType::Password;
    assert!(secret.editable_geometry(&ctx, 380.0).inlays.is_empty());
}

/// What `field` paints: the text of its glyph runs, and the pills.
fn painted(field: &RawTextField, ctx: &BuildContext<'_>) -> (Vec<String>, Vec<(f32, f32, f32)>) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 200.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    let list = tree.draw_list_snapshot(root).unwrap();
    let runs = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { text, .. } => Some(text.to_string()),
            _ => None,
        })
        .collect();
    let pills = list
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { rect, border_radius, .. } if border_radius[0] > 0.0 => Some((rect.x, rect.width, border_radius[0])),
            _ => None,
        })
        .collect();
    (runs, pills)
}

#[test]
fn the_label_of_a_hint_is_painted_and_the_text_around_it_is_too() {
    let ctx = dummy_build_context(400.0, 200.0);
    let hinted = field(TEXT, Some(hint(5, ": i32")));
    hinted.update(&ctx);
    let (runs, pills) = painted(&hinted, &ctx);
    assert!(runs.iter().any(|run| run == ": i32"), "{runs:?}");
    assert!(runs.iter().any(|run| run.contains("let")), "{runs:?}");
    assert!(pills.is_empty(), "no pill was asked for: {pills:?}");
}

#[test]
fn a_hint_in_a_pill_has_a_rounded_rectangle_that_holds_its_label() {
    let ctx = dummy_build_context(400.0, 200.0);
    let pill = Inlays::new(vec![Inlay::new(5, ": i32", GREY).background(Color::Rgba(90, 60, 80, 255)).padded(true, false)]).radius(5.0);
    let hinted = field(TEXT, Some(pill));
    hinted.update(&ctx);
    let gap = gap_of(&hinted, &ctx);
    let (runs, pills) = painted(&hinted, &ctx);
    assert!(runs.iter().any(|run| run == ": i32"), "{runs:?}");
    assert_eq!(pills.len(), 1, "{pills:?}");
    let (_, width, radius) = pills[0];
    assert!(radius > 0.0 && radius <= 5.0, "{radius}");
    // The room is the pill, its pad, and the inset inside the pill on both sides: so the pill is less than the room.
    assert!(width < gap && width > gap - 4.0 - 1.0, "a pill of {width} in {gap}");
}

#[test]
fn the_caret_is_before_the_hint_and_a_click_after_it_is_after_it() {
    let ctx = dummy_build_context(400.0, 200.0);
    let hinted = field(TEXT, Some(hint(5, ": i32")));
    let geometry = hinted.editable_geometry(&ctx, 380.0);
    let plain = field(TEXT, None).editable_geometry(&ctx, 380.0);
    let (before, after) = (geometry.caret_geometry(5).unwrap(), geometry.caret_geometry(6).unwrap());
    let y = before.y + before.height * 0.5;
    assert_eq!(geometry.hit_test(plain.caret_geometry(4).unwrap().x + 0.5, y), Some(4), "in the `x`");
    assert_eq!(geometry.hit_test(after.x + 0.5, y), Some(6), "on the space after the room");
    assert_eq!(geometry.hit_test(after.x - 0.5, y), Some(6), "at the end of the room, which is the start of what follows");
    assert!(after.x - before.x > 10.0, "the room is between them");
}

/// A long note of the same short line, which a field with a low threshold lays out a window of.
fn windowed(inlays: Option<Inlays>) -> RawTextField {
    let mut field = field(&"let x = 1;\n".repeat(200), inlays);
    field.max_lines = Some(10);
    field.window_threshold = 100;
    field
}

#[test]
fn a_window_of_a_long_text_has_the_room_too_in_the_offsets_of_the_whole_text() {
    let ctx = dummy_build_context(400.0, 200.0);
    let plain = windowed(None);
    let hinted = windowed(Some(hint(3 * 11 + 5, ": i32")));
    // A field that is made with its text has the caret at the end of it, and scrolls there.
    hinted.cursor.set_offset(0);
    let (plain_geometry, geometry) = (plain.editable_geometry(&ctx, 380.0), hinted.editable_geometry(&ctx, 380.0));
    assert!(geometry.window.is_some(), "the text is laid out a window at a time");
    let gap = geometry.inlays.first().expect("the line is in the window").gap;
    let at = |geometry: &crate::editable_text::EditableGeometry, offset: usize| geometry.caret_geometry(offset).unwrap().x;
    let start = 3 * 11;
    assert!((at(&geometry, start + 5) - at(&plain_geometry, start + 5)).abs() < 0.01, "the caret is before the hint");
    assert!((at(&geometry, start + 6) - at(&plain_geometry, start + 6) - gap).abs() < 0.01, "and what follows is after the room");
    // The line below it is untouched.
    assert!((at(&geometry, start + 11 + 6) - at(&plain_geometry, start + 11 + 6)).abs() < 0.01);

    hinted.update(&ctx);
    let (runs, _) = painted(&hinted, &ctx);
    assert!(runs.iter().any(|run| run == ": i32"), "{runs:?}");
}

#[test]
fn a_hint_in_a_window_that_is_not_laid_out_is_not_placed() {
    let ctx = dummy_build_context(400.0, 200.0);
    // Line 190 is far from the ten lines on screen.
    let hinted = windowed(Some(hint(190 * 11 + 5, ": i32")));
    assert!(hinted.editable_geometry(&ctx, 380.0).inlays.is_empty());
}

#[test]
fn a_hint_stays_beside_its_text_while_text_is_typed_before_it() {
    let ctx = dummy_build_context(400.0, 200.0);
    let made_for = aimer_text_buffer::TextSnapshot::from_text(TEXT);
    let hinted = field(TEXT, Some(hint(5, ": i32").of_text(made_for)));
    let gap = gap_of(&hinted, &ctx);
    // `ab` is typed at the start of the first line: the `x` is two bytes along, and so is the hint.
    hinted.controller.insert_str("ab", 0);
    let geometry = hinted.editable_geometry(&ctx, 380.0);
    assert_eq!(geometry.inlays.first().map(|place| place.at), Some(7));
    let plain = {
        let plain = field(&format!("ab{TEXT}"), None);
        carets(&plain, &ctx, 8)
    };
    let moved = carets(&hinted, &ctx, 8);
    assert!((moved[7] - plain[7]).abs() < 0.01, "the caret is still before the hint");
    assert!((moved[8] - plain[8] - gap).abs() < 0.01, "and the room is after the `x` it was for");

    // What was typed in the middle of what a hint was beside takes the hint away: it is for text that is not there.
    let replaced = field(TEXT, Some(hint(5, ": i32").of_text(aimer_text_buffer::TextSnapshot::from_text(TEXT))));
    replaced.controller.delete_range(4, 5);
    replaced.controller.insert_str("name", 4);
    assert_eq!(replaced.editable_geometry(&ctx, 380.0).inlays.first().map(|place| place.at), Some(8));
}

#[test]
fn a_hint_at_the_end_of_a_line_takes_no_room_and_is_drawn_after_the_last_character_where_it_overlaps_nothing() {
    let ctx = dummy_build_context(400.0, 200.0);
    // The end of the first line, before its line break: the chaining hints of a server are there.
    let end = TEXT.find('\n').unwrap();
    let pill = Inlays::new(vec![Inlay::new(end, ": u32", GREY).background(Color::Rgb(90, 60, 80)).padded(true, false)]);
    let hinted = field(TEXT, Some(pill));
    hinted.update(&ctx);
    let plain = carets(&field(TEXT, None), &ctx, TEXT.len());
    assert_eq!(carets(&hinted, &ctx, TEXT.len()), plain, "no text moved, and no caret");
    let geometry = hinted.editable_geometry(&ctx, 380.0);
    let place = geometry.inlays.first().expect("placed");
    assert!(place.eol);
    let (x, _) = geometry.inlay_origin(place, 1.0).expect("laid out");
    assert!((x - plain[end]).abs() < 0.01, "it starts where the line ends: {x} and {}", plain[end]);
    let (runs, pills) = painted(&hinted, &ctx);
    assert!(runs.iter().any(|run| run == ": u32"), "{runs:?}");
    assert_eq!(pills.len(), 1);
    // The pill is after the text of the line, and the pad is before the pill.
    assert!(pills[0].0 > plain[end], "a pill at {} for a line that ends at {}", pills[0].0, plain[end]);
}

#[test]
fn a_hint_at_the_very_end_of_the_text_is_at_the_end_of_its_line_too() {
    let ctx = dummy_build_context(400.0, 200.0);
    let hinted = field("let x", Some(hint(5, ": i32")));
    let place = hinted.editable_geometry(&ctx, 380.0).inlays.first().cloned().expect("placed");
    assert_eq!((place.at, place.eol), (5, true));
}

#[test]
fn an_underlined_name_is_underlined_to_where_it_ends_and_not_under_the_hint_that_follows_it() {
    use crate::input_field::highlight::TextHighlighter;
    use aimer_style::TextDecoration;
    use aimer_text::{SpanStyle, StyledRange};

    let ctx = dummy_build_context(400.0, 200.0);
    // `x` of `let x = 1;` is underlined, as a mutable variable is, and a hint follows it.
    let mut hinted = field(TEXT, Some(hint(5, ": i32")));
    hinted.highlighter = Some(TextHighlighter::new(|_| {
        vec![StyledRange::new(4..5, SpanStyle::new().text_decoration(TextDecoration::Underline))]
    }));
    let ranges = hinted.editable_geometry(&ctx, 380.0).ranges.clone().expect("styled");
    let underlined: Vec<_> = ranges.iter().filter(|run| run.style.text_decoration.is_some()).map(|run| run.range.clone()).collect();
    assert_eq!(underlined, [4..5], "the name and nothing else: not the room, and not the space that has it");
    let spaced = ranges.iter().find(|run| run.range == (5..6)).expect("the space has the room");
    assert!(spaced.style.word_spacing.is_some_and(|room| room > 0.0));
    assert!(spaced.style.text_decoration.is_none());
}
