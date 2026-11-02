use super::*;

#[test]
fn light_edge_needs_no_backdrop_or_intermediate_budget() {
    let capabilities = MaterialCapabilities {
        backdrop_sampling: false,
        distortion_sampling: false,
        max_surface_dimension: 0,
        max_intermediate_pixels: 0,
        max_samples: 0,
    };
    let mut edge = MaterialRequest::new(MaterialKind::LightEdge, [0.0, 0.0, 8000.0, 40.0]);
    let plan = plan_material(edge, capabilities);
    assert_eq!(plan.path, MaterialRenderPath::Gpu);
    assert_eq!(plan.intermediate, None);
    assert_eq!(plan.fallback_reason, None);
    assert_eq!(plan.phase, 0.0);
    assert_eq!(plan.distortion_strength, 0.0);
    assert_eq!(bounded_intermediate_extent(edge, capabilities), None);
    for disabled in [0, 1, 2, 3, 4] {
        edge = MaterialRequest::new(MaterialKind::LightEdge, [0.0, 0.0, 80.0, 40.0]);
        match disabled {
            0 => edge.border_width = 0.0,
            1 => edge.specular_highlight = 0.0,
            2 => edge.border_color[3] = 0.0,
            3 => edge.opacity = 0.0,
            _ => edge.bounds[2] = 0.0,
        }
        assert_eq!(
            plan_material(edge, capabilities).path,
            MaterialRenderPath::Skip
        );
    }
}

#[test]
fn light_angle_packets_round_trip_normalize_and_preserve_legacy_decoding() {
    let mut request = MaterialRequest::new(MaterialKind::LightEdge, [0.0, 0.0, 80.0, 40.0]);
    request.light_angle = 45.0;
    let bytes = packet(request);
    assert_eq!(bytes.len(), 256);
    assert_eq!(bytes[4], 3);
    assert_eq!(MaterialRequest::decode(&bytes), Ok(request));
    for (value, expected) in [
        (f32::NAN, 225.0),
        (f32::INFINITY, 225.0),
        (-135.0, 225.0),
        (450.0, 90.0),
    ] {
        let mut invalid = bytes.clone();
        invalid[252..256].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            MaterialRequest::decode(&invalid).unwrap().light_angle,
            expected
        );
    }
    for kind in [
        MaterialKind::Glass,
        MaterialKind::Liquid,
        MaterialKind::BackdropFilter,
        MaterialKind::Magnification,
    ] {
        let original = MaterialRequest::new(kind, [1.0, 2.0, 80.0, 40.0]);
        for (version, length) in [(1, 244), (2, 252)] {
            let mut bytes = packet(original);
            bytes.truncate(length);
            bytes[4] = version;
            assert_eq!(MaterialRequest::decode(&bytes), Ok(original));
        }
    }
}

