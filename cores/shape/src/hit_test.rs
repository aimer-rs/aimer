use crate::{FillRule, FillStyle, Point, ShapePath, StrokeStyle};

/// The geometric policy a shape renderer may use for pointer hit testing.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ShapeHitTest {
    /// Do not make the shape itself a hit target.
    #[default]
    None,
    /// Use the path's axis-aligned local bounds.
    Bounds,
    /// Use closed filled contours and the selected fill rule.
    Fill,
    /// Use the stroke centerline and width.
    Stroke,
    /// Hit either the fill or stroke.
    FillOrStroke,
}

impl ShapeHitTest {
    /// Tests a point against a path using the supplied optional paints.
    ///
    /// A missing fill or stroke never invents one. Non-finite points and
    /// invalid paint values safely return `false` at this boundary.
    pub fn contains(
        self,
        path: &ShapePath,
        point: Point,
        fill: Option<&FillStyle>,
        stroke: Option<&StrokeStyle>,
    ) -> bool {
        if !point.is_finite() {
            return false;
        }
        match self {
            Self::None => false,
            Self::Bounds => path.bounds().contains(point),
            Self::Fill => fill.is_some_and(|fill| {
                fill.validate().is_ok() && fill_contains(path, point, fill.rule)
            }),
            Self::Stroke => stroke.is_some_and(|stroke| {
                stroke.validate().is_ok() && stroke_contains(path, point, stroke)
            }),
            Self::FillOrStroke => {
                fill.is_some_and(|fill| {
                    fill.validate().is_ok() && fill_contains(path, point, fill.rule)
                }) || stroke.is_some_and(|stroke| {
                    stroke.validate().is_ok() && stroke_contains(path, point, stroke)
                })
            }
        }
    }
}

fn fill_contains(path: &ShapePath, point: Point, rule: FillRule) -> bool {
    let Ok(polylines) = path.flattened(0.25) else {
        return false;
    };
    let mut winding = 0i32;
    let mut crossings = 0u32;
    for polyline in polylines.iter().filter(|polyline| polyline.closed) {
        for pair in polyline.points.windows(2) {
            let [a, b] = pair else { continue };
            if point_on_segment(point, *a, *b, 1.0e-4) {
                return true;
            }
            if (a.y > point.y) != (b.y > point.y) {
                let x = a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y);
                if x > point.x {
                    crossings = crossings.wrapping_add(1);
                }
                if (b.y > a.y) && x > point.x {
                    winding += 1;
                } else if (a.y > b.y) && x > point.x {
                    winding -= 1;
                }
            }
        }
    }
    match rule {
        FillRule::EvenOdd => crossings % 2 == 1,
        FillRule::NonZero => winding != 0,
    }
}

fn stroke_contains(path: &ShapePath, point: Point, stroke: &StrokeStyle) -> bool {
    let Ok(polylines) = path.flattened(0.25) else {
        return false;
    };
    let radius = stroke.width * 0.5;
    for polyline in polylines {
        let mut distance_along = 0.0;
        for (index, pair) in polyline.points.windows(2).enumerate() {
            let [a, b] = pair else { continue };
            let length = a.distance_squared(*b).sqrt();
            if length <= f32::EPSILON {
                continue;
            }
            let segment_start = distance_along;
            let segment_end = distance_along + length;
            if stroke.dash().is_solid() {
                if segment_distance(
                    point,
                    *a,
                    *b,
                    radius,
                    stroke.line_cap,
                    polyline.closed || index > 0,
                    polyline.closed || index + 2 < polyline.points.len(),
                ) {
                    return true;
                }
            } else if dashed_segment_contains(
                point,
                *a,
                *b,
                segment_start,
                segment_end,
                radius,
                stroke,
            ) {
                return true;
            }
            distance_along = segment_end;
        }
    }
    false
}

fn dashed_segment_contains(
    point: Point,
    a: Point,
    b: Point,
    segment_start: f32,
    segment_end: f32,
    radius: f32,
    stroke: &StrokeStyle,
) -> bool {
    let length = segment_end - segment_start;
    let pattern = stroke.dash().segments();
    let period: f32 = pattern.iter().sum();
    if !period.is_finite() || period <= 0.0 {
        return false;
    }
    let mut cursor = segment_start;
    while cursor < segment_end {
        let phase = (cursor + stroke.dash().offset()).rem_euclid(period);
        let mut accumulated = 0.0;
        let mut entry = 0;
        for (index, value) in pattern.iter().copied().enumerate() {
            if phase < accumulated + value {
                entry = index;
                break;
            }
            accumulated += value;
        }
        let remaining = (accumulated + pattern[entry] - phase).max(f32::EPSILON);
        let next = (cursor + remaining).min(segment_end);
        if entry % 2 == 0 {
            let t0 = (cursor - segment_start) / length;
            let t1 = (next - segment_start) / length;
            let p0 = lerp(a, b, t0);
            let p1 = lerp(a, b, t1);
            if segment_distance(point, p0, p1, radius, stroke.line_cap, true, true) {
                return true;
            }
        }
        cursor = next;
    }
    false
}

fn segment_distance(
    point: Point,
    a: Point,
    b: Point,
    radius: f32,
    cap: crate::LineCap,
    has_previous: bool,
    has_next: bool,
) -> bool {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return point.distance_squared(a) <= radius * radius;
    }
    let length = length_squared.sqrt();
    let mut t = ((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared;
    let endpoint = if t < 0.0 {
        Some(a)
    } else if t > 1.0 {
        Some(b)
    } else {
        None
    };
    match (endpoint, cap) {
        (Some(endpoint), crate::LineCap::Butt) if !has_previous || !has_next => {
            return point.distance_squared(endpoint) <= radius * radius && (0.0..=1.0).contains(&t);
        }
        (Some(endpoint), crate::LineCap::Round) if !has_previous || !has_next => {
            return point.distance_squared(endpoint) <= radius * radius;
        }
        (Some(_), crate::LineCap::Square) if !has_previous || !has_next => {
            t = t.clamp(-radius / length, 1.0 + radius / length);
        }
        _ => {}
    }
    t = t.clamp(0.0, 1.0);
    let closest = lerp(a, b, t);
    point.distance_squared(closest) <= radius * radius
}

#[inline]
fn lerp(a: Point, b: Point, t: f32) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

#[inline]
fn point_on_segment(point: Point, a: Point, b: Point, tolerance: f32) -> bool {
    let ab_x = b.x - a.x;
    let ab_y = b.y - a.y;
    let ap_x = point.x - a.x;
    let ap_y = point.y - a.y;
    let cross = (ab_x * ap_y - ab_y * ap_x).abs();
    if cross > tolerance {
        return false;
    }
    let dot = ap_x * ab_x + ap_y * ab_y;
    dot >= -tolerance && dot <= ab_x * ab_x + ab_y * ab_y + tolerance
}
