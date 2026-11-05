//! A node that opens its list with the fade-mask marker never shows its own
//! commands: it only supplies the alpha its descendants are drawn through.

use std::sync::Arc;

use super::mask_group_tests::{SIZE, near, pixel, record, render};
use crate::AntiAlias;
use crate::draw_cmd::{DrawCommand, FADE_MASK_PIPELINE_NAME, Rect, RenderTree};
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
fn faded_scene() -> (RenderTree, crate::draw_cmd::RenderNodeId) {
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
            crate::draw_cmd::RenderOp::Draw(item) => Some(item.element),
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
    use crate::draw_cmd::COVERAGE_MASK_PIPELINE_NAME;
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

/// A cleared target costs a command buffer if the clear is a pass of its own, and
/// a masked node has two targets. The root's clear, the alpha, the content, the
/// scaling composite and the group's composite are all the passes it needs.
#[test]
fn a_fade_group_clears_its_targets_in_the_passes_that_draw_into_them() {
    use super::mask_group_tests::render_counting;
    let (tree, _) = faded_scene();
    let (_, submitted) = render_counting(AntiAlias::Analytic, &tree);
    assert!(
        submitted <= 5,
        "a fade group submitted {submitted} command buffers; its two clears should not be among them"
    );
}

// ── A fade over text, drawn as a ramp on its glyphs ───────────────────────

use super::mask_group_tests::render_with;
use crate::draw_cmd::Vec2d;
use crate::font::{FontFamily, FontStyle};
use crate::pipeline::text_pipeline::TextOverflowMode;
use crate::text_layout::TextHorizontalAlign;

fn title(text: &str) -> DrawCommand {
    DrawCommand::DrawText {
        position: Vec2d { x: 2.0, y: 4.0 },
        text: Arc::from(text),
        font_size: 20.0,
        color: Color::rgba8(0, 255, 0, 255),
        bounds_width: None,
        bounds_height: None,
        overflow: TextOverflowMode::default(),
        horizontal_align: TextHorizontalAlign::default(),
        font_family: FontFamily::default(),
        font_style: FontStyle::default(),
        font_weight: 600,
        shadow: None,
        draw_glyphs: true,
    }
}

/// root -> faded (8,40, 112x32, alpha ramp) -> text, as a faded thread title is.
fn faded_title(text: &str) -> (RenderTree, crate::draw_cmd::RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let faded = tree.add_child(root, Rect::new(8.0, 40.0, 112.0, 32.0)).unwrap();
    let content = tree.add_child(faded, Rect::new(0.0, 0.0, 112.0, 32.0)).unwrap();
    record(&tree, faded, vec![marker(), alpha_ramp(112.0, 32.0)]);
    record(&tree, content, vec![title(text)]);
    (tree, faded)
}

/// The largest difference in any channel between two renderings, and how many
/// pixels differ by more than `tolerance`.
fn differences(left: &[u8], right: &[u8], tolerance: u8) -> (u8, usize) {
    let mut largest = 0;
    let mut beyond = 0;
    for (a, b) in left.iter().zip(right) {
        let difference = a.abs_diff(*b);
        largest = largest.max(difference);
        if difference > tolerance {
            beyond += 1;
        }
    }
    (largest, beyond)
}

#[test]
fn a_title_faded_as_a_ramp_looks_as_it_does_faded_in_a_group() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let (tree, _) = faded_title("A thread title that fades");
        let (ramped, _) = render_with(antialiasing, &tree, true);
        let (grouped, _) = render_with(antialiasing, &tree, false);

        let visible = ramped.chunks(4).filter(|pixel| pixel[3] > 8).count();
        assert!(visible > 100, "{antialiasing:?}: the title is drawn at all ({visible} pixels)");
        let (largest, beyond) = differences(&ramped, &grouped, 2);
        assert!(
            beyond == 0,
            "{antialiasing:?}: {beyond} channels differ by more than 2 (largest {largest})"
        );
    }
}

#[test]
fn a_ramped_fade_costs_no_pass_of_its_own() {
    let (tree, _) = faded_title("A thread title that fades");
    let (_, ramped) = render_with(AntiAlias::Analytic, &tree, true);
    let (_, grouped) = render_with(AntiAlias::Analytic, &tree, false);
    // The root's clear and the one pass that draws the text.
    assert!(ramped <= 2, "a ramped fade submitted {ramped} command buffers");
    assert!(grouped > ramped, "as a group it submitted {grouped}");
}

#[test]
fn the_group_opacity_applies_once_to_a_ramped_title() {
    let (tree, faded) = faded_title("A thread title that fades");
    tree.set_opacity(faded, 0.5).unwrap();
    let (ramped, _) = render_with(AntiAlias::Analytic, &tree, true);
    let (grouped, _) = render_with(AntiAlias::Analytic, &tree, false);
    let (largest, beyond) = differences(&ramped, &grouped, 2);
    assert!(beyond == 0, "{beyond} channels differ by more than 2 (largest {largest})");
}

#[test]
fn text_that_could_be_an_emoji_is_still_faded_in_a_group() {
    let (tree, _) = faded_title("A title with a rocket 🚀");
    let (_, submitted) = render_with(AntiAlias::Analytic, &tree, true);
    let (_, grouped) = render_with(AntiAlias::Analytic, &tree, false);
    assert_eq!(submitted, grouped, "it takes the group path, which draws any content");
}

#[test]
fn a_fade_over_something_other_than_text_is_still_faded_in_a_group() {
    let (tree, _) = faded_scene();
    let (pixels, submitted) = render_with(AntiAlias::Analytic, &tree, true);
    near(pixel(&pixels, 64, 64), [0, 128, 0, 128], 8, "the fade still composes");
    assert!(submitted >= 5, "a fade over a rectangle submitted {submitted}");
}

/// root (clipped to a viewport) -> faded -> text: a thread title in a scroll
/// view, whose items are all framed by the viewport's clip.
fn faded_title_in_a_viewport(text: &str) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    tree.set_clip(root, Some(Rect::new(0.0, 36.0, SIZE as f32, 44.0))).unwrap();
    let faded = tree.add_child(root, Rect::new(8.0, 40.0, 112.0, 32.0)).unwrap();
    let content = tree.add_child(faded, Rect::new(0.0, 0.0, 112.0, 32.0)).unwrap();
    record(&tree, faded, vec![marker(), alpha_ramp(112.0, 32.0)]);
    record(&tree, content, vec![title(text)]);
    tree
}

#[test]
fn a_title_in_a_clipped_viewport_is_faded_as_a_ramp_and_looks_the_same() {
    let tree = faded_title_in_a_viewport("A thread title that fades");
    let (ramped, ramped_submitted) = render_with(AntiAlias::Analytic, &tree, true);
    let (grouped, grouped_submitted) = render_with(AntiAlias::Analytic, &tree, false);

    let visible = ramped.chunks(4).filter(|pixel| pixel[3] > 8).count();
    assert!(visible > 100, "the title is drawn at all ({visible} pixels)");
    let (largest, beyond) = differences(&ramped, &grouped, 2);
    assert!(beyond == 0, "{beyond} channels differ by more than 2 (largest {largest})");
    assert!(
        ramped_submitted < grouped_submitted && ramped_submitted <= 2,
        "the clip kept it from being a ramp: {ramped_submitted} command buffers against {grouped_submitted}"
    );
}
