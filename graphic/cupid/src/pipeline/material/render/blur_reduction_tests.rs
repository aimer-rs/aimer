//! A reduced backdrop blur must look like the blur it replaces.
//!
//! The blur of a wide radius is made on a smaller texture (see `downsample`).
//! These tests draw stripes behind a backdrop filter on the native Metal backend
//! and compare the result with a Gaussian computed on the CPU, so the reduced
//! passes of `material.metal` are held to the same picture as the full-size one.

#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod pixels {
    use super::super::*;
    use crate::backend::{metal::MetalBackend, *};
    use crate::draw_cmd::DrawList;
    use crate::renderer::RendererImpl;
    use crate::utilities::{Color, Rect};
    use objc2_metal::MTLPixelFormat;

    const WIDTH: u32 = 256;
    const HEIGHT: u32 = 512;
    const ROW: u32 = WIDTH * 4;
    const BYTES: u64 = (ROW * HEIGHT) as u64;
    /// The filtered rectangle: x, y, width, height.
    const FILTER: [f32; 4] = [20.0, 200.0, 216.0, 100.0];

    /// Stripes alternate every `stripe` rows between a dark and a light red.
    fn red_of_row(y: u32, stripe: u32) -> u8 {
        if (y / stripe) % 2 == 0 { 30 } else { 210 }
    }

    fn render(radius: f32, stripe: u32) -> Vec<u8> {
        let backend = MetalBackend::new().expect("blur pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        let mut draw = DrawList::new();
        for row in 0..HEIGHT / stripe + 1 {
            let red = red_of_row(row * stripe, stripe) as f32 / 255.0;
            draw.fill_rect(
                Rect::new(0.0, (row * stripe) as f32, WIDTH as f32, stripe as f32),
                Color::new(red, 0.3, 0.5, 1.0),
                [0.0; 4],
                [0.0; 4],
                Color::transparent(),
            );
        }
        let mut filter = MaterialRequest::new(MaterialKind::BackdropFilter, FILTER);
        filter.blur_radius = radius;
        // As `BackdropFilter` builds its request: the filtered capture replaces the page.
        filter.opacity = 1.0;
        filter.distortion_strength = 0.0;
        draw.draw_custom(MATERIAL_PIPELINE_NAME, filter.encode_for_test());

        let target = backend.create_texture(&TextureDescriptor {
            label: None,
            size: (WIDTH, HEIGHT, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: MTLPixelFormat::RGBA8Unorm,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "blur pixels");
        renderer.render(&backend, &view, WIDTH, HEIGHT, false, &draw);
        let readback = backend.create_buffer(&BufferDescriptor {
            label: None,
            size: BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("blur readback");
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
                    bytes_per_row: Some(ROW),
                    rows_per_image: Some(HEIGHT),
                },
            },
            Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        );
        backend.submit(encoder);
        backend.read_buffer(&readback, BYTES)
    }

    /// The red channel down the middle of the filter, blurred on the CPU by a
    /// Gaussian whose three sigma is `radius`, repeating the capture's edge rows.
    fn expected_red(radius: f32, stripe: u32, rows: std::ops::Range<u32>) -> Vec<f32> {
        let support = radius.ceil() as i64;
        let sigma = radius / 3.0;
        let top = (FILTER[1] as i64 - support).max(0);
        let bottom = ((FILTER[1] + FILTER[3]) as i64 + support).min(HEIGHT as i64);
        let weights: Vec<f32> = (-support..=support)
            .map(|d| (-(d * d) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f32 = weights.iter().sum();
        rows.map(|y| {
            let sum: f32 = weights
                .iter()
                .enumerate()
                .map(|(i, weight)| {
                    let row = (y as i64 + i as i64 - support).clamp(top, bottom - 1);
                    weight * red_of_row(row as u32, stripe) as f32
                })
                .sum();
            sum / total
        })
        .collect()
    }

    #[test]
    fn a_reduced_blur_stays_within_two_levels_of_a_cpu_gaussian() {
        // Sigma 3, 8, 16 and 32 pixels: the working texture is 1, 2, 4 and 8
        // times smaller than the capture. Stripes of 1, 3 and 5 rows are finer
        // than the reduction, which is what would alias if it only picked texels
        // instead of averaging them.
        for stripe in [40u32, 5, 3, 1] {
            for radius in [9.0f32, 24.0, 48.0, 96.0] {
                let pixels = render(radius, stripe);
                let rows = (FILTER[1] as u32 + 6)..(FILTER[1] as u32 + FILTER[3] as u32 - 6);
                let expected = expected_red(radius, stripe, rows.clone());
                let x = 128;
                let worst = rows
                    .clone()
                    .zip(&expected)
                    .map(|(y, expected)| {
                        let shown = pixels[((y * WIDTH + x) * 4) as usize] as f32;
                        (shown - expected).abs()
                    })
                    .fold(0.0f32, f32::max);
                println!("stripe {stripe:>2} radius {radius:>4}: worst {worst:.2} levels");
                assert!(
                    worst <= 2.0,
                    "stripe {stripe}, radius {radius}: {worst:.2} levels from a CPU Gaussian"
                );
            }
        }
    }
}
