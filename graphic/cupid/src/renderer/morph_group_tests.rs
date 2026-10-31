//! A node that opens its list with the morph marker blends its two children
//! with a shader instead of painting them on top of each other.

use std::sync::Arc;

use crate::AntiAlias;
use crate::backend::{metal::MetalBackend, *};
use crate::damage_region::DamageSet;
use crate::draw_cmd_v2::{
    DrawCommand, MORPH_PIPELINE_NAME, MorphParams, MorphStyle, Rect, RenderFrame, RenderNodeId,
    RenderTree,
};
use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
use crate::renderer::RendererImpl;
use crate::utilities::Color;

const SIZE: u32 = 128;
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn marker(params: MorphParams) -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(MORPH_PIPELINE_NAME),
        data: Arc::from(params.encode().to_vec()),
    }
}

fn fill(rect: Rect, color: [u8; 4]) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color: Color::rgba8(color[0], color[1], color[2], color[3]),
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

fn record(tree: &RenderTree, node: RenderNodeId, commands: Vec<DrawCommand>) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(commands)
        .unwrap();
}

/// Renders `tree` and returns its RGBA8 pixels.
fn render(antialiasing: AntiAlias, tree: &RenderTree) -> Vec<u8> {
    let backend = MetalBackend::new().expect("morph pixel regression requires Metal");
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
    let view = backend.create_texture_view(&target, "morph group pixels");
    let mut renderer = RendererImpl::<MetalBackend>::with_antialiasing(&backend, format, antialiasing);
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(SIZE, SIZE)),
    )
    .expect("lower the morph groups");
    renderer.render_packet(&backend, &view, &packet, false);

    let row = (SIZE * 4).div_ceil(256) * 256;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(SIZE),
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("morph group readback");
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

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * SIZE + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().unwrap()
}

