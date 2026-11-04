use super::fixtures::*;
use aimer::canvas::Canvas;
use aimer_cupid::draw_cmd::{Rect, RenderNodeId, RenderPaintSource, RenderTree};
use aimer_cupid::frame::FramePacket;
use aimer::{ ResolvedSize, Vec2d};
use aimer_decoration::{BackdropFilter, Decoration, Magnification};
use aimer_style::BorderRadius;

fn lens_packet(
    tree: &RenderTree,
    node: RenderNodeId,
    lens: impl Decoration,
    scale: f32,
) -> FramePacket {
    let canvas = Canvas::of(&tree.context(node).unwrap());
    lens.paint(
        &canvas,
        Vec2d::default(),
        ResolvedSize {
            width: 64.0,
            height: 32.0,
        },
        1.0,
    );
    canvas.finish();
    packet_from_tree(tree, scale)
}

fn gradient(tree: &RenderTree, alpha: u8) {
    let canvas = Canvas::of(&tree.context(background_node(tree)).unwrap());
    for x in 0..64 {
        canvas.fill_rect(
            Rect::new((FILTER_X + x) as f32, 0.0, 1.0, 64.0),
            [x as u8 * 4, 80, 120, alpha],
        );
    }
    canvas.finish();
}

fn sampled(pixels: &[u8], x: f32, y: u32) -> [f32; 4] {
    let left = (x - 0.5).floor() as u32;
    let t = x - 0.5 - left as f32;
    let a = pixel(pixels, left, y);
    let b = pixel(pixels, left + 1, y);
    std::array::from_fn(|i| a[i] as f32 * (1.0 - t) + b[i] as f32 * t)
}

pub(super) fn centered_zoom_depth_and_alpha(mut render: impl FnMut(&FramePacket) -> Vec<u8>) {
    let (tree, node) = backdrop_tree();
    for scale in [1.0, 2.0] {
        for alpha in [0, 128, 255] {
            gradient(&tree, alpha);
            let baseline = render(&lens_packet(
                &tree,
                node,
                Magnification::new().zoom(1.0).thickness(0.0),
                scale,
            ));
            for zoom in [1.0, 2.0, 8.0] {
                let flat = render(&lens_packet(
                    &tree,
                    node,
                    Magnification::new()
                        .zoom(zoom)
                        .thickness(0.0),
                    scale,
                ));
                let center = (FILTER_X + 32) as f32 * scale;
                for dx in [4, 12, 24, 32, 42, 52, 60] {
                    let x = ((FILTER_X + dx) as f32 * scale) as u32;
                    let y = (16.0 * scale) as u32;
                    let expected = sampled(&baseline, center + (x as f32 + 0.5 - center) / zoom, y);
                    let actual = pixel(&flat, x, y);
                    for channel in 0..4 {
                        assert!(
                            (actual[channel] as f32 - expected[channel]).abs() <= 2.0,
                            "zoom {zoom} scale {scale} alpha {alpha} at {dx}: expected {expected:?}, actual {actual:?}"
                        );
                    }
                }
            }
            let flat = render(&lens_packet(
                &tree,
                node,
                Magnification::new()
                    .zoom(2.0)
                    .thickness(0.0),
                scale,
            ));
            let thick = render(&lens_packet(
                &tree,
                node,
                Magnification::new()
                    .zoom(2.0)
                    .thickness(12.0),
                scale,
            ));
            let edge = ((FILTER_X + 2) as f32 * scale) as u32;
            let y = (16.0 * scale) as u32;
            if alpha > 0 {
                assert!(
                    pixel(&thick, edge, y)[0] > pixel(&flat, edge, y)[0] + 2,
                    "depth must bend the left edge inward"
                );
                let depth = 12.0 * scale;
                let offset = refracted_x(2.0 * scale + 0.5, depth, -1.0);
                let center = (FILTER_X + 32) as f32 * scale;
                let expected = sampled(
                    &baseline,
                    center + (edge as f32 + 0.5 - center + offset) / 2.0,
                    y,
                );
                assert!(
                    (pixel(&thick, edge, y)[0] as f32 - expected[0]).abs() <= 2.0,
                    "edge must match Snell refraction at scale {scale}: {expected:?}"
                );
            }
            let center = ((FILTER_X + 32) as f32 * scale) as u32;
            assert_eq!(
                pixel(&thick, center, y),
                pixel(&flat, center, y),
                "flat lens center"
            );
            let clamped = render(&lens_packet(
                &tree,
                node,
                Magnification::new()
                    .zoom(2.0)
                    .thickness(512.0),
                scale,
            ));
            let bounded = render(&lens_packet(
                &tree,
                node,
                Magnification::new()
                    .zoom(2.0)
                    .thickness(16.0),
                scale,
            ));
            assert_eq!(clamped, bounded, "depth is capped at half the shorter side");
        }
    }
}

