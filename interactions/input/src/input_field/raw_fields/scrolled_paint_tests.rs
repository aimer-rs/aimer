//! A field longer than its viewport paints its text shifted up by how far it has
//! scrolled. The renderer decides from the box a text run declares whether the run
//! can show a pixel at all, and prepares nothing for one that cannot; so the box
//! has to be the text's own, not the viewport's, or scrolling moves it off the
//! screen while its glyphs are still on it and the field draws no text.

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_style::TextOverflow;
use aimer_widget::base::BuildContext;
use aimer_widget::{Drawable, LayoutElement};

use super::RawTextField;
use super::test_support::{dummy_build_context, focused_multiline_field};
use crate::input_field::controller::TextFieldController;

const LINES: usize = 60;

fn field() -> RawTextField {
    let text = (0..LINES)
        .map(|index| format!("Line {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut field = focused_multiline_field(TextFieldController::with_initial(text), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field
}

/// The `bounds_height` of every text run `field` paints, and where each starts.
fn painted_boxes(field: &RawTextField, ctx: &BuildContext<'_>) -> Vec<(f32, Option<f32>)> {
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
            DrawCommand::DrawText {
                position,
                bounds_height,
                ..
            } => Some((position.y, *bounds_height)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_scrolled_field_declares_a_text_box_that_covers_its_text() {
    let field = field();
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);

    let geometry = field.editable_geometry(&ctx, 400.0);
    let line_height = geometry
        .interaction
        .as_ref()
        .expect("the text is laid out")
        .metrics
        .line_height;
    let total_height = LINES as f32 * line_height;

    let boxes = painted_boxes(&field, &ctx);

    assert!(!boxes.is_empty(), "the field paints its text");
    for (_, bounds_height) in boxes {
        let bounds_height = bounds_height.expect("a wrapped run declares its box");
        assert!(
            bounds_height >= total_height * 0.99,
            "the run declares {bounds_height}px for text {total_height}px tall: a box the \
             size of the viewport leaves the renderer to judge the run off screen once the \
             field has scrolled"
        );
    }
}

#[test]
fn a_short_field_keeps_declaring_its_viewport() {
    let controller = TextFieldController::with_initial("one\ntwo");
    let mut field = focused_multiline_field(controller, 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    let ctx = dummy_build_context(400.0, 200.0);
    field.update(&ctx);

    let boxes = painted_boxes(&field, &ctx);

    assert!(!boxes.is_empty());
    let viewport = boxes[0].1.expect("a wrapped run declares its box");
    // Text that fits has nothing to scroll: its box stays what it was.
    assert!(viewport > 0.0 && viewport < 3.0 * 40.0, "{viewport}");
}

/// A long text is drawn as requests of a few lines each, so that the renderer can set aside the
/// ones that are off screen. Each must land where its first line is in the field's own layout,
/// which is where the caret goes: lines that wrap, blank lines and short ones in between.
#[test]
fn a_long_text_is_drawn_in_chunks_that_sit_where_their_lines_are() {
    let text: String = (0..260)
        .map(|index| match index % 5 {
            0 => format!("A line {index} long enough that it has to wrap across the width of the field more than once, or twice\n"),
            1 => "\n".to_owned(),
            2 => format!("short {index}\n"),
            3 => "\n\n".to_owned(),
            _ => format!("tail {index}\n"),
        })
        .collect();
    assert!(text.len() > 5_000, "long enough to be cut: {}", text.len());
    let mut field = focused_multiline_field(TextFieldController::with_initial(text.clone()), 3);
    field.text_style = field.text_style.text_overflow(TextOverflow::Wrap);
    field.window_threshold = usize::MAX;
    field.max_lines = None;
    let mut ctx = dummy_build_context(400.0, 200.0);
    ctx.box_constraint.max_height = 1_000_000.0;
    ctx.parent_size.height = 1_000_000.0;
    field.update(&ctx);

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 400.0, 200.0)).unwrap();
    let node_context = tree.context(root).unwrap();
    ctx.with_local_v2_paint_context(node_context, |ctx| {
        assert!(field.can_paint(ctx));
        field.paint(ctx);
    });
    let runs: Vec<(f32, String)> = tree
        .draw_list_snapshot(root)
        .unwrap()
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawText { position, text, .. } => Some((position.y, text.to_string())),
            _ => None,
        })
        .collect();

    assert!(runs.len() > 10, "the text is cut into many requests: {}", runs.len());
    assert_eq!(runs.iter().map(|run| run.1.as_str()).collect::<Vec<_>>().join("\n"), text, "and they are all of it");
    let geometry = field.editable_geometry(&ctx, 400.0);
    let caret_y = |offset: usize| geometry.caret_geometry(offset).expect("the caret is laid out").y;
    let mut start = 0;
    for (index, (y, part)) in runs.iter().enumerate() {
        let want = runs[0].0 + caret_y(start) - caret_y(0);
        assert!((y - want).abs() < 0.5, "chunk {index} is drawn at {y}, its first line is at {want}");
        start += part.len() + 1;
    }
}