#[test]
fn magnification_style_packet_v2_is_accepted() {
    let mut bytes = packet(MaterialRequest::new(
        MaterialKind::Magnification,
        [3.0, 4.0, 80.0, 40.0],
    ));
    bytes.truncate(244);
    bytes[4] = 2;
    bytes.extend_from_slice(&43u32.to_le_bytes());
    bytes.extend_from_slice(&0.6f32.to_le_bytes());
    let decoded = MaterialRequest::decode(&bytes)
        .expect("version 2 carries magnification styles and bulge strength");
    assert_eq!(decoded.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(
        decoded.magnification_style,
        MagnificationStyle::UNIFORM
            | MagnificationStyle::BULGE
            | MagnificationStyle::HORIZONTAL
            | MagnificationStyle::LEFT
    );
    assert_eq!(decoded.bulge_strength, 0.6);
}

fn packet(request: MaterialRequest) -> Vec<u8> {
    let request = request.normalized();
    let mut bytes = Vec::with_capacity(MATERIAL_PACKET_LEN);
    bytes.extend_from_slice(b"AIMR");
    bytes.push(MATERIAL_PACKET_VERSION);
    bytes.push(request.kind as u8);
    bytes.push(request.motion_policy as u8);
    bytes.push(request.quality);
    bytes.extend_from_slice(&request.z_order.to_le_bytes());
    bytes.push(request.clip.is_some() as u8);
    bytes.extend_from_slice(&[0, 0, 0]);
    let clip = request.clip.unwrap_or(MaterialClip {
        rect: [0.0; 4],
        corner_radii: [0.0; 4],
    });
    for values in [
        request.bounds.as_slice(),
        request.transform.as_slice(),
        clip.rect.as_slice(),
        clip.corner_radii.as_slice(),
        request.tint.as_slice(),
        core::slice::from_ref(&request.opacity),
        core::slice::from_ref(&request.blur_radius),
        core::slice::from_ref(&request.saturation),
        core::slice::from_ref(&request.brightness),
        core::slice::from_ref(&request.contrast),
        request.border_color.as_slice(),
        core::slice::from_ref(&request.border_width),
        request.corner_radii.as_slice(),
        request.shadow_color.as_slice(),
        core::slice::from_ref(&request.shadow_blur),
        core::slice::from_ref(&request.elevation),
        core::slice::from_ref(&request.distortion_strength),
        core::slice::from_ref(&request.edge_lighting),
        core::slice::from_ref(&request.specular_highlight),
        core::slice::from_ref(&request.animation_speed),
        core::slice::from_ref(&request.animation_time),
        core::slice::from_ref(&request.interaction),
        core::slice::from_ref(&request.blob_amount),
        core::slice::from_ref(&request.blob_seed),
        core::slice::from_ref(&request.magnification),
        core::slice::from_ref(&request.tip_pull),
        core::slice::from_ref(&request.chromatic_aberration),
        core::slice::from_ref(&request.bevel_radius),
    ] {
        for value in values {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
    }
    bytes.extend_from_slice(&request.magnification_style.bits().to_le_bytes());
    bytes.extend_from_slice(&request.bulge_strength.to_le_bytes());
    bytes.extend_from_slice(&request.light_angle.to_le_bytes());
    bytes
}

#[test]
fn magnification_style_packets_preserve_legacy_and_reject_unknown_bits() {
    for kind in [
        MaterialKind::Magnification,
        MaterialKind::Glass,
        MaterialKind::Liquid,
        MaterialKind::BackdropFilter,
    ] {
        let request = MaterialRequest::new(kind, [1.0, 2.0, 80.0, 40.0]);
        let mut bytes = packet(request);
        assert_eq!(bytes.len(), 256);
        bytes.truncate(244);
        bytes[4] = 1;
        assert_eq!(MaterialRequest::decode(&bytes), Ok(request));
        bytes[4] = 2;
        assert!(matches!(
            MaterialRequest::decode(&bytes),
            Err(MaterialDecodeError::InvalidLength { expected: 252, .. })
        ));
    }
    let mut bytes = packet(MaterialRequest::new(
        MaterialKind::Magnification,
        [0.0, 0.0, 80.0, 40.0],
    ));
    bytes[244..248].copy_from_slice(&(1u32 << 9).to_le_bytes());
    assert_eq!(
        MaterialRequest::decode(&bytes),
        Err(MaterialDecodeError::InvalidMagnificationStyle)
    );
    bytes[244..248].copy_from_slice(&MagnificationStyle::ALL.bits().to_le_bytes());
    for (value, expected) in [
        (f32::NAN, 0.25),
        (f32::INFINITY, 0.25),
        (-1.0, 0.0),
        (2.0, 1.0),
    ] {
        bytes[248..252].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            MaterialRequest::decode(&bytes).unwrap().bulge_strength,
            expected
        );
    }
    bytes[13] = 1;
    assert_eq!(
        MaterialRequest::decode(&bytes),
        Err(MaterialDecodeError::NonZeroReserved)
    );
}

#[test]
fn packet_decoder_normalizes_values_and_preserves_geometry() {
    let mut request = MaterialRequest::new(MaterialKind::Liquid, [2.0, 3.0, 80.0, 40.0]);
    request.opacity = f32::NAN;
    request.tint = [f32::INFINITY, -1.0, 0.5, 0.25];
    request.motion_policy = MaterialMotionPolicy::Reduced;
    request.distortion_strength = 1.0;
    request.animation_speed = 4.0;
    let decoded = MaterialRequest::decode(&packet(request)).unwrap();

    assert_eq!(decoded.bounds, [2.0, 3.0, 80.0, 40.0]);
    assert_eq!(decoded.opacity, 0.72);
    assert_eq!(decoded.tint, [0.0, 0.0, 0.5, 0.25]);
    assert_eq!(decoded.distortion_strength, 0.0);
    assert_eq!(decoded.animation_speed, 0.0);
}

#[test]
fn liquid_shape_fields_round_trip_and_clamp() {
    let mut request = MaterialRequest::new(MaterialKind::Liquid, [0.0, 0.0, 40.0, 40.0]);
    request.blob_amount = 0.6;
    request.blob_seed = 0.9;
    request.magnification = 0.75;
    request.tip_pull = 0.4;
    request.chromatic_aberration = 0.65;
    request.bevel_radius = 40.0;
    let decoded = MaterialRequest::decode(&packet(request)).unwrap();
    assert_eq!(decoded.blob_amount, 0.6);
    assert_eq!(decoded.blob_seed, 0.9);
    assert_eq!(decoded.magnification, 0.75);
    assert_eq!(decoded.tip_pull, 0.4);
    assert_eq!(decoded.chromatic_aberration, 0.65);
    assert_eq!(decoded.bevel_radius, 40.0);

    let mut out_of_range = request;
    out_of_range.blob_amount = f32::NAN;
    out_of_range.blob_seed = -1.0;
    out_of_range.magnification = 5.0;
    out_of_range.tip_pull = -0.5;
    out_of_range.chromatic_aberration = 5.0;
    out_of_range.bevel_radius = f32::NAN;
    let clamped = MaterialRequest::decode(&packet(out_of_range)).unwrap();
    assert_eq!(clamped.blob_amount, 0.0);
    assert_eq!(clamped.blob_seed, 0.0);
    assert_eq!(clamped.magnification, 1.0);
    assert_eq!(clamped.tip_pull, 0.0);
    assert_eq!(clamped.chromatic_aberration, 1.0);
    assert_eq!(clamped.bevel_radius, 28.0);
}

#[test]
fn decoder_rejects_wrong_schema_headers() {
    let request = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 10.0, 10.0]);
    let mut bytes = packet(request);
    bytes[0] = b'X';
    assert_eq!(
        MaterialRequest::decode(&bytes),
        Err(MaterialDecodeError::InvalidMagic)
    );
    bytes[0] = b'A';
    bytes[4] = 99;
    assert_eq!(
        MaterialRequest::decode(&bytes),
        Err(MaterialDecodeError::UnsupportedVersion)
    );
}

