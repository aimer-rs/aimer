use super::*;

#[test]
fn light_edge_has_no_capture_and_fits_scaled_geometry_without_growing_uniforms() {
    let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [10.0, 20.0, 200.0, 80.0]);
    edge.border_width = 16.0;
    edge.corner_radii = [24.0; 4];
    edge.light_angle = 180.0;
    edge.edge_lighting = 0.65;
    edge.transform[0] = 2.0;
    edge.transform[4] = 2.0;
    assert!(backdrop_region(edge, 300, 200, 4096).is_none());
    let uniform = MaterialUniform::from_request(edge, 300, 200, false, None);
    assert_eq!(
        uniform.detail[3], 32.0,
        "logical width must survive Retina scaling"
    );
    assert_eq!(uniform.radii, [40.0; 4], "scaled radius fits the bounds");
    assert!((uniform.effect[2] + 1.0).abs() < 0.00001);
    assert!(uniform.effect[3].abs() < 0.00001);
    assert_eq!(
        uniform.light[3], 0.65,
        "reflection uses the existing lighting slot"
    );
    assert_eq!(size_of::<MaterialUniform>(), 224);
    edge.bounds[2] = 0.5;
    let uniform = MaterialUniform::from_request(edge, 300, 200, false, None);
    assert_eq!(uniform.detail[3], 0.25);
    assert_eq!(uniform.radii, [0.25; 4]);
}

