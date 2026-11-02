//! Text under a scaled node transform must be shaped at the scaled size: the
//! renderer only maps a text request's position through the transform, so the
//! glyph size and layout bounds have to arrive already scaled.

use std::sync::Arc;

use crate::draw_cmd::DrawCommand as LegacyDrawCommand;
use crate::draw_cmd_v2::{DrawCommand, Rect, RenderOp, RenderPaintSource, RenderTree};
use crate::frame::RetainedV2Item;
use crate::utilities::{Color, Mat3, Vec2d};

const DEVICE_SCALE: f32 = 2.0;

fn text_command() -> DrawCommand {
    DrawCommand::DrawText {
        position: Vec2d::new(0.0, 8.0),
        text: Arc::<str>::from("Back"),
        font_size: 8.0,
        color: Color::white(),
        bounds_width: Some(10.0),
        bounds_height: Some(20.0),
        overflow: crate::text_pipeline::TextOverflowMode::Clip,
        horizontal_align: crate::text_pipeline::text_layout::TextHorizontalAlign::Left,
        font_family: crate::font::FontFamily::SANS_SERIF,
        font_style: crate::font::FontStyle::Normal,
        font_weight: 400,
        shadow: None,
        draw_glyphs: true,
    }
}

/// Lowers one text node under `transform` and returns its font size and
/// layout bounds as the renderer receives them.
fn lowered_text(transform: Mat3) -> (f32, Option<f32>, Option<f32>) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    tree.context(root)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![text_command()])
        .unwrap();
    let item = tree
        .render_all()
        .into_iter()
        .find_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item),
            _ => None,
        })
        .expect("retained root item");
    let snapshot = item.snapshot();
    let retained = RetainedV2Item {
        element: item.element,
        revision: snapshot.revision,
        bounds: item.bounds,
        origin: item.origin,
        // Set on the item itself: the tree culls a node whose scale is zero.
        transform,
        clip: item.clip,
        clip_radius: item.clip_radius,
        commands: snapshot.commands,
    };
    super::lower_retained_v2_commands(&retained, DEVICE_SCALE)
        .unwrap()
        .into_iter()
        .find_map(|command| match command {
            LegacyDrawCommand::DrawText {
                font_size,
                bounds_width,
                bounds_height,
                ..
            } => Some((font_size, bounds_width, bounds_height)),
            _ => None,
        })
        .expect("lowered text command")
}

#[test]
fn unscaled_nodes_keep_the_device_scaled_text_size() {
    assert_eq!(
        lowered_text(Mat3::identity()),
        (16.0, Some(20.0), Some(40.0))
    );
    assert_eq!(
        lowered_text(Mat3::translate(5.0, 7.0)),
        (16.0, Some(20.0), Some(40.0))
    );
}

#[test]
fn a_scaled_node_scales_its_text_and_text_bounds() {
    assert_eq!(
        lowered_text(Mat3::scale(1.5, 1.5)),
        (24.0, Some(30.0), Some(60.0))
    );
    // Scaling about a pivot is a scale plus a translation.
    let about_center = Mat3::translate(32.0, 32.0)
        .mul(&Mat3::scale(0.5, 0.5))
        .mul(&Mat3::translate(-32.0, -32.0));
    assert_eq!(lowered_text(about_center), (8.0, Some(10.0), Some(20.0)));
}

#[test]
fn a_degenerate_node_scale_does_not_collapse_the_text() {
    for transform in [Mat3::scale(0.0, 0.0), Mat3::scale(-1.0, 1.0)] {
        let (font_size, ..) = lowered_text(transform);
        assert!(font_size.is_finite() && font_size > 0.0, "{font_size}");
    }
}
