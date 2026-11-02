use super::*;
use crate::{GradientShape, MaterialGradient};

fn gradient_request(
    shape: GradientShape,
    geometry: [f32; 4],
    stops: &[(f32, [u8; 4])],
) -> MaterialRequest {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [16.0, 16.0, 96.0, 96.0]);
    request.opacity = 1.0;
    request.corner_radii = [0.0; 4];
    request.gradient = MaterialGradient {
        shape,
        geometry,
        stop_count: stops.len() as u8,
        ..MaterialGradient::default()
    };
    for (index, (offset, color)) in stops.iter().enumerate() {
        request.gradient.offsets[index] = *offset;
        request.gradient.colors[index] = MaterialGradient::pack_color(*color);
    }
    request
}

#[test]
fn gradient_uniform_reuses_slots_without_growing_and_needs_no_capture() {
    let request = gradient_request(
        GradientShape::Radial,
        [48.0, 48.0, 40.0, 30.0],
        &[
            (0.0, [255, 0, 0, 255]),
            (0.25, [0, 255, 0, 255]),
            (0.5, [0, 0, 255, 255]),
            (0.75, [9, 8, 7, 6]),
            (1.0, [255, 255, 255, 255]),
        ],
    );
    assert!(backdrop_region(request, 300, 200, 4096).is_none());
    assert_eq!(size_of::<MaterialUniform>(), 224, "ramps fit the shared uniform");

    let uniform = MaterialUniform::from_request(request, 300, 200, false, None);
    assert_eq!(uniform.effect, [MaterialKind::Gradient as u8 as f32, 1.0, 0.0, 0.0]);
    assert_eq!(uniform.tint, [48.0, 48.0, 40.0, 30.0], "geometry");
    assert_eq!(uniform.light, [0.0, 0.25, 0.5, 0.75], "offsets 0..4");
    assert_eq!(uniform.detail[0], 1.0, "offset 4");
    assert_eq!(uniform.liquid, [1.0, 5.0, 0.0, 0.0], "shape and stop count");
    let color_bits = |slot: [f32; 4]| slot.map(f32::to_bits);
    assert_eq!(
        color_bits(uniform.border_color),
        [0, 1, 2, 3].map(|index| request.gradient.colors[index]),
        "colors travel as untouched bit patterns"
    );
    assert_eq!(
        color_bits(uniform.shadow)[0],
        request.gradient.colors[4],
        "colors 4..8"
    );
    assert_eq!(request.gradient.colors[3], u32::from_le_bytes([9, 8, 7, 6]));
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
    fn a_gradient_does_not_force_a_backdrop_capture() {
        use aimer_pipeline_core::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
        let backend = MetalBackend::new().expect("gradient pipeline regression requires Metal");
        let mut pipeline = MaterialPipeline::new(
            &backend,
            MTLPixelFormat::RGBA8Unorm,
            crate::AntiAlias::Analytic,
        );
        pipeline.begin_frame();
        let request = gradient_request(
            GradientShape::Linear,
            [0.0, 0.0, 48.0, 0.0],
            &[(0.0, [0; 4]), (1.0, [255; 4])],
        );
        let index = pipeline
            .prepare_command(&request)
            .expect("analytic material accepted");
        pipeline.prepare(&RenderContextGeneric {
            backend: &backend,
            width: SIZE,
            height: SIZE,
            is_srgb: false,
            format: MTLPixelFormat::RGBA8Unorm,
            sample_count: 1,
            source_texture: None,
        });
        assert!(!pipeline.needs_backdrop(Some(index)));
        assert!(pipeline.blur.is_none());
        assert_eq!((pipeline.backdrop_width, pipeline.backdrop_height), (1, 1));
    }
}