fn near(actual: [u8; 4], expected: [u8; 4], tolerance: u8, what: &str) {
    for (actual_channel, expected_channel) in actual.into_iter().zip(expected) {
        assert!(
            actual_channel.abs_diff(expected_channel) <= tolerance,
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

/// Adds a morph host to `parent` whose children paint `from` and `to` over the
/// whole host, and returns it with those children.
fn add_morph(
    tree: &RenderTree,
    parent: RenderNodeId,
    bounds: Rect,
    params: MorphParams,
    from: [u8; 4],
    to: [u8; 4],
) -> (RenderNodeId, RenderNodeId, RenderNodeId) {
    let host = tree.add_child(parent, bounds).unwrap();
    let local = Rect::new(0.0, 0.0, bounds.width, bounds.height);
    let from_node = tree.add_child(host, local).unwrap();
    let to_node = tree.add_child(host, local).unwrap();
    record(tree, host, vec![marker(params)]);
    record(tree, from_node, vec![fill(local, from)]);
    record(tree, to_node, vec![fill(local, to)]);
    (host, from_node, to_node)
}

/// A root with one 96 by 96 morph host at (16, 16).
fn scene(params: MorphParams) -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let (host, _, _) = add_morph(&tree, root, Rect::new(16.0, 16.0, 96.0, 96.0), params, RED, BLUE);
    (tree, host)
}

const STYLES: [MorphStyle; 7] = [
    MorphStyle::Dissolve,
    MorphStyle::Wipe,
    MorphStyle::Liquid,
    MorphStyle::Bounds,
    MorphStyle::Blur,
    MorphStyle::Slide,
    MorphStyle::Goo,
];

#[test]
fn every_style_starts_on_the_outgoing_picture_and_ends_on_the_incoming_one() {
    for antialiasing in [AntiAlias::Analytic, AntiAlias::Msaa4x] {
        for style in STYLES {
            let start = render(antialiasing, &scene(MorphParams::new(style, 0.0)).0);
            let end = render(antialiasing, &scene(MorphParams::new(style, 1.0)).0);
            for (x, y) in [(24, 24), (64, 64), (100, 100), (30, 90)] {
                near(pixel(&start, x, y), RED, 3, &format!("{style:?} {antialiasing:?} start ({x}, {y})"));
                near(pixel(&end, x, y), BLUE, 3, &format!("{style:?} {antialiasing:?} end ({x}, {y})"));
            }
            assert_eq!(pixel(&start, 4, 4), [0; 4], "{style:?}: outside the morph stays empty");
            assert_eq!(pixel(&end, 120, 120), [0; 4], "{style:?}: outside the morph stays empty");
        }
    }
}

#[test]
fn a_wipe_reveals_the_incoming_picture_behind_a_soft_edge_travelling_along_its_direction() {
    let mut params = MorphParams::new(MorphStyle::Wipe, 0.5);
    params.direction = [1.0, 0.0];
    params.softness = 0.25;
    let pixels = render(AntiAlias::Analytic, &scene(params).0);

    // The edge sits at x = 16 + 96 * 0.5 = 64: blue behind it, red ahead of it.
    near(pixel(&pixels, 24, 64), BLUE, 4, "behind the edge");
    near(pixel(&pixels, 100, 64), RED, 4, "ahead of the edge");
    near(pixel(&pixels, 64, 64), [128, 0, 128, 255], 10, "the middle of the edge");
    near(pixel(&pixels, 64, 24), [128, 0, 128, 255], 10, "the edge is straight");

    params.direction = [0.0, 1.0];
    let down = render(AntiAlias::Analytic, &scene(params).0);
    near(pixel(&down, 64, 24), BLUE, 4, "a downward wipe starts at the top");
    near(pixel(&down, 64, 100), RED, 4, "a downward wipe ends at the bottom");
}

#[test]
fn a_dissolve_hands_pixels_over_gradually() {
    let blue_pixels = |progress: f32| {
        let pixels = render(AntiAlias::Analytic, &scene(MorphParams::new(MorphStyle::Dissolve, progress)).0);
        let mut blue = 0;
        let mut red = 0;
        for y in 20..108 {
            for x in 20..108 {
                let [r, _, b, _] = pixel(&pixels, x, y);
                if b > 200 && r < 55 {
                    blue += 1;
                } else if r > 200 && b < 55 {
                    red += 1;
                }
            }
        }
        (blue, red)
    };
    let (blue_early, red_early) = blue_pixels(0.25);
    let (blue_mid, red_mid) = blue_pixels(0.5);
    let (blue_late, red_late) = blue_pixels(0.75);

    assert!(blue_early < blue_mid && blue_mid < blue_late, "{blue_early} {blue_mid} {blue_late}");
    assert!(red_early > red_mid && red_mid > red_late, "{red_early} {red_mid} {red_late}");
    assert!(blue_mid > 100 && red_mid > 100, "mid-way shows both: {blue_mid} {red_mid}");
}

#[test]
fn a_bounds_morph_stretches_both_pictures_into_the_travelling_box() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let host = tree.add_child(root, Rect::new(8.0, 8.0, 112.0, 112.0)).unwrap();
    let from = tree.add_child(host, Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let to = tree.add_child(host, Rect::new(60.0, 60.0, 50.0, 50.0)).unwrap();
    record(&tree, from, vec![fill(Rect::new(0.0, 0.0, 40.0, 40.0), RED)]);
    record(&tree, to, vec![fill(Rect::new(0.0, 0.0, 50.0, 50.0), BLUE)]);
    let render_at = |progress: f32| {
        record(&tree, host, vec![marker(MorphParams::new(MorphStyle::Bounds, progress))]);
        render(AntiAlias::Analytic, &tree)
    };

    let start = render_at(0.0);
    near(pixel(&start, 20, 20), RED, 3, "t=0 shows the outgoing box");
    assert_eq!(pixel(&start, 90, 90), [0; 4], "t=0 has not reached the incoming box");

    // The box is now (8, 8, 40, 40) -> (68, 68, 50, 50) at one half: (38, 38, 45, 45).
    let middle = render_at(0.5);
    near(pixel(&middle, 60, 60), [128, 0, 128, 255], 10, "both pictures cross-fade inside the box");
    assert_eq!(pixel(&middle, 12, 12), [0; 4], "the outgoing box has left its start");
    assert_eq!(pixel(&middle, 100, 100), [0; 4], "the incoming box has not arrived");

    let end = render_at(1.0);
    near(pixel(&end, 90, 90), BLUE, 3, "t=1 shows the incoming box");
    assert_eq!(pixel(&end, 12, 12), [0; 4], "t=1 has left the outgoing box");
}

#[test]
fn the_morph_follows_its_own_opacity() {
    let (tree, host) = scene(MorphParams::new(MorphStyle::Wipe, 0.0));
    tree.set_opacity(host, 0.5).unwrap();

    let pixels = render(AntiAlias::Analytic, &tree);

    let center = pixel(&pixels, 64, 64);
    assert!(center[3].abs_diff(128) <= 3, "half opacity: {center:?}");
    assert!(center[0].abs_diff(128) <= 3 && center[2] <= 3, "premultiplied red: {center:?}");
}

#[test]
fn separate_morphs_do_not_share_their_sources() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    add_morph(&tree, root, Rect::new(0.0, 0.0, 64.0, 128.0), MorphParams::new(MorphStyle::Wipe, 0.0), RED, GREEN);
    add_morph(&tree, root, Rect::new(64.0, 0.0, 64.0, 128.0), MorphParams::new(MorphStyle::Wipe, 1.0), RED, BLUE);

    let pixels = render(AntiAlias::Analytic, &tree);

    near(pixel(&pixels, 32, 64), RED, 3, "the first morph is still outgoing");
    near(pixel(&pixels, 96, 64), BLUE, 3, "the second morph is incoming");
}

#[test]
fn a_morph_can_be_a_source_of_another_morph() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let outer = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let local = Rect::new(0.0, 0.0, 96.0, 96.0);
    // The outer morph's outgoing picture is itself a finished morph from red to green.
    let (inner, _, _) = add_morph(
        &tree,
        outer,
        local,
        MorphParams::new(MorphStyle::Dissolve, 1.0),
        RED,
        GREEN,
    );
    let incoming = tree.add_child(outer, local).unwrap();
    record(&tree, incoming, vec![fill(local, BLUE)]);
    let _ = inner;

    record(&tree, outer, vec![marker(MorphParams::new(MorphStyle::Wipe, 0.0))]);
    let outgoing = render(AntiAlias::Analytic, &tree);
    near(pixel(&outgoing, 64, 64), GREEN, 3, "the nested morph shows its own result");

    record(&tree, outer, vec![marker(MorphParams::new(MorphStyle::Wipe, 1.0))]);
    let incoming = render(AntiAlias::Analytic, &tree);
    near(pixel(&incoming, 64, 64), BLUE, 3, "then the outer morph hands over");
}