fn refracted_x(inset: f32, depth: f32, outward_x: f32) -> f32 {
    refracted_x_with_bevel(inset, depth, depth, outward_x)
}

pub(super) fn wide_action_bar_magnifies_backdrop(mut render: impl FnMut(&FramePacket) -> Vec<u8>) {
    let (tree, node) = backdrop_tree();
    for scale in [1.0, 2.0] {
        let width = FRAME_WIDTH as f32 / scale;
        let height = FRAME_HEIGHT as f32 / scale;
        let background = Canvas::of(&tree.context(background_node(&tree)).unwrap());
        background.fill_rect(Rect::new(0.0, 0.0, width, height), [16, 40, 80, 255]);
        background.fill_rect(
            Rect::new(width * 0.25, 0.0, 16.0, height),
            [220, 100, 60, 255],
        );
        background.finish();
        for opacity in [1.0, 0.5] {
            let canvas = Canvas::of(&tree.context(node).unwrap());
            canvas.set_alpha(opacity);
            vec![
                BorderRadius::from(16.0).notype(),
                Magnification::new().zoom(1.6).thickness(32.0).notype(),
            ]
            .paint(
                    &canvas,
                    Vec2d {
                        x: -(FILTER_X as f32),
                        y: 0.0,
                    },
                    ResolvedSize { width, height },
                    scale,
                );
            canvas.restore_alpha();
            canvas.finish();
            let pixels = render(&packet_from_tree(&tree, scale));
            // A stripe at 1/4 width moves to 1/10 width under centered 1.6x zoom.
            let x = (FRAME_WIDTH as f32 * 0.1 + 8.0 * scale) as u32;
            let actual = pixel(&pixels, x, FRAME_HEIGHT / 2);
            let expected = 16.0 + (220.0 - 16.0) * opacity;
            assert!(
                (actual[0] as f32 - expected).abs() <= 1.0,
                "full-width action bar lost magnification: scale={scale}, opacity={opacity}, pixel={actual:?}"
            );
            // Rounded-edge AA and ancestor opacity still need the original
            // backdrop, not just the smaller area sampled by centered zoom.
            assert_eq!(pixel(&pixels, 0, 0), [16, 40, 80, 255]);
        }
    }
}

fn refracted_x_with_bevel(inset: f32, depth: f32, bevel_width: f32, outward_x: f32) -> f32 {
    let u = (1.0 - inset / bevel_width).clamp(0.0, 1.0);
    let slope = u / (1.0 - u * u).max(0.0001).sqrt();
    let nz = 1.0 / (1.0 + slope * slope).sqrt();
    let nx = outward_x * slope * nz;
    let eta = 2.0 / 3.0;
    let cos_t = (1.0 - eta * eta * (1.0 - nz * nz)).sqrt();
    let factor = eta * nz - cos_t;
    factor * nx / (-eta + factor * nz).abs().max(0.05) * depth
}

