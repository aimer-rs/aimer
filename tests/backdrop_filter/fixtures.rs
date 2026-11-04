use aimer::canvas::{Canvas as RetainedCanvas, FrameCanvas, InnerCanvas};
use aimer_cupid::damage_region::DamageSet;
use aimer_cupid::draw_cmd::{Rect, RenderFrame, RenderNodeId, RenderPaintSource, RenderTree};
use aimer_cupid::frame::{FramePacket, FrameRenderMetadata};
use aimer::widget::base::WindowHandle;
use aimer::{
     BuildContext, Drawable, ResolvedSize, SizedBox, Vec2d, Widget,
};
use aimer_decoration::*;

pub(super) const FRAME_WIDTH: u32 = 2_176;
pub(super) const FRAME_HEIGHT: u32 = 64;
pub(super) const FILTER_X: u32 = 1_024;
pub(super) const FILTER_SIZE: u32 = 64;

pub(super) fn filter_packet(
    tree: &RenderTree,
    node: RenderNodeId,
    filter: BackdropFilter,
) -> FramePacket {
    filter_packet_scaled(tree, node, filter, 1.0)
}

pub(super) fn filter_packet_scaled(
    tree: &RenderTree,
    node: RenderNodeId,
    filter: BackdropFilter,
    scale: f32,
) -> FramePacket {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let inner = InnerCanvas::new();
    let ctx = BuildContext::new(
        FrameCanvas::new(&inner),
        ResolvedSize {
            width: FILTER_SIZE as f32,
            height: FILTER_SIZE as f32,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(winit::dpi::PhysicalSize::new(FILTER_SIZE, FILTER_SIZE), 1.0),
        runtime.handle().clone(),
    );
    let element = DecorationBox::new()
        .decoration(filter)
        .child(
            SizedBox::new()
                .width(FILTER_SIZE as f32)
                .height(FILTER_SIZE as f32),
        )
        .to_element(&ctx);
    element.layout(&ctx);
    ctx.with_local_v2_paint_context(tree.context(node).unwrap(), |ctx| {
        element.paint(ctx)
    });
    packet_from_tree(tree, scale)
}

pub(super) fn packet_from_tree(tree: &RenderTree, scale: f32) -> FramePacket {
    FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, FRAME_WIDTH as f32, FRAME_HEIGHT as f32)],
            operations: tree.render_all(),
        },
        FrameRenderMetadata::new(scale, 0, 0, 0, 0, DamageSet::new(FRAME_WIDTH, FRAME_HEIGHT)),
    )
    .unwrap()
}

pub(super) fn background_node(tree: &RenderTree) -> RenderNodeId {
    tree.render_all()
        .into_iter()
        .find_map(|op| match op {
            aimer_cupid::draw_cmd::RenderOp::Draw(item) => Some(item.element),
            _ => None,
        })
        .unwrap()
}

pub(super) fn paint_background(tree: &RenderTree, color: [u8; 4]) {
    let canvas = RetainedCanvas::of(&tree.context(background_node(tree)).unwrap());
    canvas.fill_rect(Rect::new(FILTER_X as f32, 0.0, 64.0, 64.0), color);
    canvas.finish();
}

pub(super) fn backdrop_tree() -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree
        .add_root(Rect::new(0.0, 0.0, FRAME_WIDTH as f32, FRAME_HEIGHT as f32))
        .unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    let background = RetainedCanvas::of(&tree.context(root).unwrap());
    background.fill_rect(
        Rect::new(
            FILTER_X as f32,
            0.0,
            FILTER_SIZE as f32 / 2.0,
            FILTER_SIZE as f32,
        ),
        [0, 0, 0, 255],
    );
    background.fill_rect(
        Rect::new(
            FILTER_X as f32 + FILTER_SIZE as f32 / 2.0,
            0.0,
            FILTER_SIZE as f32 / 2.0,
            FILTER_SIZE as f32,
        ),
        [255, 255, 255, 255],
    );
    background.finish();

    let filter = tree
        .add_child(
            root,
            Rect::new(FILTER_X as f32, 0.0, FILTER_SIZE as f32, FILTER_SIZE as f32),
        )
        .unwrap();
    tree.set_paint_source(filter, RenderPaintSource::LocalV2)
        .unwrap();
    (tree, filter)
}

pub(super) fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * FRAME_WIDTH + x) * 4) as usize;
    pixels[offset..offset + 4].try_into().expect("RGBA pixel")
}

pub(super) fn paint_thin_cross(tree: &RenderTree, scale: f32) {
    let canvas = RetainedCanvas::of(&tree.context(background_node(tree)).unwrap());
    canvas.fill_rect(
        Rect::new(
            0.0,
            0.0,
            FRAME_WIDTH as f32 / scale,
            FRAME_HEIGHT as f32 / scale,
        ),
        [255; 4],
    );
    canvas.fill_rect(
        Rect::new(
            FILTER_X as f32 + 32.0,
            0.0,
            1.0 / scale,
            FRAME_HEIGHT as f32 / scale,
        ),
        [0, 0, 0, 255],
    );
    canvas.fill_rect(
        Rect::new(0.0, 16.0, FRAME_WIDTH as f32 / scale, 1.0 / scale),
        [0, 0, 0, 255],
    );
    canvas.finish();
}

pub(super) fn assert_gaussian_cross(pixels: &[u8], radius: f32, scale: f32) {
    let radius = radius * scale;
    let support = radius.ceil() as i32;
    let sigma = radius / 3.0;
    let weight = |offset: i32| (-0.5 * (offset as f32 / sigma).powi(2)).exp();
    let normalization: f32 = (-support..=support).map(weight).sum();
    let kernel = |offset: i32| {
        if offset <= support {
            weight(offset) / normalization
        } else {
            0.0
        }
    };
    let center_x = ((FILTER_X + 32) as f32 * scale) as u32;
    let center_y = (16.0 * scale) as u32;
    for offset in 0..24 {
        // A white backdrop with crossing one-device-pixel black lines has a
        // separable analytic response. Sparse taps produce peaks and gaps here.
        let expected = (255.0 * (1.0 - kernel(offset)) * (1.0 - kernel(0))).round() as u8;
        for (x, y) in [
            (center_x + offset as u32, center_y),
            (center_x, center_y + offset as u32),
        ] {
            let actual = pixel(pixels, x, y);
            assert_eq!(actual[3], 255);
            assert!(
                actual[..3]
                    .iter()
                    .all(|channel| channel.abs_diff(expected) <= 2),
                "Gaussian response at radius {radius}, scale {scale}, offset {offset}: expected {expected}, got {actual:?}"
            );
        }
    }
}