#[test]
fn a_slide_pushes_the_outgoing_picture_out_as_the_incoming_one_enters_behind_it() {
    let slide = |direction: [f32; 2], progress: f32| {
        let mut params = MorphParams::new(MorphStyle::Slide, progress);
        params.direction = direction;
        render(AntiAlias::Analytic, &scene(params).0)
    };

    // Rightward: the old picture leaves to the right, the new one comes in from
    // the left. Half way, each covers half of the 96 px host (x 16..112).
    let half = slide([1.0, 0.0], 0.5);
    near(pixel(&half, 24, 64), BLUE, 3, "the incoming picture fills the left half");
    near(pixel(&half, 40, 24), BLUE, 3, "...at every height");
    near(pixel(&half, 100, 64), RED, 3, "the outgoing picture is pushed to the right half");
    assert_eq!(pixel(&half, 4, 64), [0; 4], "nothing leaks outside the morph");

    let quarter = slide([1.0, 0.0], 0.25);
    near(pixel(&quarter, 30, 64), BLUE, 3, "a quarter of the incoming picture is in");
    near(pixel(&quarter, 60, 64), RED, 3, "the rest is still the outgoing one");
    near(pixel(&quarter, 100, 64), RED, 3, "right up to the far edge it has not left yet");

    // The other way round.
    let leftward = slide([-1.0, 0.0], 0.5);
    near(pixel(&leftward, 100, 64), BLUE, 3, "now the incoming one enters from the right");
    near(pixel(&leftward, 24, 64), RED, 3, "and the outgoing one leaves to the left");

    // Vertical: downward pushes the old picture down, the new one drops in from above.
    let downward = slide([0.0, 1.0], 0.5);
    near(pixel(&downward, 64, 24), BLUE, 3, "the incoming picture is in the top half");
    near(pixel(&downward, 64, 100), RED, 3, "the outgoing one is in the bottom half");
}

