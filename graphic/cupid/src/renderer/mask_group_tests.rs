//! A node that opens its list with the coverage-mask marker shows its own
//! commands only where its descendants have coverage.

use std::sync::Arc;

use crate::AntiAlias;
use crate::backend::{metal::MetalBackend, *};
use crate::damage_region::DamageSet;
use crate::draw_cmd_v2::{
    COVERAGE_MASK_PIPELINE_NAME, DrawCommand, Rect, RenderFrame, RenderNodeId, RenderTree,
};
use crate::frame::{FramePacket, FrameRenderMetadata};
use crate::pipeline::material::{
    GradientShape, MATERIAL_PIPELINE_NAME, MaterialGradient, MaterialKind, MaterialRequest,
};
use crate::renderer::RendererImpl;
use crate::utilities::Color;

pub(super) const SIZE: u32 = 128;

fn marker() -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(COVERAGE_MASK_PIPELINE_NAME),
        data: Arc::from(Vec::new()),
    }
}

/// A horizontal red-to-blue ramp across a `width` by `height` box.
pub(super) fn ramp(width: f32, height: f32) -> DrawCommand {
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
    request.gradient.colors[1] = MaterialGradient::pack_color([0, 0, 255, 255]);
    request.gradient.offsets[1] = 1.0;
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
        data: Arc::from(request.encode_for_test()),
    }
}

pub(super) fn disc(rect: Rect, color: Color) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color,
        border_radius: [rect.width / 2.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

pub(super) fn record(tree: &RenderTree, node: RenderNodeId, commands: Vec<DrawCommand>) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(commands)
        .unwrap();
}

/// Renders `tree` and returns its RGBA8 pixels.
pub(super) fn render(antialiasing: AntiAlias, tree: &RenderTree) -> Vec<u8> {
    let backend = MetalBackend::new().expect("mask pixel regression requires Metal");
    let format = <MetalBackend as GpuBackend>::rgba8_unorm_format();
    let target = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "mask group pixels");
    let mut renderer = RendererImpl::<MetalBackend>::with_antialiasing(&backend, format, antialiasing);
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(SIZE, SIZE)),
    )
    .expect("lower the masked groups");
    renderer.render_packet(&backend, &view, &packet, false);

    let row = (SIZE * 4).div_ceil(256) * 256;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(SIZE),
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("mask group readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(SIZE),
            },
        },
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);
    let padded = backend.read_buffer(&readback, u64::from(row) * u64::from(SIZE));
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for line in padded.chunks(row as usize).take(SIZE as usize) {
        pixels.extend_from_slice(&line[..(SIZE * 4) as usize]);
    }
    pixels
}

pub(super) fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * SIZE + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().unwrap()
}

pub(super) fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8, what: &str) {
    for (actual_channel, expected_channel) in actual.into_iter().zip(expected) {
        assert!(
            actual_channel.abs_diff(expected_channel) <= tolerance,
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

/// root -> masked (16,16, 96x96) -> coverage disc, drawn in green.
fn masked_scene(coverage: Color) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let masked = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let child = tree.add_child(masked, Rect::new(20.0, 20.0, 56.0, 56.0)).unwrap();
    record(&tree, masked, vec![marker(), ramp(96.0, 96.0)]);
    record(&tree, child, vec![disc(Rect::new(0.0, 0.0, 56.0, 56.0), coverage)]);
    tree
}

#[test]
fn the_content_shows_through_the_coverage_and_nowhere_else() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let pixels = render(antialiasing, &masked_scene(Color::rgba8(0, 255, 0, 255)));
        // The disc covers x/y 36..92 of the target; its center is (64, 64) and the
        // ramp runs across x 16..112, so the center is the ramp's midpoint.
        near(pixel(&pixels, 64, 64), [128, 0, 128, 255], 8, &format!("{antialiasing:?} center"));
        near(pixel(&pixels, 44, 64), [255 - 74, 0, 74, 255], 12, "left of the disc center");
        assert_eq!(pixel(&pixels, 24, 24), [0; 4], "content without coverage is hidden");
        assert_eq!(pixel(&pixels, 100, 100), [0; 4], "content without coverage is hidden");
        assert_eq!(pixel(&pixels, 2, 2), [0; 4], "outside the group");
        assert_eq!(pixel(&pixels, 64, 64)[1], 0, "the coverage's own color is never drawn");
    }
}

