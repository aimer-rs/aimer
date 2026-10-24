use super::test_support::{BOX, stacked, stacked_in};
use crate::{BorderRadius, Decoration, Gradient};
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::material::{GradientShape, MaterialKind, MaterialRequest};

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

fn request(gradient: Gradient) -> MaterialRequest {
    stacked(&[&gradient])
}

fn close(actual: [f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.001,
            "{actual:?} vs {expected:?}"
        );
    }
}

fn packed(color: [u8; 4]) -> u32 {
    u32::from_le_bytes(color)
}

#[test]
fn a_gradient_records_one_ramp_material_in_logical_coordinates() {
    let gradient = Gradient::linear().stop(0.0, Color::RED).stop(1.0, Color::BLUE);
    let request = request(gradient);
    assert_eq!(request.kind, MaterialKind::Gradient);
    assert_eq!(request.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(request.opacity, 1.0);
    assert_eq!(request.gradient.shape, GradientShape::Linear);
    assert_eq!(request.gradient.stop_count, 2);
    assert_eq!(request.gradient.offsets[..2], [0.0, 1.0]);
    assert_eq!(request.gradient.colors[..2], [packed(RED), packed(BLUE)]);
}

#[test]
fn linear_angles_follow_css_and_span_the_box_corner_to_corner() {
    let stops = |gradient: Gradient| gradient.stop(0.0, Color::RED).stop(1.0, Color::BLUE);
    // The default points to the bottom: down the middle of an 80x40 box.
    close(request(stops(Gradient::linear())).gradient.geometry, [40.0, 0.0, 40.0, 40.0]);
    close(
        request(stops(Gradient::linear().angle(90.0))).gradient.geometry,
        [0.0, 20.0, 80.0, 20.0],
    );
    close(
        request(stops(Gradient::linear().angle(0.0))).gradient.geometry,
        [40.0, 40.0, 40.0, 0.0],
    );
    // At 45 degrees the line is as long as the box's projection onto it, so the
    // end lines touch the corners: 80 sin 45 + 40 cos 45.
    close(
        request(stops(Gradient::linear().angle(45.0))).gradient.geometry,
        [10.0, 50.0, 70.0, -10.0],
    );
    // Angles wrap, and nonfinite ones keep the default.
    close(
        request(stops(Gradient::linear().angle(450.0))).gradient.geometry,
        request(stops(Gradient::linear().angle(90.0))).gradient.geometry,
    );
    assert_eq!(
        stops(Gradient::linear().angle(f32::NAN)),
        stops(Gradient::linear())
    );
}

#[test]
fn radial_defaults_to_the_farthest_corner_ellipse_around_the_center() {
    let stops = |gradient: Gradient| gradient.stop(0.0, Color::RED).stop(1.0, Color::BLUE);
    let centered = request(stops(Gradient::radial()));
    assert_eq!(centered.gradient.shape, GradientShape::Radial);
    let (rx, ry) = (40.0 * std::f32::consts::SQRT_2, 20.0 * std::f32::consts::SQRT_2);
    close(centered.gradient.geometry, [40.0, 20.0, rx, ry]);

    let corner = request(stops(Gradient::radial().center(0.0, 0.0)));
    close(
        corner.gradient.geometry,
        [0.0, 0.0, 80.0 * std::f32::consts::SQRT_2, 40.0 * std::f32::consts::SQRT_2],
    );
    let half = request(stops(Gradient::radial().radius(0.5)));
    close(half.gradient.geometry, [40.0, 20.0, rx * 0.5, ry * 0.5]);
}

#[test]
fn conic_starts_at_the_top_and_turns_clockwise_like_css() {
    let stops = |gradient: Gradient| gradient.stop(0.0, Color::RED).stop(1.0, Color::BLUE);
    let top = request(stops(Gradient::conic()));
    assert_eq!(top.gradient.shape, GradientShape::Conic);
    // The shader measures from screen-right, so CSS's top is -90 degrees.
    close(top.gradient.geometry, [40.0, 20.0, -std::f32::consts::FRAC_PI_2, 0.0]);
    let right = request(stops(Gradient::conic().from_angle(90.0).center(0.25, 0.75)));
    close(right.gradient.geometry, [20.0, 30.0, 0.0, 0.0]);
}

#[test]
fn stops_are_kept_sorted_clamped_and_capped() {
    let gradient = Gradient::linear()
        .stop(0.9, Color::BLUE)
        .stop(0.1, Color::RED)
        .stop(f32::NAN, Color::WHITE)
        .stop(7.0, Color::BLACK)
        .stop(-3.0, Color::Rgba(1, 2, 3, 4));
    let ramp = request(gradient).gradient;
    assert_eq!(ramp.stop_count, 4, "a nonfinite offset adds no stop");
    assert_eq!(ramp.offsets[..4], [0.0, 0.1, 0.9, 1.0]);
    assert_eq!(
        ramp.colors[..4],
        [packed([1, 2, 3, 4]), packed(RED), packed(BLUE), packed([0, 0, 0, 255])]
    );

    let mut crowded = Gradient::linear();
    for index in 0..20 {
        crowded = crowded.stop(index as f32 / 20.0, Color::RED);
    }
    assert_eq!(request(crowded).gradient.stop_count, 8, "extra stops are ignored");
}

#[test]
fn colors_spread_evenly_and_replace_earlier_stops() {
    let ramp = request(
        Gradient::linear()
            .stop(0.3, Color::WHITE)
            .colors([Color::RED, Color::WHITE, Color::BLUE]),
    )
    .gradient;
    assert_eq!(ramp.stop_count, 3);
    assert_eq!(ramp.offsets[..3], [0.0, 0.5, 1.0]);
    assert_eq!(ramp.colors[0], packed(RED));
    assert_eq!(ramp.colors[2], packed(BLUE));

    let single = request(Gradient::linear().colors([Color::BLUE])).gradient;
    assert_eq!((single.stop_count, single.offsets[0]), (1, 0.0));

    let rainbow = request(Gradient::rainbow()).gradient;
    assert_eq!(rainbow.stop_count, 7);
    assert_eq!(rainbow.shape, GradientShape::Linear);
    close(
        request(Gradient::rainbow()).gradient.geometry,
        [0.0, 20.0, 80.0, 20.0],
    );
}

#[test]
fn an_empty_gradient_and_an_empty_box_paint_nothing() {
    let record = |gradient: Gradient, size: ResolvedSize| {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
        let canvas = Canvas::of(&tree.context(root).unwrap());
        gradient.paint(&canvas, Vec2d::default(), size, 1.0);
        canvas.finish();
        tree.draw_list_snapshot(root).unwrap().commands.to_vec()
    };
    assert!(Gradient::linear().is_empty());
    assert!(record(Gradient::linear(), BOX).is_empty());
    let drawn = Gradient::linear().colors([Color::RED, Color::BLUE]);
    assert!(!drawn.is_empty());
    for size in [
        ResolvedSize { width: 0.0, height: 40.0 },
        ResolvedSize { width: 80.0, height: 0.0 },
    ] {
        assert!(record(drawn, size).is_empty(), "{size:?}");
    }
    assert!(matches!(
        record(drawn, BOX).as_slice(),
        [DrawCommand::DrawCustom { .. }]
    ));
}

#[test]
fn a_gradient_takes_its_corners_from_preceding_shape_layers() {
    let gradient = Gradient::linear().colors([Color::RED, Color::BLUE]);
    assert_eq!(stacked(&[&gradient]).corner_radii, [0.0; 4]);
    assert_eq!(
        stacked(&[&BorderRadius::from(12.0), &gradient]).corner_radii,
        [12.0; 4]
    );
    // Radii fit the shorter side of the 80x40 box.
    assert_eq!(
        stacked(&[&BorderRadius::from(2000.0), &gradient]).corner_radii,
        [20.0; 4]
    );
    for invalid in [-4.0, f32::NAN, f32::INFINITY] {
        assert_eq!(
            stacked(&[&BorderRadius::from(invalid), &gradient]).corner_radii,
            [0.0; 4],
            "{invalid}"
        );
    }
}

#[test]
fn a_gradient_declares_no_geometry_of_its_own() {
    let gradient = Gradient::radial().colors([Color::RED, Color::BLUE]);
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    assert_eq!(gradient.paint_outsets(size, 2.0), [0.0; 4]);
    assert_eq!(gradient.content_insets(size, 2.0), [0.0; 4]);
    assert_eq!(gradient.child_clip_radius(size, 2.0), None);
    assert_eq!(gradient.background_color(), None);
    assert_eq!(gradient.revision(), 0);
    // Layout does not depend on the gradient: a larger box just re-resolves it.
    let wide = stacked_in(ResolvedSize { width: 160.0, height: 40.0 }, &[&gradient]);
    assert_eq!(wide.bounds, [3.0, 4.0, 160.0, 40.0]);
}
