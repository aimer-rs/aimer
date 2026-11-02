//! Corner radii on a retained clip must survive lowering to the renderer's
//! `PushClip`.

use crate::draw_cmd::DrawCommand as LegacyDrawCommand;
use crate::draw_cmd_v2::{
    DrawCommand, Rect, RenderNodeId, RenderNodeSpec, RenderOp, RenderPaintSource, RenderTree,
};
use crate::frame::RetainedV2Item;
use crate::utilities::Color;

/// Top-left, top-right, bottom-right, bottom-left, in logical pixels.
const RADIUS: [f32; 4] = [4.0, 5.0, 6.0, 7.0];
const SCALE: f32 = 2.0;

fn clipped_root(source: RenderPaintSource, radius: [f32; 4]) -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
    tree.sync_structure_with_clip_radii(
        &[RenderNodeSpec::new(
            Some(root),
            None,
            Rect::new(0.0, 0.0, 64.0, 64.0),
        )],
        &[Some(Rect::new(0.0, 0.0, 40.0, 40.0))],
        &[radius],
    )
    .unwrap();
    tree.set_paint_source(root, source).unwrap();
    (tree, root)
}

fn lowered_local_v2(radius: [f32; 4]) -> Vec<LegacyDrawCommand> {
    let (tree, root) = clipped_root(RenderPaintSource::LocalV2, radius);
    tree.context(root)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color: Color::rgba8(20, 40, 60, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }])
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
        transform: item.transform,
        clip: item.clip,
        clip_radius: item.clip_radius,
        commands: snapshot.commands,
    };
    super::lower_retained_v2_commands(&retained, SCALE).unwrap()
}

fn push_clip_radii(commands: &[LegacyDrawCommand]) -> Vec<[f32; 4]> {
    commands
        .iter()
        .filter_map(|command| match command {
            LegacyDrawCommand::PushClip { border_radius, .. } => Some(*border_radius),
            _ => None,
        })
        .collect()
}

#[test]
fn a_local_v2_clip_carries_the_corner_radius() {
    // The clip is pushed under the logical-to-device scale transform, so the
    // renderer scales the radius with the rectangle.
    assert_eq!(push_clip_radii(&lowered_local_v2(RADIUS)), vec![RADIUS]);
}

#[test]
fn a_square_clip_still_lowers_with_zero_radii() {
    assert_eq!(push_clip_radii(&lowered_local_v2([0.0; 4])), vec![[0.0; 4]]);
}