#[test]
fn a_slide_keeps_both_pictures_opaque_and_sharp_while_they_move() {
    let mut params = MorphParams::new(MorphStyle::Slide, 0.5);
    params.direction = [1.0, 0.0];
    let pixels = render(AntiAlias::Analytic, &scene(params).0);

    // No cross-fade: apart from the seam, every pixel is one picture's own color.
    for x in (20..110).filter(|x| (*x as i32 - 64).abs() > 2) {
        let [r, g, b, a] = pixel(&pixels, x, 64);
        assert_eq!(a, 255, "x {x}");
        assert!(g == 0 && (r == 255 || b == 255), "x {x}: {:?}", [r, g, b, a]);
    }
}

#[test]
fn a_diagonal_slide_travels_the_whole_extent_along_its_direction() {
    let mut params = MorphParams::new(MorphStyle::Slide, 1.0);
    params.direction = [1.0, 1.0];
    let done = render(AntiAlias::Analytic, &scene(params).0);
    near(pixel(&done, 30, 30), BLUE, 3, "finished: the incoming picture is home");
    near(pixel(&done, 100, 100), BLUE, 3, "finished: the incoming picture is home");

    params.progress = 0.0;
    let start = render(AntiAlias::Analytic, &scene(params).0);
    near(pixel(&start, 30, 30), RED, 3, "not started: the outgoing picture is home");
}

fn rounded(rect: Rect, color: [u8; 4]) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color: Color::rgba8(color[0], color[1], color[2], color[3]),
        border_radius: [rect.height / 2.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

/// One wide pill turning into a shorter pill and a separate circle, the way a
/// toolbar's buttons split. Both pictures fill the same 96 x 32 host, so only
/// their silhouettes differ: the pill spans x 0..96, then the pill 0..44 and
/// the circle 64..96 with a 20 px gap between them.
fn division_scene(params: MorphParams) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let host = tree.add_child(root, Rect::new(16.0, 48.0, 96.0, 32.0)).unwrap();
    let local = Rect::new(0.0, 0.0, 96.0, 32.0);
    let from = tree.add_child(host, local).unwrap();
    let to = tree.add_child(host, local).unwrap();
    record(&tree, from, vec![rounded(local, RED)]);
    record(
        &tree,
        to,
        vec![
            rounded(Rect::new(0.0, 0.0, 44.0, 32.0), BLUE),
            rounded(Rect::new(64.0, 0.0, 32.0, 32.0), BLUE),
        ],
    );
    record(&tree, host, vec![marker(params)]);
    tree
}

#[test]
fn a_goo_morph_pinches_one_shape_into_two_through_a_neck() {
    let goo = |progress: f32| {
        let mut params = MorphParams::new(MorphStyle::Goo, progress);
        params.blur = 12.0;
        render(AntiAlias::Analytic, &division_scene(params))
    };
    // The middle of the gap is x 16 + 54 = 70, the neck's centre line y 64.
    let gap_centre = (70, 64);
    let gap_edge = (70, 50);

    let start = goo(0.0);
    near(pixel(&start, gap_centre.0, gap_centre.1), RED, 3, "one pill: the gap is filled");

    let end = goo(1.0);
    assert_eq!(pixel(&end, gap_centre.0, gap_centre.1), [0; 4], "two shapes: the gap is open");
    near(pixel(&end, 30, 64), BLUE, 3, "the pill is there");
    near(pixel(&end, 100, 64), BLUE, 3, "and so is the circle");

    let middle = goo(0.5);
    let neck = pixel(&middle, gap_centre.0, gap_centre.1);
    // A plain cross-fade would leave the gap half transparent here (alpha 128);
    // a goo threshold makes it solid, which is what joins the shapes.
    assert!(neck[3] > 235, "the neck joins the two shapes solidly: {neck:?}");
    assert!(
        pixel(&middle, gap_edge.0, gap_edge.1)[3] < 60,
        "the neck is narrower than the shapes it joins: {:?}",
        pixel(&middle, gap_edge.0, gap_edge.1)
    );

    // Later the neck thins and goes: solid at the centre line, then gone.
    let late = goo(0.6);
    assert!(
        pixel(&late, gap_centre.0, gap_centre.1)[3] < pixel(&middle, gap_centre.0, gap_centre.1)[3],
        "the bridge weakens as the shapes part"
    );
}