#[test]
fn partial_coverage_scales_the_content_alpha() {
    let pixels = render(AntiAlias::Analytic, &masked_scene(Color::rgba8(0, 255, 0, 128)));
    let center = pixel(&pixels, 64, 64);
    assert!(center[3].abs_diff(128) <= 3, "coverage alpha 0.5 halves the result: {center:?}");
    assert!(center[0].abs_diff(64) <= 6 && center[2].abs_diff(64) <= 6, "premultiplied: {center:?}");
}

#[test]
fn a_masked_node_leaves_its_unmasked_siblings_alone() {
    let tree = masked_scene(Color::rgba8(0, 255, 0, 255));
    let root = tree
        .render_all()
        .iter()
        .find_map(|operation| match operation {
            crate::draw_cmd_v2::RenderOp::Draw(item) => Some(item.element),
            _ => None,
        })
        .expect("the scene draws its root");
    // A plain yellow square beside the masked group.
    let sibling = tree.add_child(root, Rect::new(0.0, 0.0, 12.0, 12.0)).unwrap();
    record(
        &tree,
        sibling,
        vec![DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 12.0, 12.0),
            color: Color::rgba8(255, 255, 0, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }],
    );
    let pixels = render(AntiAlias::Analytic, &tree);
    near(pixel(&pixels, 4, 4), [255, 255, 0, 255], 2, "the sibling still draws");
    near(pixel(&pixels, 64, 64), [128, 0, 128, 255], 8, "and the masked group still composes");
}

/// The motivating case: text as the coverage, so glyph shapes cut the content.
#[test]
fn glyph_coverage_cuts_the_content_to_the_shape_of_the_text() {
    use crate::font::{FontFamily, FontStyle};
    use crate::text_pipeline::TextOverflowMode;
    use crate::text_pipeline::text_layout::TextHorizontalAlign;

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let masked = tree.add_child(root, Rect::new(8.0, 8.0, 112.0, 112.0)).unwrap();
    let text = tree.add_child(masked, Rect::new(0.0, 0.0, 112.0, 112.0)).unwrap();
    record(&tree, masked, vec![marker(), ramp(112.0, 112.0)]);
    record(
        &tree,
        text,
        vec![DrawCommand::DrawText {
            position: crate::draw_cmd_v2::Vec2d { x: 4.0, y: 100.0 },
            text: Arc::from("I"),
            font_size: 120.0,
            // Opaque white: only its coverage may matter.
            color: Color::rgba8(255, 255, 255, 255),
            bounds_width: None,
            bounds_height: None,
            overflow: TextOverflowMode::default(),
            horizontal_align: TextHorizontalAlign::default(),
            font_family: FontFamily::default(),
            font_style: FontStyle::default(),
            font_weight: 700,
            shadow: None,
            draw_glyphs: true,
        }],
    );
    let pixels = render(AntiAlias::Analytic, &tree);

    let opaque: Vec<_> = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&pixels, x, y)[3] > 200)
        .collect();
    assert!(
        opaque.len() > 200 && opaque.len() < (SIZE * SIZE / 3) as usize,
        "the glyph covers part of the box, not all of it: {}",
        opaque.len()
    );
    // Every covered pixel shows the ramp (red to blue, no green or white), never
    // the glyph's own white.
    for &(x, y) in &opaque {
        let [r, g, b, _] = pixel(&pixels, x, y);
        assert!(g <= 4, "the glyph's white must not show at ({x}, {y}): {:?}", [r, g, b]);
        assert!(r.abs_diff(255 - b) <= 40, "a point on the ramp at ({x}, {y}): {:?}", [r, g, b]);
    }
    // The ramp runs left to right across the box, so the covered pixels do too.
    let leftmost = opaque.iter().min_by_key(|&&(x, _)| x).copied().unwrap();
    let rightmost = opaque.iter().max_by_key(|&&(x, _)| x).copied().unwrap();
    assert!(
        pixel(&pixels, leftmost.0, leftmost.1)[0] > pixel(&pixels, rightmost.0, rightmost.1)[0],
        "red on the left, blue on the right"
    );
}