#[test]
fn planning_skips_zero_size_and_falls_back_without_backdrop_sampling() {
    let zero = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 0.0, 10.0]);
    assert_eq!(
        plan_material(zero, MaterialCapabilities::default()).path,
        MaterialRenderPath::Skip
    );

    let request = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 80.0, 40.0]);
    let capabilities = MaterialCapabilities {
        backdrop_sampling: false,
        ..MaterialCapabilities::default()
    };
    assert_eq!(
        plan_material(request, capabilities).path,
        MaterialRenderPath::Fallback
    );
}

#[test]
fn intermediate_extent_is_downsampled_and_bounded() {
    let request = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 1_000.0, 500.0]);
    let mut low = request;
    low.quality = 1;
    assert_eq!(
        bounded_intermediate_extent(low, MaterialCapabilities::default()),
        Some(IntermediateExtent {
            width: 250,
            height: 125,
            bytes: 125_000,
        })
    );

    let huge = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 100_000.0, 100_000.0]);
    assert!(bounded_intermediate_extent(huge, MaterialCapabilities::default()).is_none());
}

#[test]
fn liquid_distortion_requires_capability_but_glass_does_not() {
    let capabilities = MaterialCapabilities {
        distortion_sampling: false,
        ..MaterialCapabilities::default()
    };
    let liquid = MaterialRequest::new(MaterialKind::Liquid, [0.0, 0.0, 20.0, 20.0]);
    assert_eq!(
        plan_material(liquid, capabilities).path,
        MaterialRenderPath::Fallback
    );
    let glass = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 20.0, 20.0]);
    assert_eq!(
        plan_material(glass, capabilities).path,
        MaterialRenderPath::Gpu
    );
}

