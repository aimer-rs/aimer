//! A confined backdrop blur reads only inside its own clip.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd::{DrawCommand, Rect, RenderTree};
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

/// root (a dark feed) -> [backing: an opaque green fill the size of the host, veil: a
/// confined blur the size of the host], both moved down by 40 and clipped to a window
/// at the top-left of what they were moved from, as a transition's host presents them.
fn presented_backing_and_veil(blur: f32) -> RenderTree {
    use crate::utilities::Mat3;
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let backing = tree.add_child(root, Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let veil = tree.add_child(root, Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    record(&tree, root, vec![fill(Color::rgba8(20, 10, 10, 255), Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))]);
    record(&tree, backing, vec![fill(Color::rgba8(10, 200, 10, 255), Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32))]);
    let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, SIZE as f32, SIZE as f32]);
    filter.blur_radius = blur;
    filter.opacity = 1.0;
    filter.corner_radii = [0.0; 4];
    filter.confine_backdrop = true;
    record(&tree, veil, vec![DrawCommand::DrawCustom {
        pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
        data: Arc::from(filter.encode_for_test()),
    }]);
    let host = tree.context(root).unwrap();
    for index in 0..2 {
        assert!(host.set_child_presentation_at(index, Mat3::translate(0.0, 40.0), 1.0));
        assert!(host.set_child_clip_at(index, Some(Rect::new(0.0, 0.0, 60.0, 50.0)), [6.0; 4]));
    }
    tree
}

#[test]
fn nothing_is_drawn_outside_the_window_whatever_the_blur() {
    for blur in [0.0, 0.001, 0.5, 2.0, 12.0] {
        let pixels = render(AntiAlias::Analytic, &presented_backing_and_veil(blur));
        // Inside the window: the backing, blurred or not.
        let inside = pixel(&pixels, 30, 60);
        assert!(inside[1] > 150 && inside[0] < 60, "blur {blur}: the window shows the backing: {inside:?}");
        // Outside the window, where the host-sized backing would reach if unclipped.
        for (x, y) in [(100, 100), (100, 60), (30, 120)] {
            near(pixel(&pixels, x, y), [20, 10, 10, 255], 2, &format!("blur {blur} outside the window at ({x}, {y})"));
        }
    }
}
