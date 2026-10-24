//! The material pipelines are built before an animation needs them.
//!
//! Compiling the material shader is the slowest thing a first backdrop blur
//! does, and on a device that has never compiled it, it was done in the middle
//! of whatever animation first drew a blur. These tests hold the renderer to
//! doing it on the frame after the first one instead.

use super::Renderer;
use crate::backend::metal::MetalBackend;
use crate::backend::{GpuBackend, TextureDescriptor, TextureDimension, TextureUsage};
use crate::draw_cmd::DrawList;
use crate::frame::{Frame, FramePacket};
use objc2_metal::MTLPixelFormat;

fn render_empty_frame(backend: &MetalBackend, renderer: &mut Renderer<MetalBackend>) {
    let format = MTLPixelFormat::BGRA8Unorm;
    let texture = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (8, 8, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment],
    });
    let view = backend.create_texture_view(&texture, "prewarm regression");
    renderer.render_packet(
        backend,
        &view,
        &FramePacket::full(Frame::new(DrawList::new(), 8, 8)),
        false,
    );
}

#[test]
fn the_material_pipelines_are_built_on_the_second_frame_without_any_material_draw() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut renderer = Renderer::new(&backend, MTLPixelFormat::BGRA8Unorm);
    assert!(!renderer.material_pipeline_is_warm(), "nothing is built before a frame");

    render_empty_frame(&backend, &mut renderer);
    assert!(
        !renderer.material_pipeline_is_warm(),
        "the first frame is on screen sooner without the material shader"
    );

    render_empty_frame(&backend, &mut renderer);
    assert!(
        renderer.material_pipeline_is_warm(),
        "a backdrop blur drawn later must not compile the material shader mid-animation"
    );
}

#[test]
fn a_renderer_without_materials_never_builds_them() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut renderer = Renderer::new_rect_only(&backend, MTLPixelFormat::BGRA8Unorm);
    for _ in 0..3 {
        render_empty_frame(&backend, &mut renderer);
    }
    assert!(!renderer.material_pipeline_is_warm());
}
