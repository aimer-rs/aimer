use super::*;

#[test]
fn lens_style_parameters_use_box_anchor_and_only_capture_refraction_margin() {
    use crate::pipeline::material::{MagnificationStyle as S, MaterialCapabilities};
    let mut request =
        MaterialRequest::new(MaterialKind::Magnification, [100.0, 200.0, 320.0, 208.0]);
    request.magnification_style = S::UNIFORM | S::BULGE | S::HORIZONTAL | S::LEFT | S::BOTTOM;
    request.bulge_strength = 0.6;
    let uniform = MaterialUniform::from_request(request, 900, 800, false, None);
    assert_eq!(uniform.effect, [3.0, 0.72, 1.0, 0.0]);
    assert_eq!(uniform.liquid, [100.0, 408.0, 1.5, 0.6]);
    assert_eq!(&uniform.liquid2[1..3], &[0.0, 0.0]);
    let region = backdrop_region(request, 900, 800, 4096).unwrap();
    assert_eq!(
        (region.x, region.y, region.width, region.height),
        (99, 199, 322, 210)
    );
    request.magnification_style = S::REFRACTION | S::VERTICAL;
    let uniform = MaterialUniform::from_request(request, 900, 800, false, None);
    assert_eq!(uniform.effect, [3.0, 0.72, 0.0, 1.0]);
    assert_eq!(uniform.liquid, [260.0, 304.0, 1.0, 0.0]);
    assert_eq!(uniform.liquid2[1], 16.0);
    let capabilities = MaterialCapabilities::default();
    for style in [
        S::NONE,
        S::HORIZONTAL | S::LEFT,
        S::UNIFORM,
        S::BULGE,
        S::REFRACTION,
        S::ALL,
    ] {
        request.magnification_style = style;
        request.magnification = 1.0;
        request.bulge_strength = 0.0;
        request.bevel_radius = 0.0;
        assert_eq!(
            plan_material(request, capabilities).path,
            MaterialRenderPath::Skip
        );
    }
    request.magnification_style = S::BULGE;
    request.bulge_strength = 0.1;
    assert_ne!(
        plan_material(request, capabilities).path,
        MaterialRenderPath::Skip
    );
}

#[test]
fn wide_lens_plan_uses_full_resolution_with_hardware_and_pixel_budgets() {
    use crate::pipeline::material::{MaterialCapabilities, bounded_intermediate_extent};
    let capabilities = MaterialCapabilities {
        max_surface_dimension: 4_096,
        ..Default::default()
    };
    let mut request = MaterialRequest::new(MaterialKind::Magnification, [0.0, 0.0, 3_840.0, 120.0]);
    for quality in [1, 2, 4] {
        request.quality = quality;
        let extent =
            bounded_intermediate_extent(request, capabilities).expect("wide shallow lens fits");
        assert_eq!(
            (extent.width, extent.height),
            (3_840, 120),
            "lenses sample at full resolution"
        );
    }
    request.bounds[2] = 4_097.0;
    assert!(
        bounded_intermediate_extent(request, capabilities).is_none(),
        "hardware width limit"
    );
    request.bounds = [0.0, 0.0, 4_096.0, 1_024.0];
    assert!(
        bounded_intermediate_extent(request, capabilities).is_some(),
        "exact pixel budget"
    );
    request.bounds[3] += 1.0;
    assert!(
        bounded_intermediate_extent(request, capabilities).is_none(),
        "pixel budget exceeded"
    );
}

#[test]
fn rounded_lens_bevel_width_fits_corners_without_reducing_depth() {
    let mut request = MaterialRequest::new(MaterialKind::Magnification, [0.0, 0.0, 320.0, 208.0]);
    request.bevel_radius = 64.0;
    for (radii, width) in [
        ([16.0; 4], 16.0),
        ([0.0; 4], 64.0),
        ([32.0, 8.0, 24.0, 16.0], 8.0),
        ([0.0, 16.0, 0.0, 32.0], 16.0),
        ([0.25; 4], 0.25),
        ([1024.0; 4], 64.0),
    ] {
        request.corner_radii = radii;
        let uniform = MaterialUniform::from_request(request, 640, 400, false, None);
        assert_eq!(
            uniform.liquid2[1], 64.0,
            "corner rounding must not cap optical depth"
        );
        assert_eq!(uniform.liquid2[2], width);
    }
    request.bevel_radius = 0.0;
    assert_eq!(
        MaterialUniform::from_request(request, 640, 400, false, None).liquid2[2],
        0.0
    );
}

#[test]
fn lens_depth_scales_and_capture_margin_is_bounded() {
    let mut request =
        MaterialRequest::new(MaterialKind::Magnification, [100.0, 100.0, 320.0, 208.0]);
    request.transform[0] = 2.0;
    request.transform[4] = 2.0;
    assert_eq!(lens_thickness_pixels(&request), 32.0);
    let uniform = MaterialUniform::from_request(request, 640, 400, false, None);
    assert_eq!(uniform.liquid[2], 1.5);
    assert_eq!(uniform.liquid2[1], 32.0);
    let capture = backdrop_region(request, 640, 400, 2_048).unwrap();
    assert_eq!(
        (capture.x, capture.y, capture.width, capture.height),
        (35, 35, 450, 338)
    );
    request.bevel_radius = 512.0;
    assert_eq!(lens_thickness_pixels(&request), 104.0);
    request.bounds[2] = 0.25;
    assert_eq!(lens_thickness_pixels(&request), 0.125);
    assert!(backdrop_region(request, 0, 0, 2_048).is_none());
    request.bounds = [0.0, 0.0, 4096.0, 64.0];
    assert!(backdrop_region(request, 4096, 64, 2_048).is_none());
}

