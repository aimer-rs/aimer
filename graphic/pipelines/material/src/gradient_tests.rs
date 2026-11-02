use super::*;

fn ramp(shape: GradientShape, stops: &[(f32, [u8; 4])]) -> MaterialGradient {
    let mut gradient = MaterialGradient {
        shape,
        geometry: [0.0, 0.0, 80.0, 0.0],
        stop_count: stops.len() as u8,
        ..MaterialGradient::default()
    };
    for (index, (offset, color)) in stops.iter().enumerate() {
        gradient.offsets[index] = *offset;
        gradient.colors[index] = MaterialGradient::pack_color(*color);
    }
    gradient
}

fn gradient_request(gradient: MaterialGradient) -> MaterialRequest {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [4.0, 6.0, 80.0, 40.0]);
    request.gradient = gradient;
    request
}

#[test]
fn gradient_packets_round_trip_in_their_own_versioned_length() {
    let request = gradient_request(ramp(
        GradientShape::Radial,
        &[
            (0.0, [255, 0, 0, 255]),
            (0.5, [0, 255, 0, 128]),
            (1.0, [0, 0, 255, 255]),
        ],
    ));
    let bytes = request.encode_for_test();
    assert_eq!(bytes.len(), MATERIAL_GRADIENT_PACKET_LEN);
    assert_eq!(bytes.len(), 256 + GRADIENT_PACKET_EXTENSION_LEN);
    assert_eq!(bytes[4], MATERIAL_GRADIENT_PACKET_VERSION);
    assert_eq!(bytes[5], MaterialKind::Gradient as u8);
    assert_eq!(MaterialRequest::decode(&bytes), Ok(request.normalized()));
}

#[test]
fn other_kinds_keep_the_version_three_packet_and_an_empty_ramp() {
    for kind in [
        MaterialKind::Glass,
        MaterialKind::Liquid,
        MaterialKind::BackdropFilter,
        MaterialKind::Magnification,
        MaterialKind::LightEdge,
    ] {
        let mut request = MaterialRequest::new(kind, [0.0, 0.0, 20.0, 10.0]);
        // A ramp on a kind that ignores it must not change what is encoded.
        request.gradient = ramp(GradientShape::Linear, &[(0.0, [1, 2, 3, 4])]);
        let bytes = request.encode_for_test();
        assert_eq!(bytes.len(), MATERIAL_PACKET_LEN, "{kind:?}");
        assert_eq!(bytes[4], MATERIAL_PACKET_VERSION, "{kind:?}");
        assert_eq!(
            MaterialRequest::decode(&bytes).unwrap().gradient,
            MaterialGradient::default()
        );
    }
}

#[test]
fn gradient_decoding_rejects_mismatched_versions_and_malformed_ramps() {
    let request = gradient_request(ramp(
        GradientShape::Linear,
        &[(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])],
    ));
    let bytes = request.encode_for_test();

    let mut old_version = bytes[..MATERIAL_PACKET_LEN].to_vec();
    old_version[4] = MATERIAL_PACKET_VERSION;
    assert_eq!(
        MaterialRequest::decode(&old_version),
        Err(MaterialDecodeError::UnsupportedKind),
        "a ramp needs the version that carries one"
    );

    let mut other_kind = bytes.clone();
    other_kind[5] = MaterialKind::LightEdge as u8;
    assert_eq!(
        MaterialRequest::decode(&other_kind),
        Err(MaterialDecodeError::UnsupportedKind),
        "version 4 carries only ramps"
    );

    assert!(matches!(
        MaterialRequest::decode(&bytes[..bytes.len() - 1]),
        Err(MaterialDecodeError::InvalidLength { .. })
    ));

    let mut too_many = bytes.clone();
    too_many[MATERIAL_PACKET_LEN + 1] = MAX_GRADIENT_STOPS as u8 + 1;
    assert_eq!(
        MaterialRequest::decode(&too_many),
        Err(MaterialDecodeError::InvalidGradient)
    );

    let mut reserved = bytes.clone();
    reserved[MATERIAL_PACKET_LEN + 2] = 1;
    assert_eq!(
        MaterialRequest::decode(&reserved),
        Err(MaterialDecodeError::NonZeroReserved)
    );

    let mut shape = bytes;
    shape[MATERIAL_PACKET_LEN] = 3;
    assert_eq!(
        MaterialRequest::decode(&shape),
        Err(MaterialDecodeError::UnsupportedKind)
    );
}

#[test]
fn gradient_normalization_orders_offsets_and_sanitizes_values() {
    let mut gradient = ramp(
        GradientShape::Radial,
        &[
            (0.6, [10, 20, 30, 40]),
            (0.2, [11, 21, 31, 41]),
            (f32::NAN, [12, 22, 32, 42]),
            (7.0, [13, 23, 33, 43]),
        ],
    );
    gradient.geometry = [f32::NAN, 5.0, -3.0, f32::INFINITY];
    gradient.colors[5] = 0xdead_beef;
    gradient.offsets[6] = 0.9;
    let gradient = gradient.normalized();

    assert_eq!(gradient.offsets[..4], [0.6, 0.6, 0.6, 1.0]);
    assert_eq!(gradient.geometry, [0.0, 5.0, 0.0, 0.0]);
    assert_eq!(gradient.colors[5], 0, "unused slots are zeroed");
    assert_eq!(gradient.offsets[6], 0.0);

    let mut crowded = ramp(GradientShape::Linear, &[(0.0, [0; 4])]);
    crowded.stop_count = 200;
    assert_eq!(crowded.normalized().stop_count, MAX_GRADIENT_STOPS as u8);
}

#[test]
fn gradient_needs_no_backdrop_or_intermediate_budget() {
    let capabilities = MaterialCapabilities {
        backdrop_sampling: false,
        distortion_sampling: false,
        max_surface_dimension: 0,
        max_intermediate_pixels: 0,
        max_samples: 0,
    };
    let mut request = gradient_request(ramp(
        GradientShape::Linear,
        &[(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])],
    ));
    request.bounds = [0.0, 0.0, 8000.0, 40.0];
    request.opacity = 1.0;
    let plan = plan_material(request, capabilities);
    assert_eq!(plan.path, MaterialRenderPath::Gpu);
    assert_eq!(plan.intermediate, None);
    assert_eq!(plan.fallback_reason, None);
    assert_eq!(bounded_intermediate_extent(request, capabilities), None);
    assert!(!MaterialKind::Gradient.samples_backdrop());
    assert!(!MaterialKind::LightEdge.samples_backdrop());
    assert!(MaterialKind::Glass.samples_backdrop());
}

#[test]
fn an_empty_or_invisible_gradient_is_skipped() {
    let capabilities = MaterialCapabilities::default();
    let ramp = ramp(
        GradientShape::Linear,
        &[(0.0, [0, 0, 0, 255]), (1.0, [255, 255, 255, 255])],
    );
    let mut request = gradient_request(ramp);
    request.opacity = 1.0;
    assert_eq!(plan_material(request, capabilities).path, MaterialRenderPath::Gpu);

    let mut empty = request;
    empty.gradient.stop_count = 0;
    assert_eq!(plan_material(empty, capabilities).path, MaterialRenderPath::Skip);
    let mut clear = request;
    clear.opacity = 0.0;
    assert_eq!(plan_material(clear, capabilities).path, MaterialRenderPath::Skip);
    let mut flat = request;
    flat.bounds[3] = 0.0;
    assert_eq!(plan_material(flat, capabilities).path, MaterialRenderPath::Skip);
}
