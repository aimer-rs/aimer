use super::Renderer;
use crate::backend::metal::MetalBackend;
use crate::backend::{GpuBackend, TextureDescriptor, TextureDimension, TextureUsage};
use crate::damage_region::{DamageRect, DamageSet};
use crate::draw_cmd::DrawList;
use crate::frame::{Frame, FramePacket, FrameRenderMetadata};
use objc2_metal::MTLPixelFormat;

#[test]
fn an_unpresented_frame_preserves_image_updates_and_invalidates_partial_reuse() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let format = MTLPixelFormat::BGRA8Unorm;
    let mut renderer = Renderer::new(&backend, format);
    let texture = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (8, 8, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment],
    });
    let view = backend.create_texture_view(&texture, "unpresented frame regression");
    renderer.render_packet(
        &backend,
        &view,
        &FramePacket::full(Frame::new(DrawList::new(), 8, 8)),
        false,
    );
    let mut commands = DrawList::new();
    commands.load_image_with_id(91, &[255; 4], 1, 1);
    renderer.discard_frame(&backend, &FramePacket::full(Frame::new(commands, 8, 8)));
    assert_eq!(renderer.memory_stats().image_texture_count, 1);
    let mut damage = DamageSet::new(8, 8);
    damage.add(DamageRect::new(0, 0, 1, 1));
    let packet = FramePacket::new(
        Frame::new(DrawList::new(), 8, 8),
        FrameRenderMetadata::new(1.0, 0, 0, 0, 0, damage),
    );
    renderer.render_packet(&backend, &view, &packet, false);
    assert!(renderer.compositor_stats().full_repaint);
    let mut commands = DrawList::new();
    commands.remove_texture(91);
    renderer.discard_frame(&backend, &FramePacket::full(Frame::new(commands, 8, 8)));
    assert_eq!(renderer.memory_stats().image_texture_count, 0);
}
