//! A confined backdrop blur reads only inside its own clip.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
use crate::utilities::Color;

fn fill(color: Color, rect: Rect) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color,
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

/// root (white left of x = 64, black right of it) -> a 96 x 96 node at (16, 16)
/// that blurs what is behind it and is clipped to a window whose right edge is the
/// edge between the two, x = 64.
fn windowed_blur(confined: bool) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    record(
        &tree,
        root,
        vec![
            fill(Color::rgba8(255, 255, 255, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            fill(Color::rgba8(0, 0, 0, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
    filter.blur_radius = 12.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    filter.confine_backdrop = confined;
    record(
        &tree,
        node,
        vec![DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
            data: Arc::from(filter.encode_for_test()),
        }],
    );
    tree.set_clip(node, Some(Rect::new(0.0, 0.0, 48.0, 96.0))).unwrap();
    tree
}

/// The blur of a window reads what lies outside it by default, so the black beyond
/// the window's right edge darkens the white inside it; confined, the window's own
/// edge is where the picture ends and the white stays white.
#[test]
fn a_confined_blur_does_not_read_what_lies_outside_its_clip() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let open = render(antialiasing, &windowed_blur(false));
        assert!(
            pixel(&open, 62, 64)[0] < 245,
            "{antialiasing:?}: an open blur picks up the black just outside: {:?}",
            pixel(&open, 62, 64)
        );

        let confined = render(antialiasing, &windowed_blur(true));
        near(pixel(&confined, 62, 64), [255, 255, 255, 255], 3, &format!("{antialiasing:?} at the window's edge"));
        near(pixel(&confined, 30, 64), [255, 255, 255, 255], 3, &format!("{antialiasing:?} inside the window"));
        // Outside the window nothing is drawn: the backdrop is as it was.
        near(pixel(&confined, 70, 64), [0, 0, 0, 255], 2, &format!("{antialiasing:?} outside the window"));
        near(pixel(&confined, 8, 64), [255, 255, 255, 255], 2, &format!("{antialiasing:?} left of the window"));
    }
}

/// The blur still blurs inside the window: a sharp edge that lies within it softens.
#[test]
fn a_confined_blur_still_softens_what_is_inside_its_clip() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let node = tree.add_child(root, Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    record(
        &tree,
        root,
        vec![
            fill(Color::rgba8(255, 255, 255, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
            fill(Color::rgba8(0, 0, 0, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
        ],
    );
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, SIZE as f32, SIZE as f32]);
    filter.blur_radius = 12.0;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    filter.confine_backdrop = true;
    record(
        &tree,
        node,
        vec![DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
            data: Arc::from(filter.encode_for_test()),
        }],
    );
    let edge = pixel(&render(AntiAlias::Analytic, &tree), 64, 64);
    assert!(edge[0] > 60 && edge[0] < 195, "the edge inside the window is blurred: {edge:?}");
}
