//! Cubic-bezier easing, the curve behind Lottie's `o` / `i` keyframe handles.

use crate::model::Vec2;

/// Solver tolerance on the time axis. Finer than a pixel of motion for any
/// realistic animation, and near what `f32` can resolve near `1.0`.
const EPSILON: f32 = 1e-6;

/// Maps linear progress `x` in `0..=1` to eased progress, for the curve from
/// `(0, 0)` to `(1, 1)` with control points `p1` and `p2`.
///
/// The x handles are clamped to `0..=1`, which keeps the curve a function of
/// time (monotonic in x); y handles may leave that range to overshoot. Newton's
/// method does the work in a few iterations, with bisection as the fallback
/// for the flat spots where its slope vanishes.
pub(crate) fn cubic_bezier(p1: Vec2, p2: Vec2, x: f32) -> f32 {
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let (x1, x2) = (p1[0].clamp(0.0, 1.0), p2[0].clamp(0.0, 1.0));
    let (y1, y2) = (p1[1], p2[1]);

    // B(s) = ((a*s + b)*s + c)*s, with B(0) = 0 and B(1) = 1.
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sample_x = |s: f32| ((ax * s + bx) * s + cx) * s;
    let sample_y = |s: f32| ((ay * s + by) * s + cy) * s;

    let mut s = x;
    for _ in 0..8 {
        let error = sample_x(s) - x;
        if error.abs() < EPSILON {
            return sample_y(s);
        }
        let slope = (3.0 * ax * s + 2.0 * bx) * s + cx;
        if slope.abs() < 1e-6 {
            break;
        }
        s = (s - error / slope).clamp(0.0, 1.0);
    }

    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = x;
    for _ in 0..32 {
        let value = sample_x(s);
        if (value - x).abs() < EPSILON {
            break;
        }
        if x > value {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) * 0.5;
    }
    sample_y(s)
}