#[test]
fn a_goo_morph_has_no_halo_around_shapes_that_do_not_change() {
    // The same pill in both pictures: whatever the middle of the morph does, the
    // pill must not grow a halo or lose its edge.
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let host = tree.add_child(root, Rect::new(16.0, 48.0, 96.0, 32.0)).unwrap();
    let local = Rect::new(0.0, 0.0, 96.0, 32.0);
    for _ in 0..2 {
        let child = tree.add_child(host, local).unwrap();
        record(&tree, child, vec![rounded(local, RED)]);
    }
    let mut params = MorphParams::new(MorphStyle::Goo, 0.5);
    params.blur = 12.0;
    record(&tree, host, vec![marker(params)]);

    let pixels = render(AntiAlias::Analytic, &tree);

    near(pixel(&pixels, 64, 64), RED, 4, "the pill's middle keeps its color");
    assert_eq!(pixel(&pixels, 64, 44), [0; 4], "nothing above the pill");
    assert_eq!(pixel(&pixels, 64, 84), [0; 4], "nothing below the pill");
}

/// A host whose two pictures are the same small red square on nothing, so a
/// cross-fade between them is invisible unless something else changes the look.
fn square_scene(params: MorphParams) -> RenderTree {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
    let host = tree.add_child(root, Rect::new(16.0, 16.0, 96.0, 96.0)).unwrap();
    let local = Rect::new(0.0, 0.0, 96.0, 96.0);
    let square = Rect::new(36.0, 36.0, 24.0, 24.0);
    for _ in 0..2 {
        let child = tree.add_child(host, local).unwrap();
        record(&tree, child, vec![fill(square, RED)]);
    }
    record(&tree, host, vec![marker(params)]);
    tree
}

#[test]
fn a_blur_morph_softens_both_pictures_in_the_middle_and_leaves_the_ends_sharp() {
    let mut params = MorphParams::new(MorphStyle::Blur, 0.5);
    params.blur = 16.0;
    // The square covers x/y 52..76 of the target; probe just outside its edge
    // and at its centre.
    let outside = (48, 64);
    let centre = (64, 64);

    let sharp = render(AntiAlias::Analytic, &square_scene(MorphParams::new(MorphStyle::Blur, 0.5)));
    assert_eq!(pixel(&sharp, outside.0, outside.1), [0; 4], "no blur asked for, none drawn");

    let blurred = render(AntiAlias::Analytic, &square_scene(params));
    let halo = pixel(&blurred, outside.0, outside.1);
    assert!(halo[3] > 12 && halo[0] > 12, "the blur spreads past the square's edge: {halo:?}");
    let middle = pixel(&blurred, centre.0, centre.1);
    assert!(middle[3] > 200, "the middle of the square stays dense: {middle:?}");

    for progress in [0.0, 1.0] {
        let mut end = params;
        end.progress = progress;
        let pixels = render(AntiAlias::Analytic, &square_scene(end));
        assert_eq!(
            pixel(&pixels, outside.0, outside.1),
            [0; 4],
            "progress {progress}: the picture that is showing alone is sharp"
        );
        near(pixel(&pixels, centre.0, centre.1), RED, 3, "the ends keep the picture's color");
    }
}

#[test]
fn the_blur_is_the_stronger_the_further_the_morph_has_run_from_each_end() {
    let halo_at = |progress: f32| {
        let mut params = MorphParams::new(MorphStyle::Blur, progress);
        params.blur = 16.0;
        pixel(&render(AntiAlias::Analytic, &square_scene(params)), 48, 64)[3]
    };

    let (early, middle, late) = (halo_at(0.1), halo_at(0.5), halo_at(0.9));

    // Outgoing blur grows with progress while the incoming one shrinks, so the
    // two are equal at the middle and the sum is a sharp end, a soft middle.
    assert!(early < middle && late < middle, "{early} {middle} {late}");
}

#[test]
fn any_style_can_blur() {
    let mut params = MorphParams::new(MorphStyle::Wipe, 0.5);
    params.blur = 16.0;
    // A wipe over identical pictures changes nothing but the blur.
    let pixels = render(AntiAlias::Analytic, &square_scene(params));

    assert!(pixel(&pixels, 48, 64)[3] > 12, "a wipe blurs when asked to");
}

