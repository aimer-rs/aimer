//! Styled ranges on a [`RawTextWidget`]: they restyle the text without
//! touching its layout, which is what lets an editable field highlight code
//! while its caret, selection and hit testing keep using the same geometry.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_style::{LineHeight, TextAlign, TextOverflow, TextStyle};
use aimer_widget::base::{BuildContext, Color, WindowHandle};
use aimer_widget::{LayoutCache, LayoutElement};

use crate::{SpanStyle, StyledRange, TextSource};

use super::RawTextWidget;

const WIDTH: f32 = 140.0;

fn context(runtime: &tokio::runtime::Runtime) -> BuildContext<'static> {
    // The canvas outlives the test; leaking keeps the borrow `'static`.
    let inner: &'static InnerCanvas = Box::leak(Box::new(InnerCanvas::new()));
    let mut context = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: WIDTH,
            height: 400.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(winit::dpi::PhysicalSize::new(WIDTH as u32, 400), 1.0),
        runtime.handle().clone(),
    );
    context.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: WIDTH,
        max_height: 400.0,
    };
    context
}

fn widget(text: &'static str, ranges: Option<Rc<[StyledRange]>>) -> RawTextWidget {
    RawTextWidget {
        text: TextSource::Static(text),
        text_style: TextStyle::default().text_overflow(TextOverflow::Wrap),
        text_align: TextAlign::TopLeft,
        line_height: LineHeight::Normal,
        text_indent: 0.0,
        ranges,
        cache: LayoutCache::new(),
        _typeface: Cell::new(None),
    }
}

fn keyword(range: std::ops::Range<usize>) -> StyledRange {
    StyledRange::new(range, SpanStyle::new().color(Color::Rgba(200, 30, 30, 255)))
}

#[test]
fn ranges_make_the_widget_use_paragraph_layout() {
    let plain = widget("let x = 1", None);
    let ranged = widget("let x = 1", Some(Rc::from(vec![keyword(0..3)])));
    let empty = widget("let x = 1", Some(Rc::from(Vec::new())));

    assert!(!plain.uses_paragraph_layout());
    assert!(ranged.uses_paragraph_layout());
    assert!(!empty.uses_paragraph_layout(), "no ranges is the plain fast path");
}

/// A colour change must not move a single glyph: the caret of an editable
/// field is placed from this geometry.
#[test]
fn colour_only_ranges_leave_the_interaction_layout_unchanged() {
    const TEXT: &str = "fn main() {\n    let answer = 42;\n    println!(\"a fairly long string that wraps\");\n}";
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime);

    let ranges: Vec<StyledRange> = vec![
        keyword(0..2),
        keyword(3..7),
        keyword(16..19),
        keyword(32..38),
        keyword(60..90),
    ];
    let plain = widget(TEXT, None).interaction_layout(&ctx);
    let ranged = widget(TEXT, Some(Rc::from(ranges))).interaction_layout(&ctx);

    // Hard breaks plus a wrapped string, so the comparison covers both kinds.
    assert!(plain.lines.len() >= 5, "{} lines", plain.lines.len());
    assert_eq!(plain.lines.len(), ranged.lines.len(), "same line breaks");
    for (plain_line, ranged_line) in plain.lines.iter().zip(&ranged.lines) {
        assert_eq!(plain_line.text_range, ranged_line.text_range);
        assert!(
            (plain_line.width - ranged_line.width).abs() < 0.5,
            "line width {} vs {}",
            plain_line.width,
            ranged_line.width
        );
    }
    for offset in (0..=TEXT.len()).filter(|offset| TEXT.is_char_boundary(*offset)) {
        let plain_x = plain.caret_geometry(offset).map(|caret| (caret.x, caret.line_index));
        let ranged_x = ranged.caret_geometry(offset).map(|caret| (caret.x, caret.line_index));
        let (plain_x, ranged_x) = (plain_x.unwrap(), ranged_x.unwrap());
        assert_eq!(plain_x.1, ranged_x.1, "line of caret at byte {offset}");
        assert!(
            (plain_x.0 - ranged_x.0).abs() < 0.5,
            "caret x at byte {offset}: {} vs {}",
            plain_x.0,
            ranged_x.0
        );
    }
}

/// Without this the test above could pass for the wrong reason, with the
/// ranges ignored altogether: a heavier weight does change the shaped width.
#[test]
fn ranges_that_change_the_weight_do_change_the_layout() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime);
    let bold = StyledRange::new(
        0..8,
        SpanStyle::new().font_weight(aimer_style::FontWeight::Bold),
    );

    let plain = widget("wide wide wide", None).interaction_layout(&ctx);
    let ranged = widget("wide wide wide", Some(Rc::from(vec![bold]))).interaction_layout(&ctx);

    assert!(ranged.lines[0].width > plain.lines[0].width + 0.5);
}

