use aimer::canvas::Canvas;
use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd::{Rect, RenderFrame, RenderPaintSource, RenderTree};
use aimer_cupid::frame::{FramePacket, FrameRenderMetadata};
use aimer::{ ResolvedSize, Vec2d};
use aimer_decoration::{BackdropFilter, Decoration};

// A padded iPad-sized target also keeps GPU readback rows 256-byte aligned.
pub(super) const WIDTH: u32 = 2816;
pub(super) const HEIGHT: u32 = 128;

pub(super) fn wide_toolbar_blurs_and_refreshes(mut render: impl FnMut(&FramePacket) -> Vec<u8>) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 1408.0, 64.0)).unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    let node = tree
        .add_child(root, Rect::new(252.0, 0.0, 1156.0, 60.0))
        .unwrap();
    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    let foreground = tree
        .add_child(node, Rect::new(577.0, 30.0, 2.0, 2.0))
        .unwrap();
    tree.set_paint_source(foreground, RenderPaintSource::LocalV2)
        .unwrap();
    let canvas = Canvas::of(&tree.context(foreground).unwrap());
    canvas.fill_rect(Rect::new(0.0, 0.0, 2.0, 2.0), [0, 255, 0, 255]);
    canvas.finish();
    for scale in [1.0, 2.0] {
        for alpha in [128, 255] {
            let canvas = Canvas::of(&tree.context(root).unwrap());
            canvas.fill_rect(Rect::new(0.0, 0.0, 1408.0, 64.0), [0, 0, 0, alpha]);
            canvas.fill_rect(Rect::new(830.0, 0.0, 578.0, 64.0), [255, 255, 255, 255]);
            canvas.finish();
            let canvas = Canvas::of(&tree.context(node).unwrap());
            BackdropFilter::new().blur_radius(4.0).paint(
                &canvas,
                Vec2d::default(),
                ResolvedSize {
                    width: 1156.0,
                    height: 60.0,
                },
                scale,
            );
            canvas.finish();
            let packet = || {
                FramePacket::from_v2_direct(
                    RenderFrame {
                        damage: vec![Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32)],
                        operations: tree.render_all(),
                    },
                    FrameRenderMetadata::new(scale, 0, 0, 0, 0, DamageSet::new(WIDTH, HEIGHT)),
                )
                .unwrap()
            };
            let pixels = render(&packet());
            let at = |pixels: &[u8], x: u32, y: u32| {
                let offset = ((y * WIDTH + x) * 4) as usize;
                <[u8; 4]>::try_from(&pixels[offset..offset + 4]).unwrap()
            };
            let center = (830.0 * scale) as i32;
            let radius = 4.0 * scale;
            let support = radius.ceil() as i32;
            let weight = |offset: i32| (-0.5 * (offset as f32 / (radius / 3.0)).powi(2)).exp();
            let total: f32 = (-support..=support).map(weight).sum();
            for dx in -support..=support {
                let x = center + dx;
                let white: f32 = (-support..=support)
                    .filter(|tap| x + tap >= center)
                    .map(weight)
                    .sum::<f32>()
                    / total;
                let expected = 255.0 * white;
                let expected_alpha = alpha as f32 * (1.0 - white) + 255.0 * white;
                let actual = at(&pixels, x as u32, (16.0 * scale) as u32);
                assert!(
                    actual[..3]
                        .iter()
                        .all(|value| (*value as f32 - expected).abs() <= 2.0)
                        && (actual[3] as f32 - expected_alpha).abs() <= 2.0,
                    "wide toolbar Gaussian missing: scale={scale}, alpha={alpha}, dx={dx}, actual={actual:?}, expected={expected}, expected alpha={expected_alpha}"
                );
            }
            assert_eq!(
                at(&pixels, center as u32, (31.0 * scale) as u32),
                [0, 255, 0, 255],
                "foreground stays sharp"
            );
            let revision = tree.draw_list_revision(node).unwrap();
            let canvas = Canvas::of(&tree.context(root).unwrap());
            canvas.fill_rect(Rect::new(0.0, 0.0, 1408.0, 64.0), [40, 80, 120, 255]);
            canvas.finish();
            let updated = render(&packet());
            assert_eq!(
                at(&updated, center as u32, (16.0 * scale) as u32),
                [40, 80, 120, 255],
                "capture must refresh"
            );
            assert_eq!(tree.draw_list_revision(node).unwrap(), revision);
        }
    }
}
