use super::*;
use crate::{MaterialCapabilities, bounded_intermediate_extent};

#[test]
fn ipad_backdrop_capture_uses_hardware_width_and_device_scale() {
    let mut request =
        MaterialRequest::new(MaterialKind::BackdropFilter, [500.0, 0.0, 2232.0, 120.0]);
    request.blur_radius = 4.0;
    request.transform[0] = 2.0;
    request.transform[4] = 2.0;
    let region = backdrop_region(request, 2732, 2048, 4096).expect("wide iPad toolbar capture");
    assert_eq!(
        (region.x, region.y, region.width, region.height),
        (492, 0, 2240, 128)
    );
    assert!(
        backdrop_region(request, 2732, 2048, 2048).is_none(),
        "respect the actual hardware limit"
    );
    let capabilities = MaterialCapabilities {
        max_surface_dimension: 4096,
        ..Default::default()
    };
    for quality in [1, 2, 4] {
        request.quality = quality;
        let extent =
            bounded_intermediate_extent(request, capabilities).expect("wide shallow blur fits");
        assert_eq!(
            (extent.width, extent.height),
            (2232, 120),
            "Gaussian passes use full-resolution textures"
        );
        assert_eq!(
            plan_material(request, capabilities).path,
            MaterialRenderPath::Gpu
        );
    }
}

#[test]
fn wide_backdrop_retains_pixel_and_dimension_guards() {
    let capabilities = MaterialCapabilities {
        max_surface_dimension: 8192,
        ..Default::default()
    };
    let mut request =
        MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 4096.0, 1024.0]);
    request.blur_radius = 0.0;
    assert!(
        backdrop_region(request, 4096, 1024, 8192).is_some(),
        "exact pixel budget"
    );
    assert!(bounded_intermediate_extent(request, capabilities).is_some());
    request.bounds[3] += 1.0;
    assert!(
        backdrop_region(request, 4096, 1025, 8192).is_none(),
        "capture exceeds pixel budget"
    );
    assert!(
        bounded_intermediate_extent(request, capabilities).is_none(),
        "plan must match full-resolution allocation"
    );
    request.bounds = [0.0, 0.0, 8193.0, 1.0];
    assert!(
        backdrop_region(request, 8193, 1, 8192).is_none(),
        "hardware dimension exceeded"
    );
    assert!(bounded_intermediate_extent(request, capabilities).is_none());
    assert!(backdrop_region(request, 0, 1024, 8192).is_none());
    assert!(backdrop_region(request, 8192, 0, 8192).is_none());
    request.kind = MaterialKind::Glass;
    request.bounds = [0.0, 0.0, 4096.0, 32.0];
    assert!(
        backdrop_region(request, 8192, 1024, 8192).is_none(),
        "glass keeps its existing capture cap"
    );
}

#[test]
fn backdrop_capture_reads_group_local_regions_at_the_groups_frame_offset() {
    let region = BackdropRegion { x: 12, y: 30, width: 64, height: 32 };
    // The frame itself: regions are already frame coordinates.
    assert_eq!(backdrop_source_origin(region, (0, 0)), (12, 30));
    // An opacity group at (188, 4): the same local region is further into the
    // frame, not at its top-left where unrelated content (a sidebar) lives.
    assert_eq!(backdrop_source_origin(region, (188, 4)), (200, 34));
}

#[test]
fn a_confined_blur_captures_only_inside_its_bounds_and_clip() {
    let mut request =
        MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 400.0, 600.0]);
    request.blur_radius = 12.0;
    request.clip = Some(crate::MaterialClip { rect: [40.0, 100.0, 200.0, 150.0], corner_radii: [8.0; 4] });
    let region = |request| {
        let region = backdrop_region(request, 400, 600, 4096).expect("capture");
        (region.x, region.y, region.width, region.height)
    };
    // By default the blur reads the whole host: its bounds, widened by the radius.
    assert_eq!(region(request), (0, 0, 400, 600));

    request.confine_backdrop = true;
    // Confined, it reads the window and nothing outside it, so the shader's clamp
    // to the capture edge is the window's edge.
    assert_eq!(region(request), (40, 100, 200, 150));

    // Without a clip the bounds are the limit, with no widening by the radius.
    request.clip = None;
    request.bounds = [50.0, 60.0, 100.0, 80.0];
    assert_eq!(region(request), (50, 60, 100, 80));
}

#[test]
fn a_confined_blur_rounds_its_capture_outward_and_stays_on_the_frame() {
    let mut request =
        MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 400.0, 600.0]);
    request.confine_backdrop = true;
    request.clip = Some(crate::MaterialClip { rect: [10.4, 20.5, 100.2, 50.0], corner_radii: [0.0; 4] });
    let region = backdrop_region(request, 400, 600, 4096).unwrap();
    assert_eq!((region.x, region.y, region.width, region.height), (10, 20, 101, 51));

    // A window part way off the frame is cut to it; one wholly off it has nothing to read.
    request.clip = Some(crate::MaterialClip { rect: [350.0, 550.0, 200.0, 200.0], corner_radii: [0.0; 4] });
    let region = backdrop_region(request, 400, 600, 4096).unwrap();
    assert_eq!((region.x, region.y, region.width, region.height), (350, 550, 50, 50));
    request.clip = Some(crate::MaterialClip { rect: [500.0, 0.0, 50.0, 50.0], corner_radii: [0.0; 4] });
    assert!(backdrop_region(request, 400, 600, 4096).is_none());
}

#[test]
fn only_a_backdrop_filter_can_be_confined() {
    let mut request = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 100.0, 100.0]);
    request.blur_radius = 10.0;
    let widened = backdrop_region(request, 400, 400, 4096).unwrap();
    request.confine_backdrop = true;
    let confined = backdrop_region(request, 400, 400, 4096).unwrap();
    assert_eq!((widened.x, widened.y, widened.width, widened.height),
        (confined.x, confined.y, confined.width, confined.height), "glass refracts past its edge");
}

#[test]
fn the_confinement_rides_the_packet_and_unknown_flags_are_refused() {
    let mut request = MaterialRequest::new(MaterialKind::BackdropFilter, [0.0, 0.0, 64.0, 64.0]);
    assert!(!MaterialRequest::decode(&request.encode_for_test()).unwrap().confine_backdrop);
    request.confine_backdrop = true;
    let mut packet = request.encode_for_test();
    assert!(MaterialRequest::decode(&packet).unwrap().confine_backdrop);
    // Bit 0 is the only flag defined; another is not silently dropped.
    packet[13] = 0b10;
    assert_eq!(MaterialRequest::decode(&packet), Err(crate::MaterialDecodeError::NonZeroReserved));
    packet[13] = 0;
    packet[14] = 1;
    assert_eq!(MaterialRequest::decode(&packet), Err(crate::MaterialDecodeError::NonZeroReserved));
}
