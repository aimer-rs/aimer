use crate::ease::cubic_bezier;
use crate::model::Vec2;

/// The point of the curve at parameter `s`, computed directly from the control
/// points. Easing `x` must give back `y`, which pins the solver without any
/// remembered reference values.
fn point(p1: Vec2, p2: Vec2, s: f32) -> (f32, f32) {
    let u = 1.0 - s;
    let at = |a: f32, b: f32| 3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s;
    (at(p1[0], p2[0]), at(p1[1], p2[1]))
}

const CURVES: [(Vec2, Vec2); 6] = [
    ([0.42, 0.0], [0.58, 1.0]),
    ([0.42, 0.0], [1.0, 1.0]),
    ([0.0, 0.0], [0.58, 1.0]),
    ([0.25, 0.1], [0.25, 1.0]),
    ([0.9, 0.0], [0.1, 1.0]),
    ([0.2, -0.6], [0.3, 1.8]),
];

#[test]
fn ends_are_fixed_and_out_of_range_is_clamped() {
    for (p1, p2) in CURVES {
        assert_eq!(cubic_bezier(p1, p2, 0.0), 0.0);
        assert_eq!(cubic_bezier(p1, p2, 1.0), 1.0);
        assert_eq!(cubic_bezier(p1, p2, -3.0), 0.0);
        assert_eq!(cubic_bezier(p1, p2, 7.0), 1.0);
    }
}

#[test]
fn diagonal_control_points_are_linear() {
    for x in [0.1, 0.25, 0.5, 0.9] {
        let y = cubic_bezier([1.0 / 3.0, 1.0 / 3.0], [2.0 / 3.0, 2.0 / 3.0], x);
        assert!((y - x).abs() < 1e-4, "x={x} y={y}");
    }
}

#[test]
fn eased_value_lies_on_the_curve() {
    for (p1, p2) in CURVES {
        for i in 1..40 {
            let (x, want) = point(p1, p2, i as f32 / 40.0);
            let got = cubic_bezier(p1, p2, x);
            assert!((got - want).abs() < 2e-4, "{p1:?} {p2:?} x={x} got={got} want={want}");
        }
    }
}

#[test]
fn symmetric_ease_in_out_passes_through_the_middle() {
    let y = cubic_bezier([0.42, 0.0], [0.58, 1.0], 0.5);
    assert!((y - 0.5).abs() < 1e-4, "{y}");
}

#[test]
fn monotone_curves_never_decrease() {
    let (p1, p2) = ([0.42, 0.0], [0.58, 1.0]);
    let mut last = 0.0;
    for i in 0..=200 {
        let y = cubic_bezier(p1, p2, i as f32 / 200.0);
        assert!(y >= last - 1e-5, "dropped at {i}: {last} -> {y}");
        last = y;
    }
}

#[test]
fn overshoot_curves_leave_the_unit_range() {
    let lowest = (1..100)
        .map(|i| cubic_bezier([0.2, -0.6], [0.3, 1.8], i as f32 / 100.0))
        .fold(f32::MAX, f32::min);
    assert!(lowest < 0.0, "a negative control point must dip below zero, got {lowest}");
}

#[test]
fn out_of_range_x_handles_and_nan_stay_finite() {
    // Lottie x handles belong to 0..=1; anything else is clamped, not trusted.
    let y = cubic_bezier([-2.0, 0.0], [3.0, 1.0], 0.5);
    assert!(y.is_finite());
    assert_eq!(cubic_bezier([0.4, 0.0], [0.6, 1.0], f32::NAN), 0.0);
}
