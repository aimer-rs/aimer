//! Lottie shapes as SVG path commands.

use crate::model::{Bezier, Vec2};
use aimer_cupid::svg::SvgPathCommand::{self, Close, CubicTo, LineTo, MoveTo};

/// Control-point distance that makes a cubic approximate a quarter circle.
const KAPPA: f32 = 0.552_284_7;

/// Appends a rectangle centered on `center`. `roundness` is the corner radius,
/// clamped to half the short side; zero or less gives square corners.
pub(crate) fn rect(center: Vec2, size: Vec2, roundness: f32, out: &mut Vec<SvgPathCommand>) {
    let (hw, hh) = (size[0].abs() / 2.0, size[1].abs() / 2.0);
    let (l, r) = (center[0] - hw, center[0] + hw);
    let (t, b) = (center[1] - hh, center[1] + hh);
    // A NaN radius fails the comparison and falls back to square corners.
    let radius = if roundness > 0.0 { roundness.min(hw.min(hh)) } else { 0.0 };
    if radius == 0.0 {
        out.extend([MoveTo { x: l, y: t }, LineTo { x: r, y: t }, LineTo { x: r, y: b }, LineTo { x: l, y: b }, Close]);
        return;
    }
    let k = radius * KAPPA;
    let arc = |c1x, c1y, c2x, c2y, x, y| CubicTo {
        control1_x: c1x,
        control1_y: c1y,
        control2_x: c2x,
        control2_y: c2y,
        x,
        y,
    };
    out.extend([
        MoveTo { x: l + radius, y: t },
        LineTo { x: r - radius, y: t },
        arc(r - radius + k, t, r, t + radius - k, r, t + radius),
        LineTo { x: r, y: b - radius },
        arc(r, b - radius + k, r - radius + k, b, r - radius, b),
        LineTo { x: l + radius, y: b },
        arc(l + radius - k, b, l, b - radius + k, l, b - radius),
        LineTo { x: l, y: t + radius },
        arc(l, t + radius - k, l + radius - k, t, l + radius, t),
        Close,
    ]);
}

/// Appends an ellipse centered on `center`, starting at its rightmost point.
pub(crate) fn ellipse(center: Vec2, size: Vec2, out: &mut Vec<SvgPathCommand>) {
    let (rx, ry) = (size[0].abs() / 2.0, size[1].abs() / 2.0);
    let (cx, cy) = (center[0], center[1]);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    let arc = |c1x, c1y, c2x, c2y, x, y| CubicTo {
        control1_x: c1x,
        control1_y: c1y,
        control2_x: c2x,
        control2_y: c2y,
        x,
        y,
    };
    out.extend([
        MoveTo { x: cx + rx, y: cy },
        arc(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry),
        arc(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy),
        arc(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry),
        arc(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy),
        Close,
    ]);
}

/// Appends a Lottie path. Tangents are relative to their vertex; a segment with
/// both tangents zero is a straight line and costs the tessellator less.
pub(crate) fn bezier(path: &Bezier, out: &mut Vec<SvgPathCommand>) {
    // The fields are public, so do not trust them to be the same length.
    let n = path.vertices.len().min(path.in_tangents.len()).min(path.out_tangents.len());
    if n == 0 {
        return;
    }
    let v = &path.vertices;
    out.push(MoveTo { x: v[0][0], y: v[0][1] });
    let segment = |from: usize, to: usize, out: &mut Vec<SvgPathCommand>| {
        let (leave, enter) = (path.out_tangents[from], path.in_tangents[to]);
        if leave == [0.0; 2] && enter == [0.0; 2] {
            out.push(LineTo { x: v[to][0], y: v[to][1] });
        } else {
            out.push(CubicTo {
                control1_x: v[from][0] + leave[0],
                control1_y: v[from][1] + leave[1],
                control2_x: v[to][0] + enter[0],
                control2_y: v[to][1] + enter[1],
                x: v[to][0],
                y: v[to][1],
            });
        }
    };
    for i in 0..n - 1 {
        segment(i, i + 1, out);
    }
    if path.closed {
        if n > 1 {
            segment(n - 1, 0, out);
        }
        out.push(Close);
    }
}
