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

// These read `MaterialPipeline`'s private state, so they stay in this crate; the
// Metal backend is a dev-dependency.
#[cfg(all(
    not(feature = "wgpu"),
    any(target_os = "macos", target_os = "ios")
))]
mod metal_pipeline_state {
    use super::*;
    use aimer_cupid_gpu::*;
    use aimer_metal::MetalBackend;
    use objc2_metal::MTLPixelFormat;

    const SIZE: u32 = 128;
    #[test]
    fn light_edge_pipeline_requests_no_capture_or_blur_with_or_without_a_source() {
        use aimer_pipeline_core::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
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
