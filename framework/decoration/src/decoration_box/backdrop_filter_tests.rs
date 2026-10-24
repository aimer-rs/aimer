//! A backdrop filter rounds itself to the box shape declared before it.
//!
//! `BorderRadius` and a `BoxDecoration` radius declare the shape of the box on the
//! canvas, and the material layers that follow take their corners from it. The
//! filter used to ignore it, so glass built as `[BorderRadius, BackdropFilter]`
//! blurred a plain rectangle behind rounded content.

use super::test_support::stacked;
use crate::{BackdropFilter, BorderRadius, BoxBorder, BoxDecoration};
use aimer_color::prelude::Color;
use aimer_cupid::material::MaterialKind;

#[test]
fn a_backdrop_filter_without_a_shape_layer_has_square_corners() {
    assert_eq!(stacked(&[&BackdropFilter::new()]).corner_radii, [0.0; 4]);
    // A shape declared after the filter cannot reach back and reshape it.
    let late = stacked(&[&BackdropFilter::new(), &BorderRadius::from(12.0)]);
    assert_eq!(late.corner_radii, [0.0; 4]);
}

#[test]
fn a_backdrop_filter_takes_its_corners_from_preceding_shape_layers() {
    let glass = stacked(&[&BorderRadius::from(12.0), &BackdropFilter::new()]);
    assert_eq!(glass.kind, MaterialKind::BackdropFilter);
    assert_eq!(glass.corner_radii, [12.0; 4]);

    let uneven = BorderRadius::new()
        .top_left(4.0)
        .top_right(8.0)
        .bottom_right(12.0)
        .bottom_left(16.0);
    assert_eq!(
        stacked(&[&uneven, &BackdropFilter::new()]).corner_radii,
        [4.0, 8.0, 12.0, 16.0]
    );

    let shape = BoxDecoration::new().border_radius(10.0);
    assert_eq!(
        stacked(&[&shape, &BackdropFilter::new()]).corner_radii,
        [10.0; 4]
    );
}

#[test]
fn layers_that_declare_no_shape_leave_the_inherited_corners_alone() {
    let glass = stacked(&[
        &BorderRadius::from(12.0),
        &Color::BLUE,
        &BoxDecoration::new(),
        &BoxBorder::default(),
        &BackdropFilter::new(),
    ]);
    assert_eq!(glass.corner_radii, [12.0; 4]);
}

#[test]
fn a_backdrop_filter_sanitizes_the_declared_corners() {
    let huge = stacked(&[&BorderRadius::from(5000.0), &BackdropFilter::new()]);
    assert_eq!(
        huge.corner_radii,
        [aimer_canvas::MaterialDrawRequest::MAX_CORNER_RADIUS; 4],
        "radii are capped; the renderer fits them to the shorter side"
    );
    for invalid in [-4.0, f32::NAN, f32::NEG_INFINITY, f32::INFINITY] {
        let glass = stacked(&[&BorderRadius::from(invalid), &BackdropFilter::new()]);
        assert_eq!(glass.corner_radii, [0.0; 4], "{invalid}");
    }
}

/// Only the corners change: a rounded filter still adds no tint, border or shadow
/// of its own, so it stays a plain filter of what is behind the box.
#[test]
fn a_rounded_backdrop_filter_stays_a_plain_filter() {
    let glass = stacked(&[&BorderRadius::from(12.0), &BackdropFilter::new().blur_radius(8.0)]);
    assert_eq!(glass.blur_radius, 8.0);
    assert_eq!(glass.tint, [0.0; 4]);
    assert_eq!(glass.border_width, 0.0);
    assert_eq!(glass.shadow_color, [0.0; 4]);
    assert_eq!(glass.edge_lighting, 0.0);
}
