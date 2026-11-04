use super::fixtures::*;
use aimer::canvas::Canvas;
use aimer_cupid::draw_cmd::Rect;
use aimer_cupid::frame::FramePacket;
use aimer::{ResolvedSize, Vec2d};
use aimer_decoration::{Decoration, Magnification, MagnificationStyle as S};
use aimer_style::*;

fn sample(pixels: &[u8], x: f32, y: f32) -> [f32; 4] {
    let left = (x - 0.5).floor();
    let top = (y - 0.5).floor();
    let tx = x - 0.5 - left;
    let ty = y - 0.5 - top;
    let at = |dx: f32, dy: f32| {
        pixel(
            pixels,
            (left + dx).clamp(0.0, FRAME_WIDTH as f32 - 1.0) as u32,
            (top + dy).clamp(0.0, FRAME_HEIGHT as f32 - 1.0) as u32,
        )
    };
    let (a, b, c, d) = (at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0));
    std::array::from_fn(|i| {
        (a[i] as f32 * (1.0 - tx) + b[i] as f32 * tx) * (1.0 - ty)
            + (c[i] as f32 * (1.0 - tx) + d[i] as f32 * tx) * ty
    })
}

fn anchor(style: S, low: S, high: S) -> f32 {
    match (style.contains(low), style.contains(high)) {
        (true, false) => 0.0,
        (false, true) => 1.0,
        _ => 0.5,
    }
}

fn refraction(point: [f32; 2], scale: f32) -> [f32; 2] {
    let radius = 8.0 * scale;
    let q = [
        point[0].abs() - 32.0 * scale + radius,
        point[1].abs() - 16.0 * scale + radius,
    ];
    let outer = [q[0].max(0.0), q[1].max(0.0)];
    let length = outer[0].hypot(outer[1]);
    let distance = length + q[0].max(q[1]).min(0.0) - radius;
    if -distance >= radius {
        return [0.0; 2];
    }
    let direction = if length > 0.0 {
        [outer[0] / length, outer[1] / length]
    } else if q[0] > q[1] {
        [1.0, 0.0]
    } else {
        [0.0, 1.0]
    };
    let u = (1.0 + distance / radius).clamp(0.0, 1.0);
    let slope = u / (1.0 - u * u).max(0.0001).sqrt();
    let nz = 1.0 / (1.0 + slope * slope).sqrt();
    let eta = 2.0 / 3.0;
    let factor = eta * nz - (1.0 - eta * eta * (1.0 - nz * nz)).sqrt();
    let depth_factor = factor * slope * nz * 12.0 * scale / (-eta + factor * nz).abs().max(0.05);
    [
        direction[0] * point[0].signum() * depth_factor,
        direction[1] * point[1].signum() * depth_factor,
    ]
}

