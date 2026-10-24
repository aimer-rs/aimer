use super::*;
use crate::backend::metal::MetalBackend;
use crate::backend::*;
use crate::damage_region::DamageSet;
use crate::frame::Frame;
use crate::utilities::Color;
use objc2_metal::MTLPixelFormat;

const FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;

fn target(
    backend: &MetalBackend,
    width: u32,
    height: u32,
) -> <MetalBackend as GpuBackend>::Texture {
    backend.create_texture(&TextureDescriptor {
        label: None,
        size: (width, height, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: FORMAT,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    })
}

fn packet(width: u32, height: u32, draw: DrawList, damage: DamageSet) -> FramePacket {
    FramePacket::new(
        Frame::new(draw, width, height),
        FrameRenderMetadata::new(1.0, 1, 1, 1, 1, damage),
    )
}

fn pixels(backend: &MetalBackend, texture: &<MetalBackend as GpuBackend>::Texture) -> Vec<u8> {
    let readback = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: 64 * 64 * 4,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("damage clear test readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(64),
            },
        },
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
    );
    backend.submit(encoder);
    backend.read_buffer(&readback, 64 * 64 * 4)
}

#[test]
fn resizing_full_repaints_does_not_allocate_cpu_clear_images() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut renderer = Renderer::new(&backend, FORMAT);
    for (width, height) in [(64, 64), (128, 96), (63, 81), (64, 64)] {
        let texture = target(&backend, width, height);
        let view = backend.create_texture_view(&texture, "resize clear test");
        renderer.render_packet(
            &backend,
            &view,
            &packet(
                width,
                height,
                DrawList::new(),
                DamageSet::full(width, height),
            ),
            false,
        );
        assert!(
            renderer.damage_clear_texture.is_none(),
            "full repaints must clear on the GPU"
        );
    }
}

#[test]
fn gpu_full_clear_and_partial_clear_preserve_their_pixel_contract() {
    let backend = MetalBackend::new().expect("native GPU regression requires Metal");
    let mut renderer = Renderer::new(&backend, FORMAT);
    let texture = target(&backend, 64, 64);
    let view = backend.create_texture_view(&texture, "clear pixel test");
    let mut draw = DrawList::new();
    draw.fill_rect(
        Rect::new(0.0, 0.0, 16.0, 16.0),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    draw.fill_rect(
        Rect::new(40.0, 40.0, 16.0, 16.0),
        Color::blue(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    renderer.render_packet(
        &backend,
        &view,
        &packet(64, 64, draw, DamageSet::full(64, 64)),
        false,
    );
    let initial = pixels(&backend, &texture);
    let pixel =
        |bytes: &[u8], x: usize, y: usize| bytes[(y * 64 + x) * 4..(y * 64 + x + 1) * 4].to_vec();
    assert_eq!(pixel(&initial, 8, 8), [255, 0, 0, 255]);
    assert_eq!(pixel(&initial, 48, 48), [0, 0, 255, 255]);
    assert_eq!(pixel(&initial, 32, 32), [0; 4]);

    // An empty damage set preserves the complete retained image.
    renderer.render_packet(
        &backend,
        &view,
        &packet(64, 64, DrawList::new(), DamageSet::new(64, 64)),
        false,
    );
    assert_eq!(pixels(&backend, &texture), initial);

    let mut damage = DamageSet::new(64, 64);
    damage.add(DamageRect::new(0, 0, 16, 16));
    renderer.render_packet(
        &backend,
        &view,
        &packet(64, 64, DrawList::new(), damage),
        false,
    );
    let partial = pixels(&backend, &texture);
    assert_eq!(pixel(&partial, 8, 8), [0; 4]);
    assert_eq!(pixel(&partial, 48, 48), [0, 0, 255, 255]);

    renderer.render_packet(
        &backend,
        &view,
        &packet(64, 64, DrawList::new(), DamageSet::full(64, 64)),
        false,
    );
    assert!(pixels(&backend, &texture).iter().all(|byte| *byte == 0));
}