/// The `DrawText` commands `widget` records, as `(text, rgba)` in paint order.
fn recorded_runs(widget: &RawTextWidget, ctx: &BuildContext<'_>) -> Vec<(String, [u8; 4])> {
    let tree = aimer_cupid::draw_cmd::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd::Rect::new(0.0, 0.0, WIDTH, 400.0))
        .unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        let canvas = aimer_canvas::Canvas::of(ctx);
        let _ = widget.computed_size(ctx);
        assert!(widget.record_local_v2_into(&canvas, ctx, Vec2d::default(), false, |_| {}));
        canvas.finish();
    });
    tree.draw_list_snapshot(root)
        .unwrap()
        .commands
        .iter()
        .filter_map(|command| match command {
            aimer_cupid::draw_cmd::DrawCommand::DrawText { text, color, .. } => {
                Some((text.to_string(), color.to_rgba8().0))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn each_range_is_painted_in_its_own_colour() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime);
    let base_color = TextStyle::default().color.to_rgba();

    let runs = recorded_runs(
        &widget("let x = 1", Some(Rc::from(vec![keyword(0..3)]))),
        &ctx,
    );

    assert_eq!(runs.len(), 2, "one keyword run and one plain run: {runs:?}");
    assert_eq!(runs[0].0, "let");
    assert_eq!(runs[0].1, [200, 30, 30, 255]);
    assert_eq!(runs[1].0, " x = 1");
    assert_eq!(
        (runs[1].1[0], runs[1].1[1], runs[1].1[2], runs[1].1[3]),
        base_color
    );
}

#[test]
fn a_range_background_is_painted_behind_the_text() {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime);
    let mention = StyledRange::new(
        3..9,
        SpanStyle::new().background_color(Color::Rgba(10, 20, 200, 255)),
    );
    let tree = aimer_cupid::draw_cmd::RenderTree::new();
    let root = tree
        .add_root(aimer_cupid::draw_cmd::Rect::new(0.0, 0.0, WIDTH, 400.0))
        .unwrap();
    let node_context = tree.context(root).unwrap();
    let ranged = widget("hi, @alice!", Some(Rc::from(vec![mention])));
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        let canvas = aimer_canvas::Canvas::of(ctx);
        let _ = ranged.computed_size(ctx);
        assert!(ranged.record_local_v2_into(&canvas, ctx, Vec2d::default(), false, |_| {}));
        canvas.finish();
    });
    let commands = tree.draw_list_snapshot(root).unwrap().commands;

    let fill = commands
        .iter()
        .position(|command| {
            matches!(command, aimer_cupid::draw_cmd::DrawCommand::FillRect { color, .. }
                if color.to_rgba8().0 == [10, 20, 200, 255])
        })
        .expect("the mention's background is filled");
    let first_text = commands
        .iter()
        .position(|command| matches!(command, aimer_cupid::draw_cmd::DrawCommand::DrawText { .. }))
        .expect("text is drawn");
    assert!(fill < first_text, "the background goes under the glyphs");
}

/// A range that reserves a box: the widget's height and interaction layout
/// follow the box, and the source under it stays addressable.
#[test]
fn a_range_with_an_inline_box_resolves_the_height_of_its_line() {
    use aimer_widget::LayoutElement;

    const TEXT: &str = "- [ ] task\nnext";
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let ctx = context(&runtime);
    let plain = widget(TEXT, None);
    let embed = StyledRange::new(2..5, SpanStyle::new().inline_box(crate::InlineBox::new(18.0, 60.0)));
    let boxed = widget(TEXT, Some(Rc::from(vec![embed])));

    let plain_height = plain.computed_size(&ctx).height;
    let boxed_height = boxed.computed_size(&ctx).height;
    let interaction = boxed.interaction_layout(&ctx);

    assert!(boxed.uses_paragraph_layout());
    assert!(boxed_height >= plain_height + 30.0, "the 60 px box makes the first line taller: {plain_height} -> {boxed_height}");
    assert!(interaction.lines[0].ascent + (-interaction.lines[0].descent) >= 60.0 - 0.01);
    assert!(interaction.lines[1].ascent < 30.0, "the line after it is unaffected");
    let cluster = interaction.clusters.iter().find(|cluster| cluster.text_range == (2..5)).expect("the box is one cluster");
    assert!((cluster.end_x - cluster.start_x - 18.0).abs() < 0.01);
    let before = interaction.caret_geometry(2).unwrap();
    let after = interaction.caret_geometry(5).unwrap();
    assert!((after.x - before.x - 18.0).abs() < 0.01, "the caret jumps across the box");
    assert_eq!(interaction.text, TEXT, "the string itself is untouched");
}