#[test]
fn material_planning_reports_why_the_explicit_fallback_was_selected() {
    let request = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 20.0, 20.0]);
    let capabilities = MaterialCapabilities {
        backdrop_sampling: false,
        ..MaterialCapabilities::default()
    };

    let plan = plan_material(request, capabilities);

    assert_eq!(plan.path, MaterialRenderPath::Fallback);
    assert_eq!(
        plan.fallback_reason,
        Some(MaterialFallbackReason::BackdropSamplingUnavailable)
    );
}

#[test]
fn material_planning_reports_an_intermediate_budget_fallback() {
    let request = MaterialRequest::new(MaterialKind::Liquid, [0.0, 0.0, 200.0, 100.0]);
    let capabilities = MaterialCapabilities {
        max_intermediate_pixels: 1,
        ..MaterialCapabilities::default()
    };

    let plan = plan_material(request, capabilities);

    assert_eq!(plan.path, MaterialRenderPath::Fallback);
    assert_eq!(
        plan.fallback_reason,
        Some(MaterialFallbackReason::IntermediateBudgetExceeded)
    );
}

#[test]
fn fallback_multiplies_alpha_and_drops_dynamic_work() {
    let mut request = MaterialRequest::new(MaterialKind::Liquid, [0.0, 0.0, 10.0, 10.0]);
    request.opacity = 0.5;
    request.tint = [0.2, 0.3, 0.4, 0.8];
    let fallback = request.fallback();
    assert_eq!(fallback.fill_color, [0.2, 0.3, 0.4, 0.4]);
    assert_eq!(fallback.z_order, 0);
}

#[test]
fn equal_z_order_is_stable_and_different_orders_are_sorted() {
    let mut requests = [
        MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 1.0, 1.0]),
        MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 1.0, 1.0]),
        MaterialRequest::new(MaterialKind::Liquid, [0.0, 0.0, 1.0, 1.0]),
    ];
    requests[0].z_order = 3;
    requests[1].z_order = 1;
    requests[2].z_order = 2;
    order_requests(&mut requests);
    assert_eq!(requests[0].z_order, 1);
    assert_eq!(requests[1].z_order, 2);
    assert_eq!(requests[2].z_order, 3);
}

#[test]
fn shader_contains_the_material_entry_points() {
    let source = MaterialShader::source();
    assert!(source.contains("@vertex"));
    assert!(source.contains("@fragment"));
    assert!(source.contains("backdrop"));
}

#[test]
fn material_shader_contains_both_reference_surface_layers() {
    let source = MaterialShader::source();

    assert!(source.contains("glass_glow"));
    assert!(source.contains("glass_rim"));
    assert!(source.contains("liquid_shape_distance"));
    assert!(source.contains("liquid_rim"));
    assert!(source.contains("liquid_sheen"));
    assert!(source.contains("liquid_bevel_normal"));
    assert!(source.contains("liquid_bevel_refract_offset"));
    assert!(source.contains("liquid_shape_normal"));
    assert!(source.contains("liquid_chromatic_sample"));
    assert!(source.contains("adaptive_tint"));
}

