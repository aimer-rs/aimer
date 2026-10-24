use super::resolve_material_request;
use crate::pipeline::material::{GradientShape, MaterialGradient, MaterialKind, MaterialRequest};
use crate::utilities::Mat3;

fn ramp(shape: GradientShape, geometry: [f32; 4]) -> MaterialRequest {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [0.0, 0.0, 40.0, 20.0]);
    request.gradient = MaterialGradient {
        shape,
        geometry,
        stop_count: 2,
        ..MaterialGradient::default()
    };
    request.gradient.colors[1] = u32::MAX;
    request.gradient.offsets[1] = 1.0;
    request
}

#[test]
fn gradient_geometry_scales_with_the_transform_but_a_conic_angle_does_not() {
    let scale = Mat3::scale(2.0, 2.0);
    let linear = ramp(GradientShape::Linear, [1.0, 2.0, 30.0, 40.0]);
    let resolved = resolve_material_request(linear, &scale, (2.0, 2.0), None, 1.0);
    assert_eq!(resolved.gradient.geometry, [2.0, 4.0, 60.0, 80.0]);

    let radial = ramp(GradientShape::Radial, [5.0, 6.0, 7.0, 8.0]);
    let resolved = resolve_material_request(radial, &scale, (2.0, 2.0), None, 1.0);
    assert_eq!(resolved.gradient.geometry, [10.0, 12.0, 14.0, 16.0]);

    let conic = ramp(GradientShape::Conic, [10.0, 12.0, 1.5, 0.0]);
    let resolved = resolve_material_request(conic, &scale, (2.0, 2.0), None, 1.0);
    assert_eq!(resolved.gradient.geometry, [20.0, 24.0, 1.5, 0.0]);
}

#[test]
fn other_kinds_leave_their_empty_ramp_alone() {
    let scale = Mat3::scale(3.0, 3.0);
    let glass = MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, 40.0, 20.0]);
    let resolved = resolve_material_request(glass, &scale, (3.0, 3.0), None, 1.0);
    assert_eq!(resolved.gradient, MaterialGradient::default());
}