#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod pixels {
    use super::*;
    use crate::backend::{metal::MetalBackend, *};
    use crate::draw_cmd::DrawList;
    use crate::renderer::RendererImpl;
    use objc2_metal::MTLPixelFormat;

    const SIZE: u32 = 128;
    const ROW: u32 = SIZE * 4;
    const BYTES: u64 = (ROW * SIZE) as u64;

    fn pixel(bytes: &[u8], x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * SIZE + x) * 4) as usize;
        bytes[offset..offset + 4].try_into().unwrap()
    }

    fn render(
        backend: &MetalBackend,
        renderer: &mut RendererImpl<MetalBackend>,
        draw: &DrawList,
    ) -> Vec<u8> {
        let target = backend.create_texture(&TextureDescriptor {
            label: None,
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: MTLPixelFormat::RGBA8Unorm,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        let view = backend.create_texture_view(&target, "light edge pixels");
        renderer.render(backend, &view, SIZE, SIZE, false, draw);
        let readback = backend.create_buffer(&BufferDescriptor {
            label: None,
            size: BYTES,
            usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
        });
        let mut encoder = backend.create_command_encoder("light edge readback");
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
        backend.read_buffer(&readback, BYTES)
    }

    #[test]
    fn light_edge_pixels_blend_across_corner_joins_and_tight_bevels() {
        let backend = MetalBackend::new().expect("light-edge pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        for scale in [1.0, 2.0] {
            let mut edge = MaterialRequest::new(
                MaterialKind::LightEdge,
                [16.0, 16.0, 96.0, 96.0].map(|value| value / scale),
            );
            edge.border_width = 16.0 / scale;
            edge.border_color = [1.0; 4];
            edge.specular_highlight = 1.0;
            edge.opacity = 1.0;
            edge.corner_radii = [26.0 / scale; 4];
            edge.light_angle = 225.0;
            let mut draw = DrawList::new();
            draw.scale(scale, scale);
            draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
            let pixels = render(&backend, &mut renderer, &draw);
            let straight = pixel(&pixels, 64, 17)[3];
            let transition = pixel(&pixels, 44, 17)[3];
            assert!(
                transition > straight + 5,
                "corner lighting must fade into the straight side: {transition} vs {straight}"
            );
            for radius in [0.0, 4.0, 8.0] {
                edge.corner_radii = [radius / scale; 4];
                edge.light_angle = 180.0;
                let mut draw = DrawList::new();
                draw.scale(scale, scale);
                draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
                let pixels = render(&backend, &mut renderer, &draw);
                let a = pixel(&pixels, 20, 21)[3];
                let b = pixel(&pixels, 21, 20)[3];
                assert!(
                    a.abs_diff(b) < 35,
                    "wide bevel must not split at the corner diagonal (radius {radius}): {a}, {b}"
                );
            }
        }
    }

    #[test]
    fn light_edge_pixels_use_configured_reflection_without_changing_primary_light() {
        let backend = MetalBackend::new().expect("light-edge pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [16.0, 16.0, 96.0, 96.0]);
        edge.border_width = 16.0;
        edge.border_color = [1.0; 4];
        edge.specular_highlight = 1.0;
        edge.opacity = 1.0;
        edge.light_angle = 270.0;
        let mut primary = None;
        for reflection in [0.0, 0.35, 1.0] {
            edge.edge_lighting = reflection;
            let mut draw = DrawList::new();
            draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
            let pixels = render(&backend, &mut renderer, &draw);
            let top = pixel(&pixels, 64, 17)[3];
            let bottom = pixel(&pixels, 64, 110)[3];
            assert_eq!(
                *primary.get_or_insert(top),
                top,
                "reflection must not alter the primary highlight"
            );
            assert!(
                bottom.abs_diff((top as f32 * reflection).round() as u8) <= 1,
                "configured opposing reflection {reflection}: primary {top}, reflected {bottom}"
            );
        }
    }

    #[test]
    fn light_edge_pixels_blend_inward_and_reflect_on_the_opposite_edge() {
        let backend = MetalBackend::new().expect("light-edge pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [16.0, 16.0, 96.0, 96.0]);
        edge.border_width = 16.0;
        edge.border_color = [1.0; 4];
        edge.specular_highlight = 1.0;
        edge.opacity = 1.0;
        edge.light_angle = 270.0;
        for radius in [24.0, 48.0] {
            edge.corner_radii = [radius; 4];
            let mut draw = DrawList::new();
            draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
            let pixels = render(&backend, &mut renderer, &draw);
            let alpha = [17, 20, 24, 28, 32].map(|y| pixel(&pixels, 64, y)[3]);
            assert!(alpha[0] > 120, "bright outer rim: {alpha:?}");
            assert!(
                alpha[1] > alpha[2] && alpha[0] > alpha[1],
                "no flat filled band: {alpha:?}"
            );
            assert!(
                (1..20).contains(&alpha[3]),
                "soft tail before the transparent interior: {alpha:?}"
            );
            assert_eq!(alpha[4], 0);
            let reflection = pixel(&pixels, 64, 110)[3];
            assert!(
                reflection > 0 && reflection < alpha[0],
                "weaker opposing reflection"
            );
            assert_eq!(pixel(&pixels, 64, 64), [0; 4]);
        }
    }

    #[test]
    fn light_edge_pixels_are_directional_transparent_and_confined() {
        let backend = MetalBackend::new().expect("light-edge pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [16.0, 32.0, 96.0, 64.0]);
        edge.border_width = 4.0;
        edge.border_color = [1.0; 4];
        edge.corner_radii = [16.0; 4];
        edge.specular_highlight = 1.0;
        edge.opacity = 1.0;
        edge.light_angle = 225.0;
        let mut draw = DrawList::new();
        draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
        let first = render(&backend, &mut renderer, &draw);
        assert_eq!(pixel(&first, 64, 64), [0; 4], "transparent interior");
        assert_eq!(pixel(&first, 15, 64), [0; 4], "rim stays inside bounds");
        assert_eq!(pixel(&first, 16, 32), [0; 4], "rounded corner stays empty");
        assert!(
            pixel(&first, 64, 32)[3] > 40,
            "top catches upper-left light"
        );
        assert!(
            pixel(&first, 16, 64)[3] > 40,
            "left catches upper-left light"
        );
        assert!(
            pixel(&first, 64, 95)[3] < pixel(&first, 64, 32)[3],
            "opposing reflection is weaker"
        );
        assert_eq!(pixel(&first, 64, 40), [0; 4], "only the thin rim is lit");
        edge.light_angle = 45.0;
        let mut draw = DrawList::new();
        draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
        let reversed = render(&backend, &mut renderer, &draw);
        assert!(pixel(&reversed, 64, 32)[3] < pixel(&first, 64, 32)[3]);
        assert!(
            pixel(&reversed, 64, 95)[3] > 40,
            "rotated light reaches bottom"
        );
    }

    #[test]
    fn light_edge_pixels_respect_scale_clip_alpha_and_premultiplied_composition() {
        use crate::utilities::{Color, Rect};
        let backend = MetalBackend::new().expect("light-edge pixel regression requires Metal");
        let mut renderer = RendererImpl::new(&backend, MTLPixelFormat::RGBA8Unorm);
        let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [8.0, 8.0, 48.0, 40.0]);
        edge.border_width = 16.0;
        edge.border_color = [1.0, 0.0, 0.0, 0.5];
        edge.corner_radii = [8.0; 4];
        edge.specular_highlight = 1.0;
        edge.opacity = 1.0;
        edge.light_angle = 270.0;
        for background in [false, true] {
            let mut draw = DrawList::new();
            if background {
                draw.fill_rect(
                    Rect::new(0.0, 0.0, 128.0, 128.0),
                    Color::rgba8(0, 0, 255, 255),
                    [0.0; 4],
                    [0.0; 4],
                    Color::transparent(),
                );
            }
            draw.scale(2.0, 2.0);
            draw.push_clip(Rect::new(0.0, 0.0, 32.0, 64.0));
            draw.set_alpha(0.5);
            draw.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
            let pixels = render(&backend, &mut renderer, &draw);
            let highlighted = pixel(&pixels, 48, 24);
            // 8.5 device pixels into a 32-pixel blend: (1 - 8.5 / 32)^3 * 0.5 * 0.5.
            let expected = if background {
                [25, 0, 230, 255]
            } else {
                [25, 0, 0, 25]
            };
            for (actual, expected) in highlighted.into_iter().zip(expected) {
                assert!(
                    actual.abs_diff(expected) <= 1,
                    "scaled 32-pixel rim with color/scene alpha: {highlighted:?}"
                );
            }
            assert!(
                pixel(&pixels, 48, 36)[0] > 0,
                "scaled blend extends past 16 device pixels"
            );
            let untouched = if background { [0, 0, 255, 255] } else { [0; 4] };
            assert_eq!(pixel(&pixels, 80, 36), untouched, "ancestor clip");
            assert_eq!(pixel(&pixels, 16, 16), untouched, "rounded corner scales");
            assert_eq!(pixel(&pixels, 48, 64), untouched, "transparent center");
        }
    }

    #[test]
    fn light_edge_pipeline_requests_no_capture_or_blur_with_or_without_a_source() {
        use crate::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
        let backend = MetalBackend::new().expect("light-edge pipeline regression requires Metal");
        let mut pipeline = MaterialPipeline::new(
            &backend,
            MTLPixelFormat::RGBA8Unorm,
            crate::AntiAlias::Analytic,
        );
        let source = backend.create_texture(&TextureDescriptor {
            label: None,
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: MTLPixelFormat::RGBA8Unorm,
            usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
        });
        for source_texture in [None, Some(&source)] {
            pipeline.begin_frame();
            let edge = MaterialRequest::new(MaterialKind::LightEdge, [8.0, 8.0, 48.0, 40.0]);
            let index = pipeline
                .prepare_command(&edge)
                .expect("analytic material accepted");
            pipeline.prepare(&RenderContextGeneric {
                backend: &backend,
                width: SIZE,
                height: SIZE,
                is_srgb: false,
                format: MTLPixelFormat::RGBA8Unorm,
                sample_count: 1,
                source_texture,
            });
            assert!(!pipeline.needs_backdrop(Some(index)));
            assert!(pipeline.blur.is_none());
            assert_eq!((pipeline.backdrop_width, pipeline.backdrop_height), (1, 1));
        }
    }
}