pub(super) fn thick_rounded_corners_have_no_interior_normal_seam(
    mut render: impl FnMut(&FramePacket) -> Vec<u8>,
) {
    let (tree, node) = backdrop_tree();
    gradient(&tree, 255);
    for scale in [1.0, 2.0] {
        let baseline = render(&lens_packet(
            &tree,
            node,
            Magnification::new().zoom(1.0).thickness(0.0),
            scale,
        ));
        let center = (FILTER_X + 32) as f32 * scale;
        for radius in [2.0, 4.0, 8.0, 12.0] {
            for thickness in [radius, 16.0, 512.0] {
                let zoom = 1.5;
                let pixels = render(&lens_packet(
                    &tree,
                    node,
                    vec![
                        BorderRadius::from(radius).notype(),
                        Magnification::new().zoom(zoom).thickness(thickness).notype(),
                    ],
                    scale,
                ));
                // Past the corner's inner arc the bevel must already be flat.
                // Extending it to the glass depth makes the nearest-edge normal
                // jump by 90 degrees across each corner's interior diagonal.
                for (cx, cy) in [
                    (radius + 2.0, radius + 2.0),
                    (64.0 - radius - 3.0, radius + 2.0),
                    (radius + 2.0, 32.0 - radius - 3.0),
                    (64.0 - radius - 3.0, 32.0 - radius - 3.0),
                ] {
                    for dx in [-1.0, 0.0, 1.0] {
                        let x = ((FILTER_X as f32 + cx + dx) * scale) as u32;
                        let y = (cy * scale) as u32;
                        let expected =
                            sampled(&baseline, center + (x as f32 + 0.5 - center) / zoom, y);
                        let actual = pixel(&pixels, x, y);
                        assert!(
                            (actual[0] as f32 - expected[0]).abs() <= 1.5,
                            "rounded interior seam: radius={radius}, depth={thickness}, scale={scale}, point=({cx},{cy}), dx={dx}: {actual:?} vs {expected:?}"
                        );
                    }
                }
                // Fitting the bevel to the radius must preserve glass depth:
                // thicker glass still bends a ray farther along a straight edge.
                let x = ((FILTER_X + 1) as f32 * scale) as u32;
                let y = (16.0 * scale) as u32;
                let depth = thickness.min(16.0) * scale;
                let bevel = depth.min(radius * scale);
                let offset = refracted_x_with_bevel(scale + 0.5, depth, bevel, -1.0);
                let expected = sampled(
                    &baseline,
                    center + (x as f32 + 0.5 - center + offset) / zoom,
                    y,
                );
                assert!(
                    (pixel(&pixels, x, y)[0] as f32 - expected[0]).abs() <= 1.5,
                    "corner radius must constrain bevel width, not optical depth"
                );
            }
        }
    }
}

pub(super) fn rounded_corner_uses_device_space_normal(
    mut render: impl FnMut(&FramePacket) -> Vec<u8>,
) {
    let (tree, node) = backdrop_tree();
    gradient(&tree, 255);
    let baseline = render(&lens_packet(
        &tree,
        node,
        Magnification::new().zoom(1.0).thickness(0.0),
        1.0,
    ));
    let pixels = render(&lens_packet(
        &tree,
        node,
        vec![
            BorderRadius::from(12.0).notype(),
            Magnification::new().zoom(2.0).thickness(12.0).notype(),
        ],
        1.0,
    ));
    // Both coordinates are 7.5 pixels past the corner's inner square: the
    // normal is exactly diagonal, even though the lens is twice as wide as tall.
    let distance = (2.0f32 * 7.5 * 7.5).sqrt() - 12.0;
    let offset = refracted_x(-distance, 12.0, -std::f32::consts::FRAC_1_SQRT_2);
    let center = (FILTER_X + 32) as f32;
    let expected = sampled(&baseline, center + (-27.5 + offset) / 2.0, 4);
    let actual = pixel(&pixels, FILTER_X + 4, 4);
    assert!(
        (actual[0] as f32 - expected[0]).abs() <= 1.5,
        "corner must refract using device-space normal: {actual:?}, expected {expected:?}"
    );
}

pub(super) fn rounded_clip_foreground_and_fresh_capture(
    mut render: impl FnMut(&FramePacket) -> Vec<u8>,
) {
    let (tree, node) = backdrop_tree();
    gradient(&tree, 255);
    let baseline = render(&lens_packet(
        &tree,
        node,
        Magnification::new().zoom(1.0).thickness(0.0),
        1.0,
    ));
    let rounded = render(&lens_packet(
        &tree,
        node,
        vec![
            BorderRadius::from(16.0).notype(),
            Magnification::new().zoom(2.0).thickness(0.0).notype(),
        ],
        1.0,
    ));
    assert_eq!(pixel(&rounded, FILTER_X, 0), pixel(&baseline, FILTER_X, 0));
    let canvas = Canvas::of(&tree.context(node).unwrap());
    tree.set_clip(node, Some(Rect::new(16.0, 0.0, 32.0, 32.0)))
        .unwrap();
    Magnification::new()
        .zoom(2.0)
        .thickness(0.0)
        .paint(
            &canvas,
            Vec2d::default(),
            ResolvedSize {
                width: 64.0,
                height: 32.0,
            },
            1.0,
        );
    canvas.finish();
    let foreground = tree
        .add_child(node, Rect::new(30.0, 14.0, 2.0, 2.0))
        .unwrap();
    tree.set_paint_source(foreground, RenderPaintSource::LocalV2)
        .unwrap();
    let canvas = Canvas::of(&tree.context(foreground).unwrap());
    canvas.fill_rect(Rect::new(0.0, 0.0, 2.0, 2.0), [0, 255, 0, 255]);
    canvas.finish();
    let clipped = render(&packet_from_tree(&tree, 1.0));
    assert_eq!(
        pixel(&clipped, FILTER_X + 4, 16),
        pixel(&baseline, FILTER_X + 4, 16)
    );
    assert_eq!(pixel(&clipped, FILTER_X + 30, 14), [0, 255, 0, 255]);
    assert_ne!(pixel(&clipped, FILTER_X + 29, 14), [0, 255, 0, 255]);
    let revision = tree.draw_list_revision(node).unwrap();
    paint_background(&tree, [32, 64, 128, 255]);
    let updated = render(&packet_from_tree(&tree, 1.0));
    assert_eq!(pixel(&updated, FILTER_X + 24, 16), [32, 64, 128, 255]);
    assert_eq!(tree.draw_list_revision(node).unwrap(), revision);
}

