use std::collections::HashMap;
use std::sync::Arc;

use super::{SvgFillRule, SvgGeometry, SvgLineCap, SvgLineJoin, SvgPathCommand, SvgStroke};

#[derive(Debug, thiserror::Error)]
pub enum SvgTessellationError {
    #[error("SVG path is empty")]
    EmptyPath,
    #[error("SVG tessellation failed: {0}")]
    Tessellation(String),
}

#[derive(Clone, Debug)]
pub struct SvgMesh {
    pub vertices: Arc<[[f32; 2]]>,
    /// Linear edge coverage for each vertex. Values are one inside the shape
    /// and fade to zero across analytic antialiasing fringe geometry.
    pub coverages: Arc<[f32]>,
    pub indices: Arc<[u32]>,
}

impl SvgMesh {
    pub fn memory_bytes(&self) -> usize {
        self.vertices.len() * (size_of::<[f32; 2]>() + size_of::<f32>())
            + self.indices.len() * size_of::<u32>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SvgMeshStyle {
    Fill(SvgFillRule),
    Stroke {
        width: f32,
        line_cap: SvgLineCap,
        line_join: SvgLineJoin,
        miter_limit: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SvgToleranceBucket(u8);

impl SvgToleranceBucket {
    const MIN_EXPONENT: i32 = -8;
    const MAX_EXPONENT: i32 = 16;
    pub const COUNT: usize = (Self::MAX_EXPONENT - Self::MIN_EXPONENT + 1) as usize;

    pub fn from_scale(scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let exponent = scale
            .log2()
            .ceil()
            .clamp(Self::MIN_EXPONENT as f32, Self::MAX_EXPONENT as f32)
            as i32;
        Self((exponent - Self::MIN_EXPONENT) as u8)
    }

    fn tolerance(self) -> f32 {
        let exponent = i32::from(self.0) + Self::MIN_EXPONENT;
        let representative_scale = 2.0_f32.powi(exponent);
        0.25 / representative_scale
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct GeometryKey {
    path: Vec<u32>,
    style: MeshStyleKey,
    tolerance: SvgToleranceBucket,
    analytic_aa: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MeshStyleKey {
    Fill(SvgFillRule),
    Stroke {
        width: u32,
        line_cap: SvgLineCap,
        line_join: SvgLineJoin,
        miter_limit: u32,
    },
}

impl From<SvgMeshStyle> for MeshStyleKey {
    fn from(style: SvgMeshStyle) -> Self {
        match style {
            SvgMeshStyle::Fill(rule) => Self::Fill(rule),
            SvgMeshStyle::Stroke {
                width,
                line_cap,
                line_join,
                miter_limit,
            } => Self::Stroke {
                width: width.to_bits(),
                line_cap,
                line_join,
                miter_limit: miter_limit.to_bits(),
            },
        }
    }
}

struct CacheEntry {
    mesh: Arc<SvgMesh>,
    last_used: u64,
}

pub struct SvgGeometryCache {
    entries: HashMap<GeometryKey, CacheEntry>,
    max_memory_bytes: usize,
    max_entries: usize,
    memory_bytes: usize,
    usage_clock: u64,
}

impl SvgGeometryCache {
    pub fn new(max_memory_bytes: usize, max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_memory_bytes,
            max_entries,
            memory_bytes: 0,
            usage_clock: 0,
        }
    }

    pub fn mesh_for(
        &mut self,
        geometry: &SvgGeometry,
        style: SvgMeshStyle,
        physical_scale: f32,
    ) -> Result<Arc<SvgMesh>, SvgTessellationError> {
        self.mesh_for_with_analytic_aa(geometry, style, physical_scale, false)
    }

    pub(crate) fn mesh_for_with_analytic_aa(
        &mut self,
        geometry: &SvgGeometry,
        style: SvgMeshStyle,
        physical_scale: f32,
        analytic_aa: bool,
    ) -> Result<Arc<SvgMesh>, SvgTessellationError> {
        self.usage_clock = self.usage_clock.wrapping_add(1);
        let tolerance = SvgToleranceBucket::from_scale(physical_scale);
        let key = GeometryKey {
            path: path_key(geometry),
            style: style.into(),
            tolerance,
            analytic_aa,
        };
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.usage_clock;
            return Ok(entry.mesh.clone());
        }

        let mesh = Arc::new(tessellate(
            geometry,
            style,
            tolerance.tolerance(),
            analytic_aa,
        )?);
        let mesh_bytes = mesh.memory_bytes();
        if self.max_entries > 0 && mesh_bytes <= self.max_memory_bytes {
            self.memory_bytes += mesh_bytes;
            self.entries.insert(
                key,
                CacheEntry {
                    mesh: mesh.clone(),
                    last_used: self.usage_clock,
                },
            );
            self.evict_to_limits();
        }
        Ok(mesh)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }

    pub fn max_memory_bytes(&self) -> usize {
        self.max_memory_bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.memory_bytes = 0;
    }

    fn evict_to_limits(&mut self) {
        while self.entries.len() > self.max_entries || self.memory_bytes > self.max_memory_bytes {
            let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest_key) {
                self.memory_bytes = self.memory_bytes.saturating_sub(entry.mesh.memory_bytes());
            }
        }
    }
}

fn tessellate(
    geometry: &SvgGeometry,
    style: SvgMeshStyle,
    tolerance: f32,
    analytic_aa: bool,
) -> Result<SvgMesh, SvgTessellationError> {
    if geometry.commands.is_empty() {
        return Err(SvgTessellationError::EmptyPath);
    }
    let fringe_width = tolerance * 4.0;
    match style {
        SvgMeshStyle::Fill(rule) => {
            let contours = close_fill_contours(flatten_path(geometry, tolerance));
            let mut mesh = scanline_fill(&contours, rule)?;
            if analytic_aa && fringe_width.is_finite() && fringe_width > 0.0 {
                add_fill_fringe(&mut mesh, &contours, rule, fringe_width);
            }
            Ok(mesh)
        }
        SvgMeshStyle::Stroke {
            width,
            line_cap,
            line_join,
            miter_limit,
        } => {
            if !width.is_finite() || width <= 0.0 || !miter_limit.is_finite() || miter_limit < 1.0 {
                return Err(SvgTessellationError::Tessellation(
                    "invalid stroke parameters".to_owned(),
                ));
            }
            // A stroke is painted once, as the union of its geometry. It is built as
            // overlapping polygons and filled with the non-zero rule, so no point is
            // covered twice: layering the pieces is invisible for an opaque colour and
            // wrong for any translucent one.
            let outline = stroke_outline(
                &flatten_path(geometry, tolerance),
                width,
                line_cap,
                line_join,
                miter_limit,
            );
            let mut mesh = scanline_fill(&outline, SvgFillRule::NonZero)?;
            if analytic_aa && fringe_width.is_finite() && fringe_width > 0.0 {
                add_fill_fringe(&mut mesh, &outline, SvgFillRule::NonZero, fringe_width);
            }
            Ok(mesh)
        }
    }
}

#[derive(Clone, Copy)]
struct Crossing {
    left: (f32, f32),
    right: (f32, f32),
    x_mid: f32,
    winding: i32,
}

#[derive(Clone, Copy)]
struct ScanEdge {
    a: (f32, f32),
    b: (f32, f32),
    min_y: f32,
    max_y: f32,
    winding: i32,
}

fn scanline_fill(
    contours: &[Vec<(f32, f32)>],
    rule: SvgFillRule,
) -> Result<SvgMesh, SvgTessellationError> {
    let mut levels = Vec::new();
    let mut edges = Vec::new();
    for contour in contours {
        for &(x, y) in contour {
            if !x.is_finite() || !y.is_finite() {
                return Err(SvgTessellationError::Tessellation(
                    "non-finite path coordinate".into(),
                ));
            }
            levels.push(y);
        }
        for segment in contour.windows(2) {
            let (a, b) = (segment[0], segment[1]);
            if a.1 != b.1 {
                edges.push(ScanEdge {
                    a,
                    b,
                    min_y: a.1.min(b.1),
                    max_y: a.1.max(b.1),
                    winding: if b.1 > a.1 { 1 } else { -1 },
                });
            }
        }
    }
    levels.sort_by(f32::total_cmp);
    levels.dedup_by(|a, b| a.to_bits() == b.to_bits());
    edges.sort_unstable_by(|a, b| a.min_y.total_cmp(&b.min_y));
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut active_edges = Vec::new();
    let mut crossings = Vec::new();
    let mut parts = Vec::new();
    let mut next_edge = 0;
    for band in levels.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 - y0 <= f32::EPSILON {
            continue;
        }
        let mid = y0 * 0.5 + y1 * 0.5;
        while next_edge < edges.len() && edges[next_edge].min_y <= mid {
            active_edges.push(next_edge);
            next_edge += 1;
        }
        active_edges.retain(|&index| edges[index].max_y > mid);
        // No edge starts or ends inside a band, but two edges can still cross
        // inside one, and then the order they have at the middle is not the order
        // they have at both ends. A band is split at the crossing, so that the
        // order inside each part is the order it is filled in.
        parts.clear();
        parts.push((y0, y1));
        let mut splits = 0;
        while let Some((top, bottom)) = parts.pop() {
            band_crossings(&edges, &active_edges, top, bottom, &mut crossings);
            if splits < MAX_BAND_SPLITS
                && let Some(split) = first_crossing(&crossings, top, bottom)
            {
                splits += 1;
                // Popped last, so the upper part is filled first.
                parts.push((split, bottom));
                parts.push((top, split));
                continue;
            }
            fill_band(&crossings, rule, &mut vertices, &mut indices);
        }
    }
    let coverages = vec![1.0; vertices.len()];
    Ok(SvgMesh {
        vertices: vertices.into(),
        coverages: coverages.into(),
        indices: indices.into(),
    })
}

/// The most times one scanline band is split where its edges cross. Bounds the
/// work for degenerate input; a band past it is filled as it is.
const MAX_BAND_SPLITS: usize = 4_096;

/// Where each active edge is across the band `top..bottom`, left to right at its
/// middle, written into `crossings` so one buffer serves every band.
fn band_crossings(edges: &[ScanEdge], active_edges: &[usize], top: f32, bottom: f32, crossings: &mut Vec<Crossing>) {
    let mid = top * 0.5 + bottom * 0.5;
    crossings.clear();
    for &index in active_edges {
        let edge = edges[index];
        let t = (mid - edge.a.1) / (edge.b.1 - edge.a.1);
        let x_mid = edge.a.0 + (edge.b.0 - edge.a.0) * t;
        let at = |y: f32| {
            let x = if y == edge.a.1 {
                edge.a.0
            } else if y == edge.b.1 {
                edge.b.0
            } else {
                edge.a.0 + (edge.b.0 - edge.a.0) * ((y - edge.a.1) / (edge.b.1 - edge.a.1))
            };
            (x, y)
        };
        crossings.push(Crossing {
            left: at(top),
            right: at(bottom),
            x_mid,
            winding: edge.winding,
        });
    }
    crossings.sort_by(|a, b| a.x_mid.total_cmp(&b.x_mid));
}

/// The height of the first place two neighbouring edges swap sides inside the band.
fn first_crossing(crossings: &[Crossing], top: f32, bottom: f32) -> Option<f32> {
    let mut first: Option<f32> = None;
    for pair in crossings.windows(2) {
        let at_top = pair[0].left.0 - pair[1].left.0;
        let at_bottom = pair[0].right.0 - pair[1].right.0;
        if at_top * at_bottom < 0.0 {
            let height = top + (bottom - top) * (at_top / (at_top - at_bottom));
            // Strictly inside, or the split would not make the band any smaller.
            if height > top && height < bottom {
                first = Some(first.map_or(height, |first| first.min(height)));
            }
        }
    }
    first
}

/// The quads covering the inside of one band, given its edges in left-to-right order.
fn fill_band(crossings: &[Crossing], rule: SvgFillRule, vertices: &mut Vec<[f32; 2]>, indices: &mut Vec<u32>) {
    let mut start = None;
    let mut winding = 0i32;
    let mut parity = false;
    for crossing in crossings {
        let was_inside = match rule {
            SvgFillRule::NonZero => winding != 0,
            SvgFillRule::EvenOdd => parity,
        };
        winding += crossing.winding;
        parity = !parity;
        let is_inside = match rule {
            SvgFillRule::NonZero => winding != 0,
            SvgFillRule::EvenOdd => parity,
        };
        if !was_inside && is_inside {
            start = Some(*crossing);
        }
        if was_inside && !is_inside {
            if let Some(left) = start.take() {
                append_quad(vertices, indices, left.left, crossing.left, crossing.right, left.right);
            }
        }
    }
}

fn close_fill_contours(mut contours: Vec<Vec<(f32, f32)>>) -> Vec<Vec<(f32, f32)>> {
    contours.retain(|contour| contour.len() >= 3);
    for contour in &mut contours {
        if contour.first() != contour.last() {
            contour.push(contour[0]);
        }
    }
    contours
}

fn add_fill_fringe(
    mesh: &mut SvgMesh,
    contours: &[Vec<(f32, f32)>],
    rule: SvgFillRule,
    width: f32,
) {
    let mut vertices = mesh.vertices.to_vec();
    let mut coverages = mesh.coverages.to_vec();
    let mut indices = mesh.indices.to_vec();
    let mut corners: HashMap<PointKey, ([f32; 2], Vec<[f32; 2]>)> = HashMap::new();
    let fill_query = FillQueryIndex::new(contours);

    for contour in contours {
        for segment in contour.windows(2) {
            let a = [segment[0].0, segment[0].1];
            let b = [segment[1].0, segment[1].1];
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let length = dx.hypot(dy);
            if !length.is_finite() || length <= f32::EPSILON {
                continue;
            }
            let probe = (width * 0.5).min(length * 0.1).max(f32::EPSILON * 8.0);
            let normal = [-dy / length, dx / length];
            let midpoint = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let left = [
                midpoint[0] + normal[0] * probe,
                midpoint[1] + normal[1] * probe,
            ];
            let right = [
                midpoint[0] - normal[0] * probe,
                midpoint[1] - normal[1] * probe,
            ];
            let left_filled = fill_query.contains(left, rule);
            let right_filled = fill_query.contains(right, rule);
            if left_filled == right_filled {
                continue;
            }
            let outward = if left_filled {
                [-normal[0], -normal[1]]
            } else {
                normal
            };
            let outer_a = [a[0] + outward[0] * width, a[1] + outward[1] * width];
            let outer_b = [b[0] + outward[0] * width, b[1] + outward[1] * width];
            append_coverage_quad(
                &mut vertices,
                &mut coverages,
                &mut indices,
                a,
                b,
                outer_b,
                outer_a,
            );
            corners
                .entry(point_key(a))
                .or_insert_with(|| (a, Vec::new()))
                .1
                .push(outer_a);
            corners
                .entry(point_key(b))
                .or_insert_with(|| (b, Vec::new()))
                .1
                .push(outer_b);
        }
    }

    append_fringe_corner_fans(&mut vertices, &mut coverages, &mut indices, corners);
    mesh.vertices = vertices.into();
    mesh.coverages = coverages.into();
    mesh.indices = indices.into();
}

#[derive(Clone, Copy)]
struct FillEdge {
    a: (f32, f32),
    b: (f32, f32),
    min_y: f32,
    max_y: f32,
}

struct FillIntervalNode {
    center: f32,
    by_min: Vec<usize>,
    by_max: Vec<usize>,
    left: Option<Box<Self>>,
    right: Option<Box<Self>>,
}

struct FillQueryIndex {
    edges: Vec<FillEdge>,
    root: Option<Box<FillIntervalNode>>,
}

impl FillQueryIndex {
    fn new(contours: &[Vec<(f32, f32)>]) -> Self {
        let edges = contours
            .iter()
            .flat_map(|contour| contour.windows(2))
            .filter_map(|segment| {
                let (a, b) = (segment[0], segment[1]);
                (a.1 != b.1).then_some(FillEdge {
                    a,
                    b,
                    min_y: a.1.min(b.1),
                    max_y: a.1.max(b.1),
                })
            })
            .collect::<Vec<_>>();
        let indices = (0..edges.len()).collect::<Vec<_>>();
        let root = build_fill_interval_node(&edges, indices);
        Self { edges, root }
    }

    fn contains(&self, point: [f32; 2], rule: SvgFillRule) -> bool {
        let mut winding = 0_i32;
        let mut crossings = 0_u32;
        let mut node = self.root.as_deref();
        while let Some(current) = node {
            if point[1] < current.center {
                for &index in &current.by_min {
                    let edge = self.edges[index];
                    if edge.min_y > point[1] {
                        break;
                    }
                    add_fill_crossing(edge, point, &mut winding, &mut crossings);
                }
                node = current.left.as_deref();
            } else if point[1] > current.center {
                for &index in &current.by_max {
                    let edge = self.edges[index];
                    if edge.max_y <= point[1] {
                        break;
                    }
                    add_fill_crossing(edge, point, &mut winding, &mut crossings);
                }
                node = current.right.as_deref();
            } else {
                for &index in &current.by_min {
                    add_fill_crossing(self.edges[index], point, &mut winding, &mut crossings);
                }
                break;
            }
        }
        match rule {
            SvgFillRule::NonZero => winding != 0,
            SvgFillRule::EvenOdd => crossings % 2 == 1,
        }
    }
}

fn build_fill_interval_node(
    edges: &[FillEdge],
    indices: Vec<usize>,
) -> Option<Box<FillIntervalNode>> {
    if indices.is_empty() {
        return None;
    }
    let mut centers = indices
        .iter()
        .map(|&index| edges[index].min_y * 0.5 + edges[index].max_y * 0.5)
        .collect::<Vec<_>>();
    centers.sort_by(f32::total_cmp);
    let center = centers[centers.len() / 2];
    let mut by_min = Vec::new();
    let mut left = Vec::new();
    let mut right = Vec::new();
    for index in indices {
        let edge = edges[index];
        if edge.max_y < center {
            left.push(index);
        } else if edge.min_y > center {
            right.push(index);
        } else {
            by_min.push(index);
        }
    }
    by_min.sort_unstable_by(|&a, &b| edges[a].min_y.total_cmp(&edges[b].min_y));
    let mut by_max = by_min.clone();
    by_max.sort_unstable_by(|&a, &b| edges[b].max_y.total_cmp(&edges[a].max_y));
    Some(Box::new(FillIntervalNode {
        center,
        by_min,
        by_max,
        left: build_fill_interval_node(edges, left),
        right: build_fill_interval_node(edges, right),
    }))
}

fn add_fill_crossing(edge: FillEdge, point: [f32; 2], winding: &mut i32, crossings: &mut u32) {
    let (a, b) = (edge.a, edge.b);
    if !((a.1 <= point[1] && b.1 > point[1]) || (b.1 <= point[1] && a.1 > point[1])) {
        return;
    }
    let intersection_x = a.0 + (point[1] - a.1) * (b.0 - a.0) / (b.1 - a.1);
    if intersection_x > point[0] {
        *crossings += 1;
        *winding += if b.1 > a.1 { 1 } else { -1 };
    }
}

fn append_fringe_corner_fans(
    vertices: &mut Vec<[f32; 2]>,
    coverages: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    corners: HashMap<PointKey, ([f32; 2], Vec<[f32; 2]>)>,
) {
    for (_, (point, mut outer_points)) in corners {
        outer_points.sort_by(|a, b| {
            (a[1] - point[1])
                .atan2(a[0] - point[0])
                .total_cmp(&(b[1] - point[1]).atan2(b[0] - point[0]))
        });
        outer_points.dedup_by(|a, b| point_key(*a) == point_key(*b));
        if outer_points.len() < 2 {
            continue;
        }
        for pair in outer_points.windows(2) {
            append_coverage_triangle(vertices, coverages, indices, point, pair[0], pair[1]);
        }
        if outer_points.len() > 2 {
            append_coverage_triangle(
                vertices,
                coverages,
                indices,
                point,
                *outer_points.last().unwrap_or(&outer_points[0]),
                outer_points[0],
            );
        }
    }
}

fn append_quad(
    vertices: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    d: (f32, f32),
) {
    let Ok(base) = u32::try_from(vertices.len()) else {
        return;
    };
    vertices.extend([[a.0, a.1], [b.0, b.1], [c.0, c.1], [d.0, d.1]]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// The widest turn, in radians, one segment of a round join's arc may span. Fifteen
/// degrees keeps the arc within one percent of the stroke radius of a true circle.
const MAX_ROUND_JOIN_STEP: f32 = std::f32::consts::PI / 12.0;
/// Segments in a round cap, which is half a circle.
const ROUND_CAP_STEPS: usize = 8;
/// Cosine of the turn below which two segments are treated as one smooth bend, and
/// joined by a single offset point. About one degree: the point is off by less than
/// a thousandth of the stroke radius, and a flattened curve stays a strip.
const SMOOTH_TURN_COS: f32 = 0.9998;

type Point = (f32, f32);

/// One segment of a stroked polyline: its direction, its left normal, and its length.
#[derive(Clone, Copy)]
struct StrokeSegment {
    direction: Point,
    normal: Point,
    length: f32,
}

/// Polygons whose union, filled with the non-zero rule, is the stroke of `contours`.
///
/// Each contour is one outline: the offset of its left side walked forward, then
/// the cap, the offset of its right side walked back, then the cap. A closed
/// contour is two loops, a ring. Joins are added on the outside of each turn;
/// the inside of a sharp turn passes through the vertex, which leaves a small
/// overlap that the non-zero rule fills once. So a stroke is a handful of edges per
/// segment, and its pieces overlap only where the path crosses itself or sharply
/// turns, not at every join.
fn stroke_outline(
    contours: &[Vec<Point>],
    width: f32,
    cap: SvgLineCap,
    join: SvgLineJoin,
    miter_limit: f32,
) -> Vec<Vec<Point>> {
    let radius = width * 0.5;
    let mut polygons = Vec::new();
    for contour in contours {
        let closed = contour.len() > 2 && contour.first() == contour.last();
        let mut points: Vec<Point> = Vec::with_capacity(contour.len());
        for &point in contour {
            if points.last().is_none_or(|last| (point.0 - last.0).hypot(point.1 - last.1) > f32::EPSILON) {
                points.push(point);
            }
        }
        if closed && points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() < 2 {
            continue;
        }
        let reversed: Vec<Point> = points.iter().rev().copied().collect();
        let forward = walk_side(&points, closed, radius, join, miter_limit);
        let backward = walk_side(&reversed, closed, radius, join, miter_limit);
        if closed {
            push_ring(&mut polygons, forward, backward);
        } else {
            let count = points.len();
            let mut outline = forward;
            outline.extend(cap_points(points[count - 1], points[count - 2], radius, cap));
            outline.extend(backward);
            outline.extend(cap_points(points[0], points[1], radius, cap));
            push_polygon(&mut polygons, outline);
        }
    }
    polygons
}

fn segment(from: Point, to: Point) -> StrokeSegment {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy);
    let direction = (dx / length, dy / length);
    StrokeSegment { direction, normal: (-direction.1, direction.0), length }
}

/// The boundary of the stroke on the left of `points`, walked forward, with joins at
/// the turns. The right side is the same walk over the reversed points.
fn walk_side(points: &[Point], closed: bool, radius: f32, join: SvgLineJoin, miter_limit: f32) -> Vec<Point> {
    let count = points.len();
    let segments: Vec<StrokeSegment> = (0..if closed { count } else { count - 1 })
        .map(|index| segment(points[index], points[(index + 1) % count]))
        .collect();
    let offset = |point: Point, normal: Point| (point.0 + normal.0 * radius, point.1 + normal.1 * radius);
    let mut outline = Vec::with_capacity(count * 2);
    if !closed {
        outline.push(offset(points[0], segments[0].normal));
    }
    let joints = if closed { 0..count } else { 1..count - 1 };
    for index in joints {
        let incoming = segments[(index + segments.len() - 1) % segments.len()];
        let outgoing = segments[index % segments.len()];
        join_points(&mut outline, points[index], incoming, outgoing, radius, join, miter_limit);
    }
    if !closed {
        outline.push(offset(points[count - 1], segments[segments.len() - 1].normal));
    }
    outline
}

/// The points that carry the left side of the stroke around the turn at `point`.
fn join_points(
    outline: &mut Vec<Point>,
    point: Point,
    incoming: StrokeSegment,
    outgoing: StrokeSegment,
    radius: f32,
    join: SvgLineJoin,
    miter_limit: f32,
) {
    let (n1, n2) = (incoming.normal, outgoing.normal);
    let cross = incoming.direction.0 * outgoing.direction.1 - incoming.direction.1 * outgoing.direction.0;
    let dot = incoming.direction.0 * outgoing.direction.0 + incoming.direction.1 * outgoing.direction.1;
    let a = (point.0 + n1.0 * radius, point.1 + n1.1 * radius);
    let b = (point.0 + n2.0 * radius, point.1 + n2.1 * radius);
    // Where the two offset lines meet: the tip of a miter on the outside of a turn,
    // the corner of the offset on the inside. It is the vertex plus the sum of the
    // normals over one plus their dot product.
    let scale = radius / (1.0 + dot);
    let meeting = (point.0 + (n1.0 + n2.0) * scale, point.1 + (n1.1 + n2.1) * scale);
    if dot > SMOOTH_TURN_COS {
        outline.push(meeting);
        return;
    }
    if cross > 0.0 {
        // The inside of the turn. The offset lines cross inside both segments when the
        // turn is gentle enough for the stroke's width; where they do not, the offset
        // goes through the vertex instead, and the overlap that leaves is filled once.
        let reach = radius * cross / (1.0 + dot);
        if reach <= incoming.length.min(outgoing.length) {
            outline.push(meeting);
        } else {
            outline.extend([a, point, b]);
        }
        return;
    }
    // The outside of the turn.
    match join {
        SvgLineJoin::Round => {
            // A 180 degree turn has no cross product to say which way round to go, and
            // the way round is forward, past the vertex.
            let sweep = if cross == 0.0 && dot < 0.0 {
                -std::f32::consts::PI
            } else {
                cross.atan2(dot)
            };
            let steps = ((sweep.abs() / MAX_ROUND_JOIN_STEP).ceil() as usize).max(1);
            let start = (n1.0 * radius, n1.1 * radius);
            for step in 0..=steps {
                let (sin, cos) = (sweep * step as f32 / steps as f32).sin_cos();
                outline.push((
                    point.0 + start.0 * cos - start.1 * sin,
                    point.1 + start.0 * sin + start.1 * cos,
                ));
            }
        }
        SvgLineJoin::Miter | SvgLineJoin::MiterClip
            if 1.0 + dot > f32::EPSILON
                && (meeting.0 - point.0).hypot(meeting.1 - point.1) <= miter_limit * radius =>
        {
            outline.extend([a, meeting, b]);
        }
        _ => outline.extend([a, b]),
    }
}

/// The points that round or square the end of a stroke at `end`, whose path arrives
/// from `previous`, between the two sides of the stroke there.
fn cap_points(end: Point, previous: Point, radius: f32, cap: SvgLineCap) -> Vec<Point> {
    let (dx, dy) = (end.0 - previous.0, end.1 - previous.1);
    let length = dx.hypot(dy);
    let direction = (dx / length, dy / length);
    let normal = (-direction.1, direction.0);
    match cap {
        SvgLineCap::Butt => Vec::new(),
        SvgLineCap::Square => vec![
            (
                end.0 + (normal.0 + direction.0) * radius,
                end.1 + (normal.1 + direction.1) * radius,
            ),
            (
                end.0 + (direction.0 - normal.0) * radius,
                end.1 + (direction.1 - normal.1) * radius,
            ),
        ],
        SvgLineCap::Round => (1..ROUND_CAP_STEPS)
            .map(|step| {
                let (sin, cos) = (std::f32::consts::PI * step as f32 / ROUND_CAP_STEPS as f32).sin_cos();
                (
                    end.0 + (normal.0 * cos + direction.0 * sin) * radius,
                    end.1 + (normal.1 * cos + direction.1 * sin) * radius,
                )
            })
            .collect(),
    }
}

fn signed_area(points: &[Point]) -> f32 {
    let mut area = 0.0_f32;
    for (index, point) in points.iter().enumerate() {
        let next = points[(index + 1) % points.len()];
        area += point.0 * next.1 - next.0 * point.1;
    }
    area
}

/// Adds `points` as a closed polygon wound positively, dropping a degenerate one.
fn push_polygon(polygons: &mut Vec<Vec<Point>>, mut points: Vec<Point>) {
    let area = signed_area(&points);
    if !area.is_finite() || area.abs() <= f32::EPSILON {
        return;
    }
    if area < 0.0 {
        points.reverse();
    }
    points.push(points[0]);
    polygons.push(points);
}

/// Adds the two loops of a closed stroke. The larger is the outer edge and is wound
/// positively like every other piece; the other is the inner edge, wound the other
/// way, so that the non-zero rule leaves the hole inside it unfilled.
fn push_ring(polygons: &mut Vec<Vec<Point>>, a: Vec<Point>, b: Vec<Point>) {
    let (area_a, area_b) = (signed_area(&a), signed_area(&b));
    if !area_a.is_finite() || !area_b.is_finite() {
        return;
    }
    let (mut outer, mut inner) = if area_a.abs() >= area_b.abs() { (a, b) } else { (b, a) };
    if signed_area(&outer) < 0.0 {
        outer.reverse();
    }
    if signed_area(&inner) > 0.0 {
        inner.reverse();
    }
    for mut polygon in [outer, inner] {
        if polygon.len() >= 3 {
            polygon.push(polygon[0]);
            polygons.push(polygon);
        }
    }
}

type PointKey = (u32, u32);

fn point_key(point: [f32; 2]) -> PointKey {
    (point[0].to_bits(), point[1].to_bits())
}

fn append_coverage_quad(
    vertices: &mut Vec<[f32; 2]>,
    coverages: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    d: [f32; 2],
) {
    let Ok(base) = u32::try_from(vertices.len()) else {
        return;
    };
    vertices.extend([a, b, c, d]);
    coverages.extend([1.0, 1.0, 0.0, 0.0]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn append_coverage_triangle(
    vertices: &mut Vec<[f32; 2]>,
    coverages: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
) {
    let Ok(base) = u32::try_from(vertices.len()) else {
        return;
    };
    vertices.extend([a, b, c]);
    coverages.extend([1.0, 0.0, 0.0]);
    indices.extend([base, base + 1, base + 2]);
}

/// Tessellates a stroke with the SVG dash pattern retained by [`SvgStroke`].
///
/// The path is flattened into bounded line pieces before the stroke mesh is
/// generated. Each visible dash is an independent subpath, which preserves the
/// requested cap style and makes gaps real geometry gaps.
pub fn tessellate_dashed_stroke(
    geometry: &SvgGeometry,
    stroke: &SvgStroke,
    physical_scale: f32,
) -> Result<SvgMesh, SvgTessellationError> {
    let tolerance = SvgToleranceBucket::from_scale(physical_scale).tolerance();
    if stroke.dash_array.is_empty() {
        return tessellate(
            geometry,
            SvgMeshStyle::Stroke {
                width: stroke.width,
                line_cap: stroke.line_cap,
                line_join: stroke.line_join,
                miter_limit: stroke.miter_limit,
            },
            tolerance,
            false,
        );
    }
    if !stroke.dash_offset.is_finite()
        || !stroke.width.is_finite()
        || stroke.width <= 0.0
        || !stroke.miter_limit.is_finite()
        || stroke.miter_limit < 1.0
        || stroke
            .dash_array
            .iter()
            .any(|dash| !dash.is_finite() || *dash < 0.0)
    {
        return Err(SvgTessellationError::Tessellation(
            "invalid dashed stroke parameters".to_owned(),
        ));
    }

    let pattern_sum = stroke.dash_array.iter().copied().sum::<f32>();
    if !pattern_sum.is_finite() {
        return Err(SvgTessellationError::Tessellation(
            "dashed stroke pattern is not finite".to_owned(),
        ));
    }
    if pattern_sum <= f32::EPSILON {
        return Ok(empty_mesh());
    }

    let mut pattern = stroke.dash_array.to_vec();
    if pattern.len() % 2 == 1 {
        pattern.extend_from_within(..);
    }
    let commands = dashed_commands(geometry, &pattern, stroke.dash_offset, tolerance);
    if commands.is_empty() {
        return Ok(empty_mesh());
    }
    tessellate(
        &SvgGeometry {
            commands: commands.into(),
        },
        SvgMeshStyle::Stroke {
            width: stroke.width,
            line_cap: stroke.line_cap,
            line_join: stroke.line_join,
            miter_limit: stroke.miter_limit,
        },
        tolerance,
        false,
    )
}

fn empty_mesh() -> SvgMesh {
    SvgMesh {
        vertices: Arc::from([]),
        coverages: Arc::from([]),
        indices: Arc::from([]),
    }
}

fn dashed_commands(
    geometry: &SvgGeometry,
    pattern: &[f32],
    dash_offset: f32,
    tolerance: f32,
) -> Vec<SvgPathCommand> {
    let Some(period) = pattern.iter().copied().reduce(|sum, value| sum + value) else {
        return Vec::new();
    };
    if !period.is_finite() || period <= f32::EPSILON {
        return Vec::new();
    }

    let mut commands = Vec::new();
    for contour in flatten_path(geometry, tolerance) {
        if contour.len() < 2 {
            continue;
        }
        let (mut pattern_index, mut remaining, mut on) =
            dash_cursor(pattern, dash_offset.rem_euclid(period));
        for segment in contour.windows(2) {
            let start = segment[0];
            let end = segment[1];
            let delta = (end.0 - start.0, end.1 - start.1);
            let length = delta.0.hypot(delta.1);
            if !length.is_finite() || length <= f32::EPSILON {
                continue;
            }
            let mut distance = 0.0;
            while distance < length {
                while remaining <= f32::EPSILON {
                    advance_dash_cursor(pattern, &mut pattern_index, &mut remaining, &mut on);
                }
                let step = remaining.min(length - distance);
                if on && step > f32::EPSILON {
                    let start_t = distance / length;
                    let end_t = (distance + step) / length;
                    commands.push(SvgPathCommand::MoveTo {
                        x: start.0 + delta.0 * start_t,
                        y: start.1 + delta.1 * start_t,
                    });
                    commands.push(SvgPathCommand::LineTo {
                        x: start.0 + delta.0 * end_t,
                        y: start.1 + delta.1 * end_t,
                    });
                }
                distance += step;
                remaining -= step;
                if remaining <= f32::EPSILON {
                    advance_dash_cursor(pattern, &mut pattern_index, &mut remaining, &mut on);
                }
            }
        }
    }
    commands
}

fn dash_cursor(pattern: &[f32], mut phase: f32) -> (usize, f32, bool) {
    let mut index = 0;
    while pattern[index] <= f32::EPSILON || phase >= pattern[index] {
        if pattern[index] > f32::EPSILON {
            phase -= pattern[index];
        }
        index = (index + 1) % pattern.len();
        if phase <= f32::EPSILON {
            phase = 0.0;
            break;
        }
    }
    (index, (pattern[index] - phase).max(0.0), index % 2 == 0)
}

fn advance_dash_cursor(pattern: &[f32], index: &mut usize, remaining: &mut f32, on: &mut bool) {
    for _ in 0..pattern.len() {
        *index = (*index + 1) % pattern.len();
        *remaining = pattern[*index];
        *on = *index % 2 == 0;
        if *remaining > f32::EPSILON {
            return;
        }
    }
    *remaining = f32::MAX;
    *on = false;
}

const MAX_CURVE_SUBDIVISION_DEPTH: u8 = 12;

fn flatten_path(geometry: &SvgGeometry, tolerance: f32) -> Vec<Vec<(f32, f32)>> {
    let mut contours = Vec::new();
    let mut contour = Vec::new();
    let tolerance_squared = tolerance * tolerance;
    let mut current = (0.0, 0.0);
    for command in geometry.commands.iter().copied() {
        match command {
            SvgPathCommand::MoveTo { x, y } => {
                if contour.len() >= 2 {
                    contours.push(std::mem::take(&mut contour));
                } else {
                    contour.clear();
                }
                current = (x, y);
                contour.push(current);
            }
            SvgPathCommand::LineTo { x, y } => {
                if contour.is_empty() {
                    contour.push(current);
                }
                current = (x, y);
                contour.push(current);
            }
            SvgPathCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                if contour.is_empty() {
                    contour.push(current);
                }
                flatten_quadratic(
                    &mut contour,
                    current,
                    (control_x, control_y),
                    (x, y),
                    tolerance_squared,
                    0,
                );
                current = (x, y);
            }
            SvgPathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                if contour.is_empty() {
                    contour.push(current);
                }
                flatten_cubic(
                    &mut contour,
                    current,
                    (control1_x, control1_y),
                    (control2_x, control2_y),
                    (x, y),
                    tolerance_squared,
                    0,
                );
                current = (x, y);
            }
            SvgPathCommand::Close => {
                if let Some(first) = contour.first().copied()
                    && Some(first) != contour.last().copied()
                {
                    contour.push(first);
                }
                current = contour.first().copied().unwrap_or(current);
            }
        }
    }
    if contour.len() >= 2 {
        contours.push(contour);
    }
    contours
}

fn flatten_quadratic(
    contour: &mut Vec<(f32, f32)>,
    start: (f32, f32),
    control: (f32, f32),
    end: (f32, f32),
    tolerance_squared: f32,
    depth: u8,
) {
    if depth >= MAX_CURVE_SUBDIVISION_DEPTH
        || distance_to_segment_squared(control, start, end) <= tolerance_squared
    {
        contour.push(end);
        return;
    }

    let start_control = midpoint(start, control);
    let control_end = midpoint(control, end);
    let middle = midpoint(start_control, control_end);
    flatten_quadratic(
        contour,
        start,
        start_control,
        middle,
        tolerance_squared,
        depth + 1,
    );
    flatten_quadratic(
        contour,
        middle,
        control_end,
        end,
        tolerance_squared,
        depth + 1,
    );
}

fn flatten_cubic(
    contour: &mut Vec<(f32, f32)>,
    start: (f32, f32),
    control1: (f32, f32),
    control2: (f32, f32),
    end: (f32, f32),
    tolerance_squared: f32,
    depth: u8,
) {
    if depth >= MAX_CURVE_SUBDIVISION_DEPTH
        || (distance_to_segment_squared(control1, start, end) <= tolerance_squared
            && distance_to_segment_squared(control2, start, end) <= tolerance_squared)
    {
        contour.push(end);
        return;
    }

    let start_control1 = midpoint(start, control1);
    let control1_control2 = midpoint(control1, control2);
    let control2_end = midpoint(control2, end);
    let start_middle = midpoint(start_control1, control1_control2);
    let middle_end = midpoint(control1_control2, control2_end);
    let middle = midpoint(start_middle, middle_end);
    flatten_cubic(
        contour,
        start,
        start_control1,
        start_middle,
        middle,
        tolerance_squared,
        depth + 1,
    );
    flatten_cubic(
        contour,
        middle,
        middle_end,
        control2_end,
        end,
        tolerance_squared,
        depth + 1,
    );
}

fn midpoint(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    (a.0 * 0.5 + b.0 * 0.5, a.1 * 0.5 + b.1 * 0.5)
}

fn distance_to_segment_squared(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let segment = (end.0 - start.0, end.1 - start.1);
    let length_squared = segment.0 * segment.0 + segment.1 * segment.1;
    let projection = if length_squared > 0.0 {
        (((point.0 - start.0) * segment.0 + (point.1 - start.1) * segment.1) / length_squared)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    let nearest = (
        start.0 + segment.0 * projection,
        start.1 + segment.1 * projection,
    );
    let delta = (point.0 - nearest.0, point.1 - nearest.1);
    delta.0 * delta.0 + delta.1 * delta.1
}

fn path_key(geometry: &SvgGeometry) -> Vec<u32> {
    let mut key = Vec::with_capacity(geometry.commands.len() * 7);
    for command in geometry.commands.iter() {
        match *command {
            SvgPathCommand::MoveTo { x, y } => key.extend([0, x.to_bits(), y.to_bits()]),
            SvgPathCommand::LineTo { x, y } => key.extend([1, x.to_bits(), y.to_bits()]),
            SvgPathCommand::QuadraticTo {
                control_x,
                control_y,
                x,
                y,
            } => {
                key.extend([
                    2,
                    control_x.to_bits(),
                    control_y.to_bits(),
                    x.to_bits(),
                    y.to_bits(),
                ]);
            }
            SvgPathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => key.extend([
                3,
                control1_x.to_bits(),
                control1_y.to_bits(),
                control2_x.to_bits(),
                control2_y.to_bits(),
                x.to_bits(),
                y.to_bits(),
            ]),
            SvgPathCommand::Close => key.push(4),
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubic_flattening_respects_the_screen_space_error_bound() {
        let geometry = SvgGeometry {
            commands: Arc::from([
                SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                SvgPathCommand::CubicTo {
                    control1_x: 0.0,
                    control1_y: 10.0,
                    control2_x: 10.0,
                    control2_y: 10.0,
                    x: 10.0,
                    y: 0.0,
                },
            ]),
        };
        let physical_scale = 4.0;
        let tolerance = SvgToleranceBucket::from_scale(physical_scale).tolerance();
        let contour = &flatten_path(&geometry, tolerance)[0];
        let mut maximum_error_squared = 0.0_f32;

        for step in 0..=1000 {
            let t = step as f32 / 1000.0;
            let inverse = 1.0 - t;
            let point = (
                inverse.powi(3) * 0.0
                    + 3.0 * inverse.powi(2) * t * 0.0
                    + 3.0 * inverse * t.powi(2) * 10.0
                    + t.powi(3) * 10.0,
                inverse.powi(3) * 0.0
                    + 3.0 * inverse.powi(2) * t * 10.0
                    + 3.0 * inverse * t.powi(2) * 10.0
                    + t.powi(3) * 0.0,
            );
            let nearest_error_squared = contour
                .windows(2)
                .map(|segment| distance_to_segment_squared(point, segment[0], segment[1]))
                .fold(f32::INFINITY, f32::min);
            maximum_error_squared = maximum_error_squared.max(nearest_error_squared);
        }

        assert!(maximum_error_squared.sqrt() * physical_scale <= 0.25 + 1e-4);
    }

    #[test]
    fn quadratic_flattening_refines_for_the_screen_space_error_bound() {
        let geometry = SvgGeometry {
            commands: Arc::from([
                SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                SvgPathCommand::QuadraticTo {
                    control_x: 5.0,
                    control_y: 10.0,
                    x: 10.0,
                    y: 0.0,
                },
            ]),
        };
        let physical_scale = 4.0;
        let tolerance = SvgToleranceBucket::from_scale(physical_scale).tolerance();
        let contour = &flatten_path(&geometry, tolerance)[0];
        let mut maximum_error_squared = 0.0_f32;

        for step in 0..=1000 {
            let t = step as f32 / 1000.0;
            let inverse = 1.0 - t;
            let point = (
                inverse.powi(2) * 0.0 + 2.0 * inverse * t * 5.0 + t.powi(2) * 10.0,
                inverse.powi(2) * 0.0 + 2.0 * inverse * t * 10.0 + t.powi(2) * 0.0,
            );
            let nearest_error_squared = contour
                .windows(2)
                .map(|segment| distance_to_segment_squared(point, segment[0], segment[1]))
                .fold(f32::INFINITY, f32::min);
            maximum_error_squared = maximum_error_squared.max(nearest_error_squared);
        }

        assert!(maximum_error_squared.sqrt() * physical_scale <= 0.25 + 1e-4);
    }

    #[test]
    fn straight_bezier_does_not_subdivide_at_large_scales() {
        let geometry = SvgGeometry {
            commands: Arc::from([
                SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                SvgPathCommand::CubicTo {
                    control1_x: 3.0,
                    control1_y: 0.0,
                    control2_x: 7.0,
                    control2_y: 0.0,
                    x: 10.0,
                    y: 0.0,
                },
            ]),
        };
        let contour = flatten_path(
            &geometry,
            SvgToleranceBucket::from_scale(10_000.0).tolerance(),
        );

        assert_eq!(contour[0].len(), 2);
    }

    /// How many triangles of `mesh` cover `point`, counting only the strict
    /// interior so a point on a shared edge is not counted twice, which is also
    /// how the rasterizer treats it.
    fn layers_at(mesh: &SvgMesh, point: [f32; 2]) -> usize {
        mesh.indices
            .chunks_exact(3)
            .filter(|triangle| barycentric(mesh, triangle, point).is_some_and(|w| w.iter().all(|w| *w > 1e-4)))
            .count()
    }

    /// Whether any triangle of `mesh` covers `point`, edges included. Unlike
    /// [`layers_at`] a point on a shared edge, which a long thin triangle puts
    /// within float error of many sample points, still counts as covered.
    fn covered_at(mesh: &SvgMesh, point: [f32; 2]) -> bool {
        mesh.indices
            .chunks_exact(3)
            .any(|triangle| barycentric(mesh, triangle, point).is_some_and(|w| w.iter().all(|w| *w >= -1e-3)))
    }

    /// Alpha of a stroke of `alpha` composited triangle by triangle at `point`,
    /// as the GPU does, with each vertex's edge coverage interpolated.
    fn composited_alpha(mesh: &SvgMesh, point: [f32; 2], alpha: f32) -> f32 {
        let mut result = 0.0_f32;
        for triangle in mesh.indices.chunks_exact(3) {
            if let Some(w) = barycentric(mesh, triangle, point)
                && w.iter().all(|w| *w > 1e-4)
            {
                let coverage: f32 = (0..3).map(|i| w[i] * mesh.coverages[triangle[i] as usize]).sum();
                result = 1.0 - (1.0 - result) * (1.0 - alpha * coverage);
            }
        }
        result
    }

    fn barycentric(mesh: &SvgMesh, triangle: &[u32], p: [f32; 2]) -> Option<[f32; 3]> {
        let (a, b, c) = (mesh.vertices[triangle[0] as usize], mesh.vertices[triangle[1] as usize], mesh.vertices[triangle[2] as usize]);
        let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if d.abs() < 1e-9 {
            return None;
        }
        let w0 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
        let w1 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
        Some([w0, w1, 1.0 - w0 - w1])
    }

    /// The first stroked path of `path`, as an SVG `d` string, with its stroke style.
    fn stroked(path: &str, width: f32, cap: &str, join: &str) -> (SvgGeometry, SvgMeshStyle) {
        let svg = format!(
            r##"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="{path}" stroke="#000000" stroke-width="{width}" stroke-linecap="{cap}" stroke-linejoin="{join}"/></svg>"##
        );
        let parsed = crate::svg::parser::parse_svg_document(svg.as_bytes()).unwrap();
        let node = parsed.scene.nodes.iter().find(|node| node.geometry.is_some()).unwrap();
        let stroke = node.stroke.as_ref().unwrap();
        let style = SvgMeshStyle::Stroke {
            width: stroke.width,
            line_cap: stroke.line_cap,
            line_join: stroke.line_join,
            miter_limit: stroke.miter_limit,
        };
        (parsed.scene.geometry(node).unwrap().clone(), style)
    }

    fn mesh_of(path: &str, width: f32, cap: &str, join: &str, analytic_aa: bool) -> SvgMesh {
        let (geometry, style) = stroked(path, width, cap, join);
        let mut cache = SvgGeometryCache::new(1 << 24, 4);
        // 3.3: the copy icon at 20 logical pixels on a 2x display.
        (*cache.mesh_for_with_analytic_aa(&geometry, style, 3.3, analytic_aa).unwrap()).clone()
    }

    /// The most layers any point of a 0.1-spaced grid over `0..24` is covered by. The
    /// grid is offset by amounts no path coordinate has, so no sample sits on a
    /// triangle edge or a scanline boundary.
    fn deepest_overlap(mesh: &SvgMesh) -> (usize, [f32; 2]) {
        let mut deepest = (0, [0.0, 0.0]);
        for iy in 0..240 {
            for ix in 0..240 {
                let point = [ix as f32 * 0.1 + 0.0537, iy as f32 * 0.1 + 0.0411];
                let layers = layers_at(mesh, point);
                if layers > deepest.0 {
                    deepest = (layers, point);
                }
            }
        }
        deepest
    }

    const COPY_ICON: &str = "M9 15H5C3.89543 15 3 14.1046 3 13V5C3 3.89543 3.89543 3 5 3H13C14.1046 3 15 3.89543 15 5V9M11 21H19C20.1046 21 21 20.1046 21 19V11C21 9.89543 20.1046 9 19 9H11C9.89543 9 9 9.89543 9 11V19C9 20.1046 9.89543 21 11 21Z";

    /// A stroke is painted once, as the union of its geometry. Layering its pieces
    /// is invisible for an opaque colour and wrong for any translucent one.
    #[test]
    fn crossing_strokes_cover_every_point_once() {
        let mesh = mesh_of("M2 2L18 18M2 18L18 2", 2.0, "round", "round", false);
        let (layers, at) = deepest_overlap(&mesh);
        assert_eq!(layers, 1, "covered {layers} times at {at:?}");
        assert_eq!(layers_at(&mesh, [10.013, 10.021]), 1, "the crossing itself is covered");
    }

    #[test]
    fn a_joined_stroke_does_not_overlap_the_segments_it_joins() {
        for join in ["round", "bevel", "miter"] {
            let mesh = mesh_of("M2 2L18 2L18 18L2 18", 2.0, "round", join, false);
            let (layers, at) = deepest_overlap(&mesh);
            assert_eq!(layers, 1, "{join}: covered {layers} times at {at:?}");
        }
    }

    #[test]
    fn a_curved_stroke_does_not_overlap_itself() {
        let mesh = mesh_of("M12 3C17 3 21 7 21 12C21 17 17 21 12 21C7 21 3 17 3 12C3 7 7 3 12 3Z", 2.0, "round", "round", false);
        let (layers, at) = deepest_overlap(&mesh);
        assert_eq!(layers, 1, "covered {layers} times at {at:?}");
    }

    #[test]
    fn the_copy_icons_stroke_is_one_layer_everywhere() {
        let mesh = mesh_of(COPY_ICON, 2.0, "round", "round", false);
        let (layers, at) = deepest_overlap(&mesh);
        assert_eq!(layers, 1, "covered {layers} times at {at:?}");
    }

    /// Merging the pieces must not lose any of the stroke or add any to it.
    #[test]
    fn the_union_covers_exactly_the_stroke_band() {
        let mesh = mesh_of("M2 2L18 2L18 18L2 18", 2.0, "round", "round", false);
        let segments = [([2.0_f32, 2.0], [18.0, 2.0]), ([18.0, 2.0], [18.0, 18.0]), ([18.0, 18.0], [2.0, 18.0])];
        let distance = |p: [f32; 2]| {
            segments.iter().map(|(a, b)| {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let t = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                ((p[0] - (a[0] + t * dx)).hypot(p[1] - (a[1] + t * dy))) as f32
            }).fold(f32::MAX, f32::min)
        };
        for iy in 0..240 {
            for ix in 0..240 {
                let point = [ix as f32 * 0.1 + 0.0537, iy as f32 * 0.1 + 0.0411];
                if distance(point) <= 0.85 {
                    assert!(covered_at(&mesh, point), "{point:?} is inside the stroke");
                } else if distance(point) >= 1.15 {
                    assert!(!covered_at(&mesh, point), "{point:?} is outside the stroke");
                }
            }
        }
    }

    /// With edge anti-aliasing on, a translucent stroke is one alpha throughout,
    /// crossings included: the fringe belongs to the union's edge, not to each piece.
    #[test]
    fn a_translucent_stroke_has_one_alpha_across_a_crossing() {
        let mesh = mesh_of("M2 2L18 18M2 18L18 2", 2.0, "round", "round", true);
        // Inside the stroke, away from any edge: on a centre line, and at the crossing.
        // Off any triangle edge or scanline band boundary, which a strict-interior
        // test would skip.
        for point in [[10.013, 10.021], [6.013, 6.021], [14.013, 6.021], [9.013, 9.021], [11.013, 9.021]] {
            let alpha = composited_alpha(&mesh, point, 0.6);
            assert!((alpha - 0.6).abs() < 0.01, "alpha {alpha} at {point:?}");
        }
    }

    #[test]
    fn a_translucent_copy_icon_is_one_alpha_inside_its_stroke() {
        let mesh = mesh_of(COPY_ICON, 2.0, "round", "round", true);
        // Centre of the front square's bottom edge, and the crossings of the two squares.
        for point in [[15.013, 21.021], [15.013, 9.021], [9.013, 14.021], [9.013, 10.021]] {
            let alpha = composited_alpha(&mesh, point, 0.6);
            assert!((alpha - 0.6).abs() < 0.01, "alpha {alpha} at {point:?}");
        }
    }

    /// The outside of a corner is filled out to the join's tip, with no notch
    /// between the two segments' ends.
    #[test]
    fn a_miter_join_fills_the_whole_outer_corner() {
        let mesh = mesh_of("M2 2L18 2L18 18", 2.0, "butt", "miter", false);
        // The corner square [18, 19] x [1, 2], including the wedge next to the corner point.
        for point in [[18.3, 1.7], [18.5, 1.5], [18.7, 1.3], [18.9, 1.1]] {
            assert!(covered_at(&mesh, point), "{point:?}");
        }
        assert!(!covered_at(&mesh, [19.2, 0.8]), "past the tip");
        assert_eq!(deepest_overlap(&mesh).0, 1);
    }

    #[test]
    fn a_bevel_join_cuts_the_corner_off() {
        let mesh = mesh_of("M2 2L18 2L18 18", 2.0, "butt", "bevel", false);
        assert!(covered_at(&mesh, [18.3, 1.7]), "the wedge beside the corner point");
        assert!(!covered_at(&mesh, [18.9, 1.1]), "the corner itself is cut");
        assert_eq!(deepest_overlap(&mesh).0, 1);
    }
}
