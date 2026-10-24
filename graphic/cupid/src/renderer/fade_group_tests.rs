//! A node that opens its list with the fade-mask marker never shows its own
//! commands: it only supplies the alpha its descendants are drawn through.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd_v2::{DrawCommand, FADE_MASK_PIPELINE_NAME, Rect, RenderTree};
use crate::pipeline::material::{
    GradientShape, MATERIAL_PIPELINE_NAME, MaterialGradient, MaterialKind, MaterialRequest,
};
use crate::utilities::Color;

fn marker() -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(FADE_MASK_PIPELINE_NAME),
        data: Arc::from(Vec::new()),
    }
}

/// A horizontal ramp across a `width` by `height` box that is opaque red on the
/// left and fully transparent on the right. Its red must never reach the target.
fn alpha_ramp(width: f32, height: f32) -> DrawCommand {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [0.0, 0.0, width, height]);
    request.opacity = 1.0;
    request.corner_radii = [0.0; 4];
    request.gradient = MaterialGradient {
        shape: GradientShape::Linear,
        geometry: [0.0, 0.0, width, 0.0],
        stop_count: 2,
        ..MaterialGradient::default()
    };
    request.gradient.colors[0] = MaterialGradient::pack_color([255, 0, 0, 255]);
    request.gradient.colors[1] = MaterialGradient::pack_color([255, 0, 0, 0]);
    request.gradient.offsets[1] = 1.0;
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
        data: Arc::from(request.encode_for_test()),
    }
}

fn solid(rect: Rect, color: Color) -> DrawCommand {
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

/// root -> faded (16,16, 96x96, alpha ramp) -> opaque green content (20,20, 56x56).
fn faded_scene() -> (RenderTree, crate::draw_cmd_v2::RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let faded = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let content = tree.add_child(faded, Rect::new(20.0, 20.0, 56.0, 56.0)).unwrap();
    record(&tree, faded, vec![marker(), alpha_ramp(96.0, 96.0)]);
    record(&tree, content, vec![solid(Rect::new(0.0, 0.0, 56.0, 56.0), Color::rgba8(0, 255, 0, 255))]);
    (tree, faded)
}

#[test]
fn the_content_is_scaled_by_the_alpha_and_the_alpha_is_never_drawn() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let (tree, _) = faded_scene();
        let pixels = render(antialiasing, &tree);
        // The ramp runs over x 16..112 of the target, the content over 36..92.
        near(pixel(&pixels, 40, 64), [0, 191, 0, 191], 8, &format!("{antialiasing:?} a quarter in"));
        near(pixel(&pixels, 64, 64), [0, 128, 0, 128], 8, &format!("{antialiasing:?} halfway"));
        near(pixel(&pixels, 88, 64), [0, 64, 0, 64], 8, &format!("{antialiasing:?} three quarters in"));
        // Where the ramp is opaque but nothing was drawn, nothing shows: the
        // mask's own red must not leak.
        assert_eq!(pixel(&pixels, 24, 24), [0; 4], "{antialiasing:?}: alpha without content");
        assert_eq!(pixel(&pixels, 100, 100), [0; 4], "{antialiasing:?}: alpha without content");
        assert_eq!(pixel(&pixels, 2, 2), [0; 4], "{antialiasing:?}: outside the group");
        assert_eq!(pixel(&pixels, 64, 64)[0], 0, "{antialiasing:?}: no red from the mask");
    }
}

#[test]
fn the_group_opacity_applies_once_on_top_of_the_alpha() {
    let (tree, faded) = faded_scene();
    tree.set_opacity(faded, 0.5).unwrap();
    let pixels = render(AntiAlias::Analytic, &tree);
    // Half the halfway alpha: applying the opacity to the mask as well as the
    // content would give a quarter of it.
    near(pixel(&pixels, 64, 64), [0, 64, 0, 64], 8, "opacity 0.5 over alpha 0.5");
}

#[test]
fn a_fade_leaves_its_unfaded_siblings_alone() {
    let (tree, _) = faded_scene();
    let root = tree
        .render_all()
        .iter()
        .find_map(|operation| match operation {
            crate::draw_cmd_v2::RenderOp::Draw(item) => Some(item.element),
            _ => None,
        })
        .expect("the scene draws its root");
    let sibling = tree.add_child(root, Rect::new(0.0, 0.0, 12.0, 12.0)).unwrap();
    record(&tree, sibling, vec![solid(Rect::new(0.0, 0.0, 12.0, 12.0), Color::rgba8(255, 255, 0, 255))]);
    let pixels = render(AntiAlias::Analytic, &tree);
    near(pixel(&pixels, 4, 4), [255, 255, 0, 255], 2, "the sibling still draws");
    near(pixel(&pixels, 64, 64), [0, 128, 0, 128], 8, "and the fade still composes");
}

#[test]
fn a_fade_inside_a_fade_multiplies_the_alphas() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let outer = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let inner = tree.add_child(outer, Rect::new(20.0, 20.0, 56.0, 56.0)).unwrap();
    let content = tree.add_child(inner, Rect::new(0.0, 0.0, 56.0, 56.0)).unwrap();
    record(&tree, outer, vec![marker(), alpha_ramp(96.0, 96.0)]);
    // The inner mask is half opaque everywhere.
    record(
        &tree,
        inner,
        vec![marker(), solid(Rect::new(0.0, 0.0, 56.0, 56.0), Color::rgba8(255, 0, 0, 128))],
    );
    record(&tree, content, vec![solid(Rect::new(0.0, 0.0, 56.0, 56.0), Color::rgba8(0, 255, 0, 255))]);
    let pixels = render(AntiAlias::Analytic, &tree);
    // Half of the outer ramp's halfway alpha.
    near(pixel(&pixels, 64, 64), [0, 64, 0, 64], 8, "0.5 of 0.5");
    assert_eq!(pixel(&pixels, 24, 24), [0; 4], "outside the content");
}

#[test]
fn a_fade_inside_a_coverage_mask_keeps_both_effects() {
    use crate::draw_cmd_v2::COVERAGE_MASK_PIPELINE_NAME;
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    // A blue box shown only through its descendant, which is itself a fade.
    let masked = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let faded = tree.add_child(masked, Rect::new(0.0, 0.0, 96.0, 96.0)).unwrap();
    let content = tree.add_child(faded, Rect::new(4.0, 4.0, 88.0, 88.0)).unwrap();
    record(
        &tree,
        masked,
        vec![
            DrawCommand::DrawCustom {
                pipeline_name: Arc::from(COVERAGE_MASK_PIPELINE_NAME),
                data: Arc::from(Vec::new()),
            },
            solid(Rect::new(0.0, 0.0, 96.0, 96.0), Color::rgba8(0, 0, 255, 255)),
        ],
    );
    record(&tree, faded, vec![marker(), alpha_ramp(96.0, 96.0)]);
    record(&tree, content, vec![solid(Rect::new(0.0, 0.0, 88.0, 88.0), Color::rgba8(0, 255, 0, 255))]);
    let pixels = render(AntiAlias::Analytic, &tree);
    // The coverage is the faded content's alpha: the blue box shows at the
    // ramp's halfway alpha and the content's own green never does.
    near(pixel(&pixels, 64, 64), [0, 0, 128, 128], 8, "blue through the faded coverage");
}