/// Glass text: a material that samples the backdrop, masked by coverage.
#[test]
fn a_backdrop_filter_shows_the_filtered_backdrop_only_through_the_coverage() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
        let masked = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
        let coverage = tree.add_child(masked, Rect::new(20.0, 20.0, 56.0, 56.0)).unwrap();
        let split = |color: Color, rect: Rect| DrawCommand::FillRect {
            rect,
            color,
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        };
        // Red on the left half, blue on the right: a backdrop with colour to remove.
        record(
            &tree,
            root,
            vec![
                split(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
                split(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
            ],
        );
        let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 96.0, 96.0]);
        filter.blur_radius = 0.0;
        filter.saturation = 0.0;
        filter.opacity = 1.0;
        filter.corner_radii = [0.0; 4];
        record(
            &tree,
            masked,
            vec![
                marker(),
                DrawCommand::DrawCustom {
                    pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                    data: Arc::from(filter.encode_for_test()),
                },
            ],
        );
        record(&tree, coverage, vec![disc(Rect::new(0.0, 0.0, 56.0, 56.0), Color::rgba8(0, 255, 0, 255))]);

        let pixels = render(antialiasing, &tree);
        let left = pixel(&pixels, 52, 64); // inside the disc, over the red half
        let right = pixel(&pixels, 76, 64); // inside the disc, over the blue half
        assert!(
            left[0].abs_diff(left[1]) <= 3 && left[1].abs_diff(left[2]) <= 3 && left[3] > 250,
            "{antialiasing:?}: desaturated red through the disc: {left:?}"
        );
        assert!(
            right[0].abs_diff(right[1]) <= 3 && right[1].abs_diff(right[2]) <= 3 && right[3] > 250,
            "{antialiasing:?}: desaturated blue through the disc: {right:?}"
        );
        assert_ne!(left, right, "the two halves keep their different luminance");
        near(pixel(&pixels, 24, 64), [255, 0, 0, 255], 2, "outside the disc the backdrop is untouched");
        near(pixel(&pixels, 100, 64), [0, 0, 255, 255], 2, "outside the disc the backdrop is untouched");
    }
}

/// A backdrop filter on a node with a rounded clip blurs only inside that clip:
/// this is what lets a veil follow a growing window instead of filling the host.
#[test]
fn a_backdrop_filter_is_confined_to_its_nodes_rounded_clip() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
        // A full-host filter, as a veil is, shown through a 64 x 48 rounded window
        // at (32, 40).
        let veil = tree.add_child(root, Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
        tree.set_clip_with_radius(veil, Some(Rect::new(32.0, 40.0, 64.0, 48.0)), [16.0; 4]).unwrap();
        let split = |color: Color, rect: Rect| DrawCommand::FillRect {
            rect,
            color,
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        };
        record(
            &tree,
            root,
            vec![
                split(Color::rgba8(255, 0, 0, 255), Rect::new(0.0, 0.0, 64.0, SIZE as f32)),
                split(Color::rgba8(0, 0, 255, 255), Rect::new(64.0, 0.0, 64.0, SIZE as f32)),
            ],
        );
        let mut filter = MaterialRequest::new(
            MaterialKind::BackdropFilter,
            [0.0, 0.0, SIZE as f32, SIZE as f32],
        );
        filter.blur_radius = 0.0;
        filter.saturation = 0.0;
        filter.opacity = 1.0;
        filter.corner_radii = [0.0; 4];
        record(
            &tree,
            veil,
            vec![DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(filter.encode_for_test()),
            }],
        );

        let pixels = render(antialiasing, &tree);
        let grey = |[r, g, b, a]: [u8; 4]| r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3 && a > 250;
        assert!(grey(pixel(&pixels, 48, 64)), "{antialiasing:?}: inside the window, over red");
        assert!(grey(pixel(&pixels, 80, 64)), "{antialiasing:?}: inside the window, over blue");
        near(pixel(&pixels, 10, 64), [255, 0, 0, 255], 2, "left of the window is untouched");
        near(pixel(&pixels, 120, 64), [0, 0, 255, 255], 2, "right of the window is untouched");
        near(pixel(&pixels, 48, 20), [255, 0, 0, 255], 2, "above the window is untouched");
        near(pixel(&pixels, 80, 110), [0, 0, 255, 255], 2, "below the window is untouched");
        // The window's corner is rounded by 16: the very corner pixel is outside it.
        near(pixel(&pixels, 33, 41), [255, 0, 0, 255], 4, "the rounded corner is untouched");
    }
}