#[test]
fn the_blur_follows_the_device_scale() {
    // The same logical blur reaches twice as many pixels on a 2x target.
    let instance = |scale: f32| {
        let mut params = MorphParams::new(MorphStyle::Blur, 0.5);
        params.blur = 8.0;
        crate::pipeline::morph_pipeline::MorphInstance::new(&params, 1.0, scale, None, None).noise[1]
    };
    assert_eq!(instance(2.0), 2.0 * instance(1.0));
}

/// A renderer and target that outlive a frame, so a later frame can replay only
/// its damage and leave every other pixel as the earlier frames drew it.
struct Screen {
    backend: MetalBackend,
    target: <MetalBackend as GpuBackend>::Texture,
    view: <MetalBackend as GpuBackend>::TextureView,
    renderer: RendererImpl<MetalBackend>,
}

impl Screen {
    fn new() -> Self {
        let backend = MetalBackend::new().expect("morph pixel regression requires Metal");
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
        let view = backend.create_texture_view(&target, "morph screen");
        let renderer =
            RendererImpl::<MetalBackend>::with_antialiasing(&backend, format, AntiAlias::Analytic);
        Self { backend, target, view, renderer }
    }

    /// Draws the whole tree, as the first frame of a window does.
    fn present_all(&mut self, tree: &RenderTree) {
        let _ = tree.take_damage();
        let damage = vec![Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)];
        self.draw(RenderFrame { damage, operations: tree.render_all() });
    }

    /// Draws what changed in `tree` since the last frame, the way the app does.
    fn present(&mut self, tree: &RenderTree) {
        self.draw(tree.render_pending());
    }

    fn draw(&mut self, frame: RenderFrame) {
        // Unlike `from_v2_direct`, this accepts a frame that replays only damage.
        let packet = FramePacket::from_v2_direct_with_frame(
            frame,
            FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(SIZE, SIZE)),
            Frame::new(crate::draw_cmd::DrawList::new(), SIZE, SIZE),
        )
        .expect("lower the frame");
        self.renderer.render_packet(&self.backend, &self.view, &packet, false);
    }

    fn pixels(&self) -> Vec<u8> {
        let row = (SIZE * 4).div_ceil(256) * 256;
        let readback = self.backend.create_buffer(&BufferDescriptor {
            label: None,
            size: u64::from(row) * u64::from(SIZE),
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = self.backend.create_command_encoder("morph screen readback");
        self.backend.copy_texture_to_buffer(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &self.target,
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
            Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
        );
        self.backend.submit(encoder);
        let padded = self.backend.read_buffer(&readback, u64::from(row) * u64::from(SIZE));
        let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for line in padded.chunks(row as usize).take(SIZE as usize) {
            pixels.extend_from_slice(&line[..(SIZE * 4) as usize]);
        }
        pixels
    }
}

/// The outgoing picture is taller than the incoming one the host is sized to.
/// Frames that replay only the damage must still clear the part of the outgoing
/// picture the incoming one never covers, or it stays on screen until something
/// else repaints there.
#[test]
fn a_taller_outgoing_picture_is_cleared_by_damage_limited_frames() {
    for style in STYLES {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32)).unwrap();
        let host = tree.add_child(root, Rect::new(16.0, 16.0, 60.0, 20.0)).unwrap();
        // Small enough that the damage stays under the half of the target that
        // would promote it to a full repaint.
        let from = tree.add_child(host, Rect::new(0.0, 0.0, 60.0, 60.0)).unwrap();
        let to = tree.add_child(host, Rect::new(0.0, 0.0, 60.0, 20.0)).unwrap();
        record(&tree, from, vec![fill(Rect::new(0.0, 0.0, 60.0, 60.0), RED)]);
        record(&tree, to, vec![fill(Rect::new(0.0, 0.0, 60.0, 20.0), BLUE)]);
        record(&tree, host, vec![marker(MorphParams::new(style, 0.0))]);

        let mut screen = Screen::new();
        screen.present_all(&tree);
        near(pixel(&screen.pixels(), 40, 60), RED, 3, &format!("{style:?}: the tall picture shows first"));

        // A frame in the middle of the morph, then its last one.
        record(&tree, host, vec![marker(MorphParams::new(style, 0.5))]);
        screen.present(&tree);
        record(&tree, host, vec![marker(MorphParams::new(style, 1.0))]);
        screen.present(&tree);

        let pixels = screen.pixels();
        near(pixel(&pixels, 40, 26), BLUE, 3, &format!("{style:?}: the incoming picture"));
        assert_eq!(
            pixel(&pixels, 40, 60),
            [0; 4],
            "{style:?}: the outgoing picture's lower part is gone"
        );
    }
}

