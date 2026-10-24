//! OpenGL backend tests that drive the real renderer.
//!
//! They live beside the renderer rather than in `aimer_cupid_opengl` because
//! they need draw lists, and the backend crate must not depend on them.

use aimer_cupid_opengl::*;
use crate::backend::*;
use crate::draw_cmd::DrawList;
use crate::renderer::RendererImpl;
use crate::utilities::{Color, Rect};

#[test]
fn generic_renderer_renders_and_reads_back_with_headless_opengl() {
    const SIZE: u32 = 32;
    let backend = match OpenGlBackend::new_headless() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("skipping: a headless OpenGL 3.3 context is unavailable: {error}");
            return;
        }
    };
    let mut renderer =
        RendererImpl::<OpenGlBackend>::new(&backend, OpenGlBackend::rgba8_unorm_format());
    let mut draw = DrawList::new();
    draw.fill_rect(
        Rect::new(0.0, 0.0, SIZE as f32, (SIZE / 2) as f32),
        Color::red(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    draw.fill_rect(
        Rect::new(0.0, (SIZE / 2) as f32, SIZE as f32, (SIZE / 2) as f32),
        Color::blue(),
        [0.0; 4],
        [0.0; 4],
        Color::transparent(),
    );
    let target = backend.create_texture(&TextureDescriptor {
        label: Some("Cupid OpenGL readback test target".to_owned()),
        size: (SIZE, SIZE, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: OpenGlBackend::rgba8_unorm_format(),
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "Cupid OpenGL test view");
    renderer.render(&backend, &view, SIZE, SIZE, false, &draw);
    let gl_error = backend
        .pending_gl_error()
        .expect("the backend context should be current");
    assert_eq!(gl_error, 0, "OpenGL reported an error after rendering");

    let row_bytes = SIZE * 4;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: Some("Cupid OpenGL readback test buffer".to_owned()),
        size: (row_bytes * SIZE) as u64,
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("Cupid OpenGL readback test");
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
                bytes_per_row: Some(row_bytes),
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
    let pixels = backend.read_buffer(&readback, (row_bytes * SIZE) as u64);
    let colored_pixels = pixels
        .chunks_exact(4)
        .filter(|pixel| pixel.iter().any(|channel| *channel != 0))
        .count();
    assert_eq!(
        &pixels[..4],
        &[0, 0, 255, 255],
        "the bottom logical band should stay at the bottom"
    );
    let top = ((SIZE - 1) * row_bytes) as usize;
    assert_eq!(
        &pixels[top..top + 4],
        &[255, 0, 0, 255],
        "the top logical band should stay at the top"
    );
    assert_eq!(colored_pixels, (SIZE * SIZE) as usize);
}