pub(super) fn nested_filters_and_budget_fallback(mut render: impl FnMut(&FramePacket) -> Vec<u8>) {
    use BackdropFilter;
    let (tree, node) = backdrop_tree();
    gradient(&tree, 255);
    let lens = Magnification::new()
        .zoom(2.0)
        .thickness(0.0);
    let size = ResolvedSize {
        width: 64.0,
        height: 32.0,
    };
    let flat = render(&lens_packet(&tree, node, lens, 1.0));
    for filter_first in [true, false] {
        let canvas = Canvas::of(&tree.context(node).unwrap());
        let filter = BackdropFilter::new().blur_radius(0.0).brightness(0.5);
        if filter_first {
            filter.paint(&canvas, Vec2d::default(), size, 1.0);
            lens.paint(&canvas, Vec2d::default(), size, 1.0);
        } else {
            lens.paint(&canvas, Vec2d::default(), size, 1.0);
            filter.paint(&canvas, Vec2d::default(), size, 1.0);
        }
        canvas.finish();
        let combined = render(&packet_from_tree(&tree, 1.0));
        for dx in [8, 24, 40, 56] {
            let expected = pixel(&flat, FILTER_X + dx, 16);
            let actual = pixel(&combined, FILTER_X + dx, 16);
            assert_eq!(actual[3], 255);
            for channel in 0..3 {
                assert!(
                    actual[channel].abs_diff(expected[channel] / 2) <= 2,
                    "ordered lens/filter capture, filter_first={filter_first}: {actual:?} vs {expected:?}"
                );
            }
        }
    }
    let baseline = render(&lens_packet(&tree, node, lens.zoom(1.0), 1.0));
    let canvas = Canvas::of(&tree.context(node).unwrap());
    // A wide shallow lens is supported; exceed the full-resolution pixel
    // budget in both dimensions to exercise the unavailable-capture fallback.
    lens.paint(
        &canvas,
        Vec2d {
            x: -(FILTER_X as f32),
            y: 0.0,
        },
        ResolvedSize {
            width: 4096.0,
            height: 4096.0,
        },
        1.0,
    );
    canvas.finish();
    let fallback = render(&packet_from_tree(&tree, 1.0));
    assert_eq!(
        fallback, baseline,
        "over-budget capture leaves backdrop unchanged"
    );
    let canvas = Canvas::of(&tree.context(node).unwrap());
    Magnification::new().zoom(8.0).thickness(512.0).paint(
        &canvas,
        Vec2d { x: 31.5, y: 15.5 },
        ResolvedSize {
            width: 1.0,
            height: 1.0,
        },
        1.0,
    );
    canvas.finish();
    let tiny = render(&packet_from_tree(&tree, 1.0));
    assert_eq!(pixel(&tiny, FILTER_X + 31, 15)[3], 255);
    let canvas = Canvas::of(&tree.context(node).unwrap());
    canvas.set_alpha(0.0);
    lens.paint(&canvas, Vec2d::default(), size, 1.0);
    canvas.restore_alpha();
    canvas.finish();
    let hidden = render(&packet_from_tree(&tree, 1.0));
    assert!(hidden == baseline, "zero ancestor opacity hides the lens");
    let canvas = Canvas::of(&tree.context(node).unwrap());
    canvas.set_alpha(0.5);
    lens.paint(&canvas, Vec2d::default(), size, 1.0);
    canvas.restore_alpha();
    canvas.finish();
    let faded = render(&packet_from_tree(&tree, 1.0));
    let original = pixel(&baseline, FILTER_X + 8, 16);
    let magnified = pixel(&flat, FILTER_X + 8, 16);
    let actual = pixel(&faded, FILTER_X + 8, 16);
    assert!(
        (actual[0] as f32 - (original[0] as f32 + magnified[0] as f32) * 0.5).abs() <= 1.0,
        "ancestor opacity blends lens with unchanged capture"
    );
}
