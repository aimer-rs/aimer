use super::*;
use crate::pipeline::material::{MaterialCapabilities, bounded_intermediate_extent};

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