pub(super) fn styles_match_sampling_and_preserve_disabled_axes(
    mut render: impl FnMut(&FramePacket) -> Vec<u8>,
) {
    let (tree, node) = backdrop_tree();
    for scale in [1.0, 2.0] {
        for alpha in [128, 255] {
            let canvas = Canvas::of(&tree.context(background_node(&tree)).unwrap());
            for y in 0..32 {
                for x in 0..64 {
                    canvas.fill_rect(
                        Rect::new((FILTER_X + x) as f32, y as f32, 1.0, 1.0),
                        [x as u8 * 4, y as u8 * 8, 120, alpha],
                    );
                }
            }
            canvas.finish();
            let paint = |style| {
                let canvas = Canvas::of(&tree.context(node).unwrap());
                vec![
                    BorderRadius::from(8.0).notype(),
                    Magnification::new()
                        .style(style)
                        .zoom(1.8)
                        .bulge(0.6)
                        .thickness(12.0)
                        .notype(),
                ]
                .paint(
                        &canvas,
                        Vec2d::default(),
                        ResolvedSize {
                            width: 64.0,
                            height: 32.0,
                        },
                        scale,
                    );
                canvas.finish();
                packet_from_tree(&tree, scale)
            };
            let baseline = render(&paint(S::NONE));
            for style in [
                S::UNIFORM,
                S::UNIFORM | S::LEFT,
                S::UNIFORM | S::RIGHT,
                S::UNIFORM | S::TOP,
                S::UNIFORM | S::BOTTOM,
                S::UNIFORM | S::LEFT | S::TOP,
                S::UNIFORM | S::RIGHT | S::BOTTOM,
                S::UNIFORM | S::LEFT | S::RIGHT | S::TOP | S::BOTTOM,
                S::UNIFORM | S::HORIZONTAL | S::LEFT,
                S::UNIFORM | S::VERTICAL | S::BOTTOM,
                S::BULGE,
                S::BULGE | S::HORIZONTAL | S::LEFT,
                S::BULGE | S::VERTICAL | S::TOP,
                S::REFRACTION,
                S::REFRACTION | S::HORIZONTAL,
                S::REFRACTION | S::VERTICAL,
                S::UNIFORM | S::BULGE,
                S::DEFAULT,
                S::ALL,
                S::ALL & !S::VERTICAL & !S::RIGHT,
                S::ALL & !S::HORIZONTAL & !S::TOP,
                S::HORIZONTAL | S::LEFT,
            ] {
                let pixels = render(&paint(style));
                let ax = if style.contains(S::VERTICAL) && !style.contains(S::HORIZONTAL) {
                    0.0
                } else {
                    1.0
                };
                let ay = if style.contains(S::HORIZONTAL) && !style.contains(S::VERTICAL) {
                    0.0
                } else {
                    1.0
                };
                let origin = [
                    (FILTER_X as f32 + 64.0 * anchor(style, S::LEFT, S::RIGHT)) * scale,
                    32.0 * anchor(style, S::TOP, S::BOTTOM) * scale,
                ];
                let extent = [
                    (origin[0] - FILTER_X as f32 * scale)
                        .max((FILTER_X + 64) as f32 * scale - origin[0]),
                    origin[1].max(32.0 * scale - origin[1]),
                ];
                for (dx, dy) in [
                    (8, 4),
                    (16, 8),
                    (24, 12),
                    (32, 16),
                    (40, 20),
                    (52, 26),
                    (56, 28),
                ] {
                    let x = ((FILTER_X + dx) as f32 * scale) as u32;
                    let y = (dy as f32 * scale) as u32;
                    let point = [x as f32 + 0.5, y as f32 + 0.5];
                    let n = [
                        (point[0] - origin[0]) / extent[0] * ax,
                        (point[1] - origin[1]) / extent[1] * ay,
                    ];
                    let falloff = (1.0 - n[0] * n[0] - n[1] * n[1]).max(0.0);
                    let bulge = if style.contains(S::BULGE) {
                        1.0 + 0.6 * falloff * falloff
                    } else {
                        1.0
                    };
                    let zoom = if style.contains(S::UNIFORM) { 1.8 } else { 1.0 };
                    let offset = if style.contains(S::REFRACTION) {
                        refraction(
                            [
                                point[0] - (FILTER_X + 32) as f32 * scale,
                                point[1] - 16.0 * scale,
                            ],
                            scale,
                        )
                    } else {
                        [0.0; 2]
                    };
                    let source = std::array::from_fn::<_, 2, _>(|i| {
                        let warped =
                            origin[i] + (point[i] - origin[i] + offset[i]) / (zoom * bulge);
                        point[i] + (warped - point[i]) * [ax, ay][i]
                    });
                    let expected = sample(&baseline, source[0], source[1]);
                    let actual = pixel(&pixels, x, y);
                    for channel in 0..4 {
                        assert!(
                            (actual[channel] as f32 - expected[channel]).abs() <= 2.0,
                            "style={style:?}, scale={scale}, alpha={alpha}, at=({dx},{dy}), channel={channel}: {actual:?} vs {expected:?}"
                        );
                    }
                }
            }
        }
    }
}