#[test]
fn glass_shader_uses_the_public_lighting_controls_and_user_tint() {
    let source = MaterialShader::source();

    assert!(source.contains("glass_frosted_base"));
    assert!(source.contains("material.tint.rgb"));
    assert!(source.contains("material.detail.x"));
    assert!(source.contains("material.light.w"));
    assert!(source.contains("let edge_color"));
}

#[test]
fn material_shader_samples_a_pane_local_screen_space_backdrop() {
    let source = MaterialShader::source();

    assert!(source.contains("sample_backdrop"));
    assert!(source.contains("frosted_backdrop"));
    assert!(source.contains("input.pixel_pos"));
    assert!(source.contains("material.viewport.w"));
    assert!(source.contains("material.backdrop_rect.xy"));
    assert!(source.contains("material.backdrop_rect.zw"));
}

#[test]
fn glass_shader_uses_a_bounded_nine_tap_frosted_kernel() {
    let source = MaterialShader::source();

    let kernel = source
        .split("fn frosted_backdrop")
        .nth(1)
        .and_then(|source| source.split("fn backdrop_to_material_rgb").next())
        .expect("frosted backdrop function");
    let frosted_path = kernel
        .split("let reed")
        .nth(1)
        .expect("non-zero frosted path");
    assert_eq!(frosted_path.matches("sample_backdrop(").count(), 9);
    assert!(kernel.contains("reed"));
    assert!(kernel.contains("ripple"));
}

#[test]
fn glass_shader_skips_unneeded_backdrop_reads() {
    let source = MaterialShader::source();
    let kernel = source
        .split("fn frosted_backdrop")
        .nth(1)
        .and_then(|source| source.split("fn backdrop_to_material_rgb").next())
        .expect("frosted backdrop function");
    let fragment = source
        .split("fn fs_main")
        .nth(1)
        .expect("fragment function");

    assert!(kernel.contains("if radius <= 0.0"));
    let liquid_branch = fragment
        .find("if material.effect.x > 0.5")
        .expect("liquid-only branch");
    let refracted_sample = fragment
        .find("let refracted")
        .expect("liquid refraction sample");
    assert!(liquid_branch < refracted_sample);
}

// The largest iPad (13-inch Pro) in physical pixels. A capture is clamped to
// the viewport, so this is the biggest intermediate a material can ask for.
const LARGEST_IPAD: (u32, u32) = (2_064, 2_752);

#[test]
fn the_default_budget_was_too_small_for_a_full_screen_capture_on_the_largest_ipad() {
    let pixels = u64::from(LARGEST_IPAD.0) * u64::from(LARGEST_IPAD.1);
    assert!(intermediate_pixel_budget(false) < pixels);
    assert!(intermediate_dimension_budget(false) < LARGEST_IPAD.1);
}

#[test]
fn the_ios_budget_holds_a_full_screen_capture_on_the_largest_ipad() {
    let pixels = u64::from(LARGEST_IPAD.0) * u64::from(LARGEST_IPAD.1);
    assert!(intermediate_pixel_budget(true) >= pixels);
    assert!(intermediate_dimension_budget(true) >= LARGEST_IPAD.1);
}

#[test]
fn other_platforms_keep_the_conservative_budget() {
    assert_eq!(intermediate_pixel_budget(false), 4 * 1024 * 1024);
    assert_eq!(intermediate_dimension_budget(false), 2_048);
}

#[test]
fn default_capabilities_follow_the_platform_budget() {
    let capabilities = MaterialCapabilities::default();
    assert_eq!(capabilities.max_intermediate_pixels, MAX_INTERMEDIATE_PIXELS);
    assert_eq!(capabilities.max_surface_dimension, MAX_INTERMEDIATE_DIMENSION);
    assert_eq!(
        MAX_INTERMEDIATE_PIXELS,
        intermediate_pixel_budget(cfg!(target_os = "ios"))
    );
}
