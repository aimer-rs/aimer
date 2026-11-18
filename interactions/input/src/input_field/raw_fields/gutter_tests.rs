//! The margin of line numbers, the band behind the caret's line, and the caret's line and column.

use aimer_cupid::draw_cmd::{Color as DrawColor, DrawCommand, Rect, RenderTree};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{Drawable, LayoutElement};

use super::test_support::{dummy_build_context, focused_multiline_field};
use super::RawTextField;
use crate::TextEditingController;
use crate::editable_text::TextEditingValue;
use crate::input_field::gutter::LineNumbers;

const BAND: Color = Color::Rgba(40, 60, 90, 255);

fn field(text: &str, numbers: Option<LineNumbers>, band: Option<Color>, wrap: bool) -> (RawTextField, BuildContext<'static>) {
    let mut field = focused_multiline_field(TextEditingController::with_text(text), 3);
    field.text_style = field.text_style.text_overflow(aimer_style::TextOverflow::Wrap);
    field.line_numbers = numbers;
    field.current_line = band;
    field.wrap = wrap;
    field.max_lines = Some(8);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);
    (field, ctx)
}

fn commands(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 300.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

/// The text runs that are drawn, with where each starts: (text, x, y).
fn runs(commands: &[DrawCommand]) -> Vec<(String, f32, f32)> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { text, position, .. } => Some((text.to_string(), position.x, position.y)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_margin_is_part_of_the_left_padding_and_grows_with_the_number_of_digits() {
    let lines = |count: usize| (0..count).map(|n| format!("line {n}\n")).collect::<String>();
    let (plain, ctx) = field(&lines(5), None, None, true);
    let (numbered, _) = field(&lines(5), Some(LineNumbers::new()), None, true);
    let plain_layout = plain.local_v2_layout(&ctx).expect("laid out");
    let numbered_layout = numbered.local_v2_layout(&ctx).expect("laid out");
    assert_eq!(plain.gutter_width(1.0), 0.0);
    let margin = numbered.gutter_width(1.0);
    assert!(margin > 24.0, "gaps and two digits: {margin}");
    assert_eq!(numbered_layout.padding_left, plain_layout.padding_left + margin);
    assert_eq!(numbered_layout.content_width, plain_layout.content_width - margin);

    // Past 99 lines a third digit is needed; 999 and 9999 widen it again.
    let (hundred, _) = field(&lines(120), Some(LineNumbers::new()), None, true);
    assert!(hundred.gutter_width(1.0) > margin, "three digits are wider than two");
    // The least digits keeps it from growing as a file does.
    let (steady, _) = field(&lines(5), Some(LineNumbers::new().min_digits(4)), None, true);
    assert!(steady.gutter_width(1.0) > margin);
}

#[test]
fn a_number_is_drawn_for_each_line_left_of_the_text() {
    let (field, ctx) = field("alpha\nbeta\ngamma\ndelta", Some(LineNumbers::new()), None, true);
    let runs = runs(&commands(&field, &ctx));
    let text_x = runs.iter().filter(|run| run.0.contains("alpha")).map(|run| run.1).fold(f32::MAX, f32::min);
    let numbers: String = runs.iter().filter(|run| run.1 < text_x).map(|run| run.0.clone()).collect();
    assert_eq!(numbers.replace('\n', ""), "1234", "{runs:?}");
}

#[test]
fn a_line_that_wraps_has_its_number_once_and_the_next_number_is_where_its_text_is() {
    let long = "word ".repeat(40);
    let text = format!("{long}\nsecond");
    let (field, ctx) = field(&text, Some(LineNumbers::new()), None, true);
    let rows = field.geometry_cache.latest().expect("laid out").visual_lines.len();
    assert!(rows >= 4, "the first line wraps: {rows} rows");
    let runs = runs(&commands(&field, &ctx));
    let text_x = runs.iter().filter(|run| run.0.contains("word") || run.0.contains("second")).map(|run| run.1).fold(f32::MAX, f32::min);
    let numbers: Vec<&(String, f32, f32)> = runs.iter().filter(|run| run.1 < text_x).collect();
    assert_eq!(numbers.iter().map(|run| run.0.as_str()).collect::<String>(), "12", "{runs:?}");
    // 1 is on the first row, and 2 on the row of the second line: one row of the field's text for
    // each row the first line wraps onto, below it.
    let line_height = field.local_v2_layout(&ctx).expect("laid out").line_height / 1.0;
    let gap = numbers[1].2 - numbers[0].2;
    assert!((gap - (rows - 1) as f32 * line_height).abs() < 2.0, "{gap} for {rows} rows of {line_height}");
}

#[test]
fn the_band_is_drawn_behind_the_caret_line_across_the_whole_field_and_only_when_asked() {
    let text = "one\ntwo\nthree\nfour";
    let (with, ctx) = field(text, Some(LineNumbers::new()), Some(BAND), false);
    with.cursor.set_offset(5);
    with.update(&ctx);
    let band: DrawColor = {
        let (r, g, b, a) = BAND.to_rgba();
        DrawColor::rgba8(r, g, b, a)
    };
    let fills = |field: &RawTextField| {
        commands(field, &ctx)
            .into_iter()
            .filter_map(|command| match command {
                DrawCommand::FillRect { rect, color, .. } if color == band => Some(rect),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let bands = fills(&with);
    assert_eq!(bands.len(), 1, "one band: {bands:?}");
    let layout = with.local_v2_layout(&ctx).expect("laid out");
    let rect = bands[0];
    assert!((rect.width - layout.box_width).abs() < 1.0, "across the whole field: {rect:?}");
    assert!((rect.height - layout.line_height).abs() < 1.5, "one row tall: {rect:?}");
    // The caret is on the second line: the band is one row below the first.
    with.cursor.set_offset(0);
    with.update(&ctx);
    let first = fills(&with)[0];
    assert!((rect.y - first.y - layout.line_height).abs() < 1.5, "a row lower for the next line: {} and {}", first.y, rect.y);

    let (without, _) = field(text, None, None, false);
    assert!(fills(&without).is_empty(), "no band unless asked for");
}

#[test]
fn the_line_and_column_of_a_byte_count_graphemes_and_start_at_the_line() {
    for large in [false, true] {
        let text = "alpha\nbet\u{e9} \u{1f468}\u{200d}\u{1f469}x\nlast";
        let value = if large { TextEditingValue::with_str(&text.repeat(1500)) } else { TextEditingValue::with_str(text) };
        // The start of the text, the middle of a line, a line start, after an accent and an emoji.
        assert_eq!(value.line_and_column(0), (0, 0));
        assert_eq!(value.line_and_column(3), (0, 3));
        assert_eq!(value.line_and_column(6), (1, 0), "just after the first line feed");
        assert_eq!(value.line_and_column(6 + "bet\u{e9} ".len()), (1, 5), "an accented letter is one grapheme, in two bytes");
        assert_eq!(value.line_and_column(6 + "bet\u{e9} \u{1f468}\u{200d}\u{1f469}".len()), (1, 6), "a family emoji is one");
        assert_eq!(value.line_and_column(text.len() - 4), (2, 0));
        assert_eq!(value.line_and_column(usize::MAX).0, if large { 2 * 1500 } else { 2 }, "past the end is the end");
    }
}

#[test]
fn the_lines_of_a_text_are_one_more_than_its_line_feeds() {
    for text in ["", "one", "one\n", "a\nb\nc", "\n\n"] {
        assert_eq!(TextEditingValue::with_str(text).line_count(), text.matches('\n').count() + 1, "{text:?}");
    }
    assert_eq!(TextEditingValue::with_str(&"x\n".repeat(5000)).line_count(), 5001, "a long text answers from its tree");
}

#[test]
fn a_controller_reports_the_caret_as_a_status_bar_does() {
    let controller = TextEditingController::with_text("fn main() {\n    println!(\"hi\");\n}");
    assert_eq!(controller.cursor_position(), (3, 2), "the caret is at the end of the text");
    controller.set_selection_graphemes(0, 0);
    assert_eq!(controller.cursor_position(), (1, 1));
    assert_eq!(controller.line_count(), 3);
}

#[test]
fn a_listener_hears_every_change_until_it_is_dropped() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let controller = TextEditingController::with_text("one two");
    let heard: Rc<RefCell<Vec<(usize, usize)>>> = Rc::default();
    let sink = Rc::clone(&heard);
    let positions = controller.clone();
    let listener = controller.listen(move |_| sink.borrow_mut().push(positions.cursor_position()));

    controller.set_selection_graphemes(0, 3);
    controller.set_text("x\ny");
    assert!(controller.undo());
    assert_eq!(heard.borrow().len(), 3, "a selection, a text and an undo: {:?}", heard.borrow());
    assert_eq!(heard.borrow()[0], (1, 4), "the caret is at the end of the selection, seen after the change");

    drop(listener);
    controller.set_text("quiet");
    assert_eq!(heard.borrow().len(), 3, "a dropped listener hears nothing more");
}

#[test]
fn a_value_writes_its_text_to_a_writer_whole_in_order_short_or_long() {
    for text in ["", "one\ntwo", "caf\u{e9} \u{4f60}\u{597d}\r\n", &"a line of a long text, with caf\u{e9}\n".repeat(900)] {
        let value = TextEditingValue::with_str(text);
        let mut out = Vec::new();
        value.write_to(&mut out).expect("a Vec takes writes");
        assert_eq!(out, text.as_bytes(), "{} bytes", text.len());
    }
    // A writer that fails ends the write with its error.
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = TextEditingValue::with_str("x").write_to(&mut Full).unwrap_err();
    assert_eq!(error.to_string(), "disk full");
}

/// A line much taller than the others, as a rendered heading is, must not move the numbers after it
/// off their lines: each number stands on the baseline of its own line.
#[test]
fn a_number_is_on_its_own_line_even_when_the_lines_are_not_the_same_height() {
    use crate::input_field::highlight::TextHighlighter;
    use aimer_text::{SpanStyle, StyledRange};

    let text = "alpha\nbeta\ngamma\ndelta";
    let tall = TextHighlighter::new(|text| vec![StyledRange::new(0..text.find('\n').unwrap(), SpanStyle::new().font_size(40))]);
    let mut field = focused_multiline_field(TextEditingController::with_text(text), 3);
    field.text_style = field.text_style.text_overflow(aimer_style::TextOverflow::Wrap);
    field.line_numbers = Some(LineNumbers::new());
    field.wrap = true;
    field.max_lines = Some(8);
    field.highlighter = Some(tall);
    field.cursor.set_offset(0);
    let ctx = dummy_build_context(400.0, 300.0);
    field.update(&ctx);

    let runs = runs(&commands(&field, &ctx));
    let y_of = |word: &str| runs.iter().find(|run| run.0.contains(word)).map(|run| run.2).unwrap_or_else(|| panic!("{word} in {runs:?}"));
    let text_x = runs.iter().filter(|run| run.0.contains("beta")).map(|run| run.1).fold(f32::MAX, f32::min);
    let numbers: Vec<&(String, f32, f32)> = runs.iter().filter(|run| run.1 < text_x && run.0.trim().parse::<usize>().is_ok()).collect();
    assert_eq!(numbers.len(), 4, "{runs:?}");
    // The distance between the numbers is the distance between their lines, whatever the heights.
    for (number, (from, to)) in [("beta", "gamma"), ("gamma", "delta"), ("alpha", "beta")].into_iter().enumerate() {
        let (first, second) = match number {
            0 => (1, 2),
            1 => (2, 3),
            _ => (0, 1),
        };
        let numbers_gap = numbers[second].2 - numbers[first].2;
        let lines_gap = y_of(to) - y_of(from);
        assert!((numbers_gap - lines_gap).abs() < 3.0, "numbers {first}->{second} are {numbers_gap} apart, the lines {from}->{to} {lines_gap}");
    }
}

/// A row an embedded widget stands in for has its number on its first line, at the top, and not down on
/// the baseline at the bottom of the box.
#[test]
fn the_number_of_a_row_with_an_embedded_widget_is_at_its_top() {
    use super::embed_tests::Probe;
    use crate::input_field::embed::{TextEmbed, TextEmbedder};

    let probe = Probe::new(120.0, std::rc::Rc::default());
    let embedder = TextEmbedder::new(move |text, _| {
        let probe = probe.clone();
        vec![TextEmbed::new(0..text.find('\n').unwrap(), 9, move || probe.clone())]
    });
    let mut config = super::test_support::field_config(TextEditingController::with_text("alpha\nbeta\ngamma"));
    config.line_numbers = Some(LineNumbers::new());
    config.embedder = Some(embedder);
    config.max_lines = Some(8);
    config.wrap = true;
    let field = RawTextField::new(config, crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
    field.cursor.set_offset(8);
    let ctx = dummy_build_context(400.0, 400.0);
    field.update(&ctx);
    let layout = field.local_v2_layout(&ctx).expect("laid out");
    let runs = runs(&commands(&field, &ctx));
    let text_x = runs.iter().filter(|run| run.0.contains("beta")).map(|run| run.1).fold(f32::MAX, f32::min);
    let numbers: Vec<&(String, f32, f32)> = runs.iter().filter(|run| run.1 < text_x && run.0.trim().parse::<usize>().is_ok()).collect();
    assert_eq!(numbers.len(), 3, "{runs:?}");
    assert!(numbers[0].2 < layout.padding_top + 20.0, "the number of the embedded row is at {}, not at the bottom of a 120 px box", numbers[0].2);
    assert!(numbers[1].2 > numbers[0].2 + 100.0, "and the next line's number is below the box: {numbers:?}");
}

#[test]
fn no_band_is_drawn_for_a_caret_inside_an_embedded_widget() {
    use super::embed_tests::Probe;
    use crate::input_field::embed::{TextEmbed, TextEmbedder};

    let make = |caret: usize| {
        let probe = Probe::new(80.0, std::rc::Rc::default());
        let embedder = TextEmbedder::new(move |text, _| {
            let probe = probe.clone();
            vec![TextEmbed::new(0..text.find('\n').unwrap(), 9, move || probe.clone())]
        });
        let mut config = super::test_support::field_config(TextEditingController::with_text("alpha\nbeta\ngamma"));
        config.line_numbers = Some(LineNumbers::new());
        config.current_line = Some(BAND);
        config.embedder = Some(embedder);
        config.max_lines = Some(8);
        config.wrap = true;
        let field = RawTextField::new(config, crate::input_field::caret::CaretBlink::new(), aimer_widget::FocusNode::new());
        field.cursor.set_offset(caret);
        let ctx = dummy_build_context(400.0, 400.0);
        field.update(&ctx);
        let drawn = commands(&field, &ctx);
        let (r, g, b, a) = BAND.to_rgba();
        let band = DrawColor::rgba8(r, g, b, a);
        drawn.iter().filter(|command| matches!(command, DrawCommand::FillRect { color, .. } if *color == band)).count()
    };
    assert_eq!(make(2), 0, "the caret is inside the embedded lines");
    assert_eq!(make(8), 1, "a caret on an ordinary line still has its band");
}