/// Per-frame cost of a running morph against the same two pictures stacked,
/// at a realistic size. Run with
/// `cargo nextest run -p aimer_cupid --lib -E 'test(profile_morph_frames)' --run-ignored only --no-capture`.
#[test]
#[ignore = "profile harness"]
fn profile_morph_frames() {
    const WIDTH: u32 = 900;
    const HEIGHT: u32 = 800;
    const FRAMES: u32 = 120;

    let backend = MetalBackend::new().expect("profiling requires Metal");
    let format = <MetalBackend as GpuBackend>::rgba8_unorm_format();
    let target = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (WIDTH, HEIGHT, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "morph profile target");
    let mut renderer =
        RendererImpl::<MetalBackend>::with_antialiasing(&backend, format, AntiAlias::Analytic);

    let sync = |backend: &MetalBackend| {
        // Reading one row back waits for every submitted pass to finish.
        let readback = backend.create_buffer(&BufferDescriptor {
            label: None,
            size: 256,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("morph profile sync");
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
                    bytes_per_row: Some(256),
                    rows_per_image: Some(1),
                },
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);
        let _ = backend.read_buffer(&readback, 256);
    };

    let card = Rect::new(0.0, 0.0, 600.0, 500.0);
    let mut measure_with = |label: &str, tree: &RenderTree, host: Option<RenderNodeId>, style: MorphStyle, blur: f32| {
        let mut run = |frames: u32| {
            let started = aimer_time::Instant::now();
            for frame in 0..frames {
                if let Some(host) = host {
                    let progress = frame as f32 / frames as f32;
                    let mut params = MorphParams::new(style, progress);
                    params.blur = blur;
                    record(tree, host, vec![marker(params)]);
                }
                let packet = FramePacket::from_v2_direct(
                    RenderFrame {
                        damage: vec![Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)],
                        operations: tree.render_all(),
                    },
                    FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(WIDTH, HEIGHT)),
                )
                .expect("lower the profile frame");
                renderer.render_packet(&backend, &view, &packet, false);
            }
            sync(&backend);
            started.elapsed()
        };
        run(10); // warm the pipelines and the group targets
        let elapsed = run(FRAMES);
        println!(
            "{label}: {:.3} ms/frame over {FRAMES} frames",
            elapsed.as_secs_f64() * 1000.0 / f64::from(FRAMES)
        );
    };

    let mut measure_styled = |label: &str, tree: &RenderTree, host: RenderNodeId, style: MorphStyle, blur: f32| {
        measure_with(label, tree, Some(host), style, blur)
    };
    for (style, blur) in [
        (MorphStyle::Dissolve, 0.0),
        (MorphStyle::Blur, 10.0),
        (MorphStyle::Goo, 14.0),
    ] {
        let morph = RenderTree::new();
        let root = morph.add_root(Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)).unwrap();
        let (host, _, _) = add_morph(&morph, root, Rect::new(100.0, 100.0, 600.0, 500.0), MorphParams::new(style, 0.0), RED, BLUE);
        measure_styled(&format!("{style:?} morph, two 600x500 pictures"), &morph, host, style, blur);
    }
    // The size a toolbar's controls have, where a goo morph is meant to be used.
    let small = RenderTree::new();
    let root = small.add_root(Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)).unwrap();
    let (host, _, _) = add_morph(&small, root, Rect::new(100.0, 100.0, 300.0, 56.0), MorphParams::new(MorphStyle::Goo, 0.0), RED, BLUE);
    measure_styled("Goo morph, two 300x56 pictures", &small, host, MorphStyle::Goo, 14.0);

    let stacked = RenderTree::new();
    let root = stacked.add_root(Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)).unwrap();
    let outer = stacked.add_child(root, Rect::new(100.0, 100.0, 600.0, 500.0)).unwrap();
    for color in [RED, BLUE] {
        let child = stacked.add_child(outer, card).unwrap();
        record(&stacked, child, vec![fill(card, color)]);
    }
    measure_with("same pictures stacked, no morph", &stacked, None, MorphStyle::Dissolve, 0.0);
}