#[test]
fn shared_lens_capture_keeps_combined_extent_within_budget() {
    let mut extent = [1, 1];
    let wide = BackdropRegion {
        x: 0,
        y: 0,
        width: 4_096,
        height: 64,
    };
    let tall = BackdropRegion {
        x: 0,
        y: 0,
        width: 64,
        height: 4_096,
    };
    assert!(include_backdrop_region(wide, &mut extent, 4_096).is_some());
    assert_eq!(extent, [4_096, 64]);
    assert!(include_backdrop_region(tall, &mut extent, 4_096).is_none());
    assert_eq!(
        extent,
        [4_096, 64],
        "rejected capture must not grow the shared texture"
    );
    assert!(
        include_backdrop_region(
            BackdropRegion {
                height: 1_024,
                ..wide
            },
            &mut extent,
            4_096
        )
        .is_some()
    );
    assert!(
        include_backdrop_region(
            BackdropRegion {
                height: 1_025,
                ..wide
            },
            &mut extent,
            4_096
        )
        .is_none()
    );
    assert_eq!(extent, [4_096, 1_024]);
    assert!(include_backdrop_region(wide, &mut [1, 1], 2_048).is_none());
}

#[test]
fn retina_action_bar_capture_includes_refraction_margin() {
    // RustForum's 992x60 logical bar fits under 2,048 device pixels itself,
    // but its refraction margin makes the capture wider on a Retina display.
    let mut request =
        MaterialRequest::new(MaterialKind::Magnification, [496.0, 0.0, 1_984.0, 120.0]);
    request.transform[0] = 2.0;
    request.transform[4] = 2.0;
    request.bevel_radius = 32.0;
    assert!(backdrop_region(request, 2_480, 1_560, 2_048).is_none());
    let region = backdrop_region(request, 2_480, 1_560, 4_096).expect("Retina action bar capture");
    assert_eq!(
        (region.x, region.y, region.width, region.height),
        (375, 0, 2_105, 241)
    );
}

#[test]
fn lens_wire_normalization_keeps_liquid_semantics_separate() {
    let mut lens = MaterialRequest::new(MaterialKind::Magnification, [0.0, 0.0, 64.0, 32.0]);
    lens.magnification = f32::NAN;
    lens.bevel_radius = f32::INFINITY;
    let normalized = lens.normalized();
    assert_eq!(normalized.magnification, 1.5);
    assert_eq!(normalized.bevel_radius, 16.0);
    lens.magnification = -1.0;
    assert_eq!(lens.normalized().magnification, 1.0);
    lens.magnification = 99.0;
    assert_eq!(lens.normalized().magnification, 8.0);
    lens.kind = MaterialKind::Liquid;
    assert_eq!(lens.normalized().magnification, 1.0);
    lens.magnification = f32::NAN;
    assert_eq!(lens.normalized().magnification, 0.45);
    assert_eq!(lens.normalized().bevel_radius, 28.0);
}

#[cfg(feature = "wgpu")]
#[test]
fn steady_lens_frames_reuse_resources_without_gaussian_intermediates() {
    use crate::backend::*;
    use crate::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
    use aimer_utils::SyncFuture;
    let instance = ::wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&Default::default())
        .block()
        .expect("GPU adapter");
    let (device, queue) = adapter
        .request_device(&Default::default())
        .block()
        .expect("GPU device");
    let backend = crate::WgpuBackend::new(device, queue);
    let format = ::wgpu::TextureFormat::Rgba8Unorm;
    let source = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (640, 400, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::TextureBinding, TextureUsage::RenderAttachment],
    });
    let ctx = RenderContextGeneric {
        backend: &backend,
        width: 640,
        height: 400,
        is_srgb: false,
        format,
        sample_count: 1,
        source_texture: Some(&source),
    };
    let mut parent = MaterialPipeline::new(&backend, format, crate::AntiAlias::Analytic);
    let request = MaterialRequest::new(MaterialKind::Magnification, [160.0, 92.0, 320.0, 208.0]);
    parent.prepare_command(&request);
    parent.prepare(&ctx);
    let original = (
        parent.backdrop_texture.clone(),
        parent.uniform_buffer.clone(),
        parent.bind_group.clone(),
        parent.upload.as_ptr(),
    );
    for _ in 0..3 {
        parent.begin_frame();
        parent.prepare_command(&request);
        parent.prepare(&ctx);
        assert!(
            parent.blur.is_none(),
            "lens-only frames must not allocate a Gaussian intermediate"
        );
        assert_eq!(parent.backdrop_texture, original.0);
        assert_eq!(parent.uniform_buffer, original.1);
        assert_eq!(parent.bind_group, original.2);
        assert_eq!(parent.upload.as_ptr(), original.3);
    }
}
