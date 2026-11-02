use super::{
    Edge, FlattenedGlyph, MAX_FLATTENED_POINTS, Point, SAMPLE_GRID, SMALL_GLYPH_SAMPLE_GRID,
    edge_winding,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CoverageFillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug)]
struct ScanlineIntersection {
    x: f32,
    winding: i8,
}

#[derive(Clone, Copy, Debug)]
struct CoverageSpan {
    start: f32,
    end: f32,
}

#[derive(Clone)]
struct ScanlinePlan {
    sample_grid: u32,
    offsets: Vec<usize>,
    spans: Vec<CoverageSpan>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveScanlineEdge {
    edge_index: usize,
    x: f32,
    winding: i8,
}

#[derive(Clone, Default)]
pub(super) struct EdgeRows {
    offsets: Vec<usize>,
    indices: Vec<usize>,
}

impl EdgeRows {
    #[inline]
    fn row(&self, row: usize) -> &[usize] {
        let Some((&start, &end)) = self.offsets.get(row).zip(self.offsets.get(row + 1)) else {
            return &[];
        };
        self.indices.get(start..end).unwrap_or(&[])
    }
}

#[derive(Default)]
pub(crate) struct RasterScratch {
    active_edges: Vec<ActiveScanlineEdge>,
    active_positions: Vec<usize>,
}

pub(super) fn build_coverage_bitmap(
    flattened: &FlattenedGlyph,
    scratch: &mut RasterScratch,
) -> Option<Vec<u8>> {
    build_coverage_bitmap_with_fill_rule(flattened, scratch, CoverageFillRule::NonZero)
}

pub(super) fn build_coverage_bitmap_with_fill_rule(
    flattened: &FlattenedGlyph,
    scratch: &mut RasterScratch,
    fill_rule: CoverageFillRule,
) -> Option<Vec<u8>> {
    let bitmap_len = flattened.width.checked_mul(flattened.height)?;
    let mut bitmap = vec![0; bitmap_len];
    let has_coverage = scan_convert_into_with_fill_rule(
        &flattened.edges,
        Some(&flattened.row_edges),
        flattened.width,
        flattened.height,
        SAMPLE_GRID,
        &mut bitmap,
        scratch,
        fill_rule,
    );
    if !has_coverage {
        scan_convert_into_with_fill_rule(
            &flattened.edges,
            Some(&flattened.row_edges),
            flattened.width,
            flattened.height,
            SMALL_GLYPH_SAMPLE_GRID,
            &mut bitmap,
            scratch,
            fill_rule,
        );
    }
    Some(bitmap)
}

fn contours_to_edges(contours: &[Vec<Point>]) -> Vec<Edge> {
    contours
        .iter()
        .flat_map(|contour| contour.windows(2).map(|edge| Edge::new(edge[0], edge[1])))
        .collect()
}

pub(super) fn edge_rows(edges: &[Edge], height: usize) -> Option<EdgeRows> {
    let mut counts = vec![0_usize; height];
    let height = height as f32;
    for edge in edges {
        if edge.min_y >= edge.max_y {
            continue;
        }
        let first_row = (height - edge.max_y).floor().max(0.0) as usize;
        let last_row = (height - edge.min_y).ceil().min(height) as usize;
        for row in first_row.min(counts.len())..last_row.min(counts.len()) {
            let row_top = height - row as f32;
            let row_bottom = row_top - 1.0;
            if edge.max_y > row_bottom && edge.min_y < row_top {
                counts[row] = counts[row].checked_add(1)?;
            }
        }
    }

    let mut offsets: Vec<usize> = Vec::with_capacity(counts.len().checked_add(1)?);
    offsets.push(0);
    for &count in &counts {
        let next = offsets.last().copied()?.checked_add(count)?;
        offsets.push(next);
    }

    let mut indices = vec![0; offsets.last().copied()?];
    // Reuse the counting array as the write cursor. The previous version
    // cloned one cursor per row, which added a second allocation to every
    // cold glyph flattening.
    for (row, cursor) in counts.iter_mut().enumerate() {
        *cursor = offsets[row];
    }
    let height_f32 = height;
    for (index, edge) in edges.iter().enumerate() {
        if edge.min_y >= edge.max_y {
            continue;
        }
        let first_row = (height_f32 - edge.max_y).floor().max(0.0) as usize;
        let last_row = (height_f32 - edge.min_y).ceil().min(height_f32) as usize;
        for row in first_row.min(counts.len())..last_row.min(counts.len()) {
            let row_top = height_f32 - row as f32;
            let row_bottom = row_top - 1.0;
            if edge.max_y > row_bottom && edge.min_y < row_top {
                let position = counts[row];
                counts[row] = position.checked_add(1)?;
                *indices.get_mut(position)? = index;
            }
        }
    }

    Some(EdgeRows { offsets, indices })
}

fn build_scanline_plan(
    edges: &[Edge],
    row_edges: &EdgeRows,
    width: usize,
    height: usize,
    sample_grid: u32,
) -> Option<ScanlinePlan> {
    build_scanline_plan_with(edges, row_edges, width, height, sample_grid, |_, _| {})
}

fn build_scanline_plan_and_fill(
    edges: &[Edge],
    row_edges: &EdgeRows,
    width: usize,
    height: usize,
    sample_grid: u32,
    bitmap: &mut [u8],
) -> Option<(ScanlinePlan, bool)> {
    let bitmap_len = width.checked_mul(height)?;
    if bitmap.len() < bitmap_len {
        return None;
    }
    bitmap[..bitmap_len].fill(0);

    let sample_grid_usize = usize::try_from(sample_grid).ok()?;
    let mut has_coverage = false;
    let plan = build_scanline_plan_with(
        edges,
        row_edges,
        width,
        height,
        sample_grid,
        |scanline, spans| {
            let row = scanline / sample_grid_usize;
            has_coverage |= fill_coverage_spans(spans, row * width, width, sample_grid, bitmap);
        },
    )?;
    normalize_bitmap(&mut bitmap[..bitmap_len], sample_grid);
    Some((plan, has_coverage))
}

fn build_scanline_plan_with<F>(
    edges: &[Edge],
    row_edges: &EdgeRows,
    width: usize,
    height: usize,
    sample_grid: u32,
    mut emit_spans: F,
) -> Option<ScanlinePlan>
where
    F: FnMut(usize, &[CoverageSpan]),
{
    if sample_grid == 0 {
        return None;
    }
    let sample_grid_usize = usize::try_from(sample_grid).ok()?;
    let scanline_count = height.checked_mul(sample_grid_usize)?;
    let mut offsets = Vec::new();
    offsets.try_reserve(scanline_count.checked_add(1)?).ok()?;
    offsets.push(0);

    let mut intersections = Vec::new();
    intersections.try_reserve(edges.len()).ok()?;
    let mut spans = Vec::new();

    for row in 0..height {
        let edges_for_row = row_edges.row(row);
        for sample_y in 0..sample_grid {
            let y = height as f32 - (row as f32 + (sample_y as f32 + 0.5) / sample_grid as f32);
            intersections.clear();
            for &edge_index in edges_for_row {
                let edge = edges[edge_index];
                if y < edge.min_y || y >= edge.max_y {
                    continue;
                }
                let crossing = edge.inverse_slope * (y - edge.start.y) + edge.start.x;
                if crossing.is_finite() {
                    intersections.push(ScanlineIntersection {
                        x: crossing,
                        winding: edge.winding,
                    });
                }
            }
            intersections.sort_unstable_by(|first, second| first.x.total_cmp(&second.x));

            let line_start = spans.len();
            let mut winding = 0_i32;
            let mut span_start = 0.0_f32;
            for intersection in &intersections {
                let start = span_start.max(0.0);
                let end = intersection.x.min(width as f32);
                if start >= end {
                    winding += i32::from(intersection.winding);
                    span_start = intersection.x;
                    continue;
                }
                if winding != 0 {
                    if spans.len() >= MAX_FLATTENED_POINTS {
                        return None;
                    }
                    spans.push(CoverageSpan { start, end });
                }
                winding += i32::from(intersection.winding);
                span_start = intersection.x;
            }
            emit_spans(
                row * sample_grid_usize + sample_y as usize,
                &spans[line_start..],
            );
            offsets.push(spans.len());
        }
    }

    Some(ScanlinePlan {
        sample_grid,
        offsets,
        spans,
    })
}

fn fill_scanline_plan(plan: &ScanlinePlan, width: usize, height: usize, bitmap: &mut [u8]) -> bool {
    let Some(bitmap_len) = width.checked_mul(height) else {
        return false;
    };
    if bitmap.len() < bitmap_len || plan.sample_grid == 0 {
        return false;
    }
    let Ok(sample_grid) = usize::try_from(plan.sample_grid) else {
        return false;
    };
    let Some(scanline_count) = height.checked_mul(sample_grid) else {
        return false;
    };
    if plan.offsets.len() < scanline_count.saturating_add(1) {
        return false;
    }
    bitmap[..bitmap_len].fill(0);

    let mut has_coverage = false;
    for row in 0..height {
        let row_offset = row * width;
        for sample_y in 0..sample_grid {
            let scanline = row * sample_grid + sample_y;
            let start = plan.offsets[scanline];
            let end = plan.offsets[scanline + 1];
            has_coverage |= fill_coverage_spans(
                &plan.spans[start..end],
                row_offset,
                width,
                plan.sample_grid,
                bitmap,
            );
        }
    }

    normalize_bitmap(&mut bitmap[..bitmap_len], plan.sample_grid);
    has_coverage
}

fn fill_coverage_spans(
    spans: &[CoverageSpan],
    row_offset: usize,
    width: usize,
    sample_grid: u32,
    bitmap: &mut [u8],
) -> bool {
    let row = &mut bitmap[row_offset..row_offset + width];
    let mut has_coverage = false;
    for span in spans {
        has_coverage |= fill_coverage_span(row, span.start, span.end, sample_grid);
    }
    has_coverage
}

#[inline]
fn fill_coverage_span(row: &mut [u8], start: f32, end: f32, sample_grid: u32) -> bool {
    // Scanline crossings supply finite coordinates clipped to this row.
    if start >= end || sample_grid == 0 {
        return false;
    }
    let first_column = start.floor() as usize;
    let last_column = end.ceil().min(row.len() as f32) as usize;
    let full_start = start.ceil() as usize;
    let full_end = end.floor() as usize;
    let mut has_coverage = false;

    if full_start < full_end {
        if first_column < full_start {
            let samples = horizontal_coverage_samples(start, end, first_column as f32, sample_grid);
            if samples != 0 {
                row[first_column] += samples as u8;
            }
        }
        // Disjoint spans contribute at most sample_grid samples per Y sample.
        // Normal 4x4 and retry 8x8 grids therefore accumulate at most 16 or 64.
        // Full pixels need no per-column floating-point coverage checks.
        aimer_simd::kernels::add_u8_in_place(&mut row[full_start..full_end], sample_grid as u8);
        has_coverage = true;
        if full_end < last_column {
            let samples = horizontal_coverage_samples(start, end, full_end as f32, sample_grid);
            if samples != 0 {
                row[full_end] += samples as u8;
            }
        }
    } else {
        // A span without a full pixel touches at most two boundary pixels.
        for column in first_column..last_column {
            let samples = horizontal_coverage_samples(start, end, column as f32, sample_grid);
            if samples != 0 {
                row[column] += samples as u8;
                has_coverage = true;
            }
        }
    }
    has_coverage
}

fn normalize_bitmap(bitmap: &mut [u8], sample_grid: u32) {
    aimer_simd::kernels::normalize_coverage_u8(bitmap, sample_grid);
}

fn scan_convert_reference(
    contours: &[Vec<Point>],
    width: usize,
    height: usize,
    bitmap_len: usize,
) -> Vec<u8> {
    let edges = contours
        .iter()
        .flat_map(|contour| contour.windows(2).map(|edge| (edge[0], edge[1])))
        .collect::<Vec<_>>();
    let mut intersections = Vec::with_capacity(edges.len());
    let mut covered_samples = vec![0_u8; bitmap_len];

    for row in 0..height {
        for sample_y in 0..SAMPLE_GRID {
            let y = height as f32 - (row as f32 + (sample_y as f32 + 0.5) / SAMPLE_GRID as f32);
            intersections.clear();
            for (start, end) in &edges {
                if (start.y > y) == (end.y > y) {
                    continue;
                }
                let crossing = (end.x - start.x) * (y - start.y) / (end.y - start.y) + start.x;
                if crossing.is_finite() {
                    intersections.push(ScanlineIntersection {
                        x: crossing,
                        winding: edge_winding(*start, *end),
                    });
                }
            }
            intersections.sort_unstable_by(|first, second| first.x.total_cmp(&second.x));

            let mut winding = 0_i32;
            let mut span_start = 0.0_f32;
            for intersection in &intersections {
                let start = span_start.max(0.0);
                let end = intersection.x.min(width as f32);
                if start >= end {
                    winding += i32::from(intersection.winding);
                    span_start = intersection.x;
                    continue;
                }
                if winding != 0 {
                    let first_column = start.floor() as usize;
                    let last_column = end.ceil().min(width as f32) as usize;
                    for column in first_column..last_column {
                        for sample_x in 0..SAMPLE_GRID {
                            let x = column as f32 + (sample_x as f32 + 0.5) / SAMPLE_GRID as f32;
                            if x >= start && x < end {
                                covered_samples[row * width + column] += 1;
                            }
                        }
                    }
                }
                winding += i32::from(intersection.winding);
                span_start = intersection.x;
            }
        }
    }

    covered_samples
        .into_iter()
        .map(|covered| {
            (u32::from(covered) * 255 + SAMPLE_GRID * SAMPLE_GRID / 2)
                .checked_div(SAMPLE_GRID * SAMPLE_GRID)
                .unwrap_or(0) as u8
        })
        .collect()
}

fn scan_convert_into_with_rows(
    edges: &[Edge],
    row_edges: Option<&EdgeRows>,
    width: usize,
    height: usize,
    sample_grid: u32,
    bitmap: &mut [u8],
    scratch: &mut RasterScratch,
) -> bool {
    scan_convert_into_with_fill_rule(
        edges,
        row_edges,
        width,
        height,
        sample_grid,
        bitmap,
        scratch,
        CoverageFillRule::NonZero,
    )
}

fn scan_convert_into_with_fill_rule(
    edges: &[Edge],
    row_edges: Option<&EdgeRows>,
    width: usize,
    height: usize,
    sample_grid: u32,
    bitmap: &mut [u8],
    scratch: &mut RasterScratch,
    fill_rule: CoverageFillRule,
) -> bool {
    let Some(bitmap_len) = width.checked_mul(height) else {
        return false;
    };
    if bitmap.len() < bitmap_len {
        return false;
    }
    if sample_grid == 0 {
        return false;
    }
    bitmap[..bitmap_len].fill(0);

    scratch.active_edges.clear();
    scratch.active_positions.resize(edges.len(), usize::MAX);
    scratch.active_positions.fill(usize::MAX);
    let mut has_coverage = false;

    for row in 0..height {
        scratch.active_edges.clear();
        let candidates = row_edges.map(|rows| rows.row(row));

        for sample_y in 0..sample_grid {
            let y = height as f32 - (row as f32 + (sample_y as f32 + 0.5) / sample_grid as f32);
            update_active_edges(edges, candidates, y, scratch);
            sort_active_edges(scratch, sample_y == 0);
            has_coverage |=
                fill_active_scanline(row, width, sample_grid, bitmap, scratch, fill_rule);
        }

        for active in &scratch.active_edges {
            scratch.active_positions[active.edge_index] = usize::MAX;
        }
    }

    scratch.active_edges.clear();
    normalize_bitmap(&mut bitmap[..bitmap_len], sample_grid);
    has_coverage
}

fn update_active_edges(
    edges: &[Edge],
    candidates: Option<&[usize]>,
    y: f32,
    scratch: &mut RasterScratch,
) {
    let mut active_index = 0;
    while active_index < scratch.active_edges.len() {
        let edge_index = scratch.active_edges[active_index].edge_index;
        let edge = edges[edge_index];
        if y < edge.min_y || y >= edge.max_y {
            scratch.active_positions[edge_index] = usize::MAX;
            scratch.active_edges.swap_remove(active_index);
            if let Some(moved) = scratch.active_edges.get(active_index) {
                scratch.active_positions[moved.edge_index] = active_index;
            }
            continue;
        }
        // Recompute from the sample coordinate instead of accumulating a
        // slope delta. Accumulation can cross a coverage-sample boundary due
        // to float drift and change a glyph's edge by one antialiasing sample.
        scratch.active_edges[active_index].x =
            edge.inverse_slope * (y - edge.start.y) + edge.start.x;
        active_index += 1;
    }

    let mut activate = |edge_index: usize| {
        let edge = edges[edge_index];
        if y < edge.min_y || y >= edge.max_y {
            return;
        }
        if scratch.active_positions[edge_index] != usize::MAX {
            return;
        }
        let active_index = scratch.active_edges.len();
        scratch.active_edges.push(ActiveScanlineEdge {
            edge_index,
            x: edge.inverse_slope * (y - edge.start.y) + edge.start.x,
            winding: edge.winding,
        });
        scratch.active_positions[edge_index] = active_index;
    };

    if let Some(candidates) = candidates {
        for &edge_index in candidates {
            activate(edge_index);
        }
    } else {
        for edge_index in 0..edges.len() {
            activate(edge_index);
        }
    }
}

fn sort_active_edges(scratch: &mut RasterScratch, full_sort: bool) {
    if full_sort {
        scratch
            .active_edges
            .sort_unstable_by(|first, second| first.x.total_cmp(&second.x));
        for (index, active) in scratch.active_edges.iter().enumerate() {
            scratch.active_positions[active.edge_index] = index;
        }
        return;
    }

    // The crossings are already in the previous sample's order. Insertion
    // sort repairs only the edges that crossed between samples and is much
    // cheaper than starting a general comparison sort for every scanline.
    for index in 1..scratch.active_edges.len() {
        let mut position = index;
        while position > 0
            && scratch.active_edges[position]
                .x
                .total_cmp(&scratch.active_edges[position - 1].x)
                .is_lt()
        {
            scratch.active_edges.swap(position, position - 1);
            let right = scratch.active_edges[position].edge_index;
            let left = scratch.active_edges[position - 1].edge_index;
            scratch.active_positions[right] = position;
            scratch.active_positions[left] = position - 1;
            position -= 1;
        }
    }
}

fn fill_active_scanline(
    row: usize,
    width: usize,
    sample_grid: u32,
    bitmap: &mut [u8],
    scratch: &RasterScratch,
    fill_rule: CoverageFillRule,
) -> bool {
    let mut has_coverage = false;
    let mut winding = 0_i32;
    let mut span_start = 0.0_f32;
    let row_offset = row * width;
    let row_bitmap = &mut bitmap[row_offset..row_offset + width];

    for intersection in &scratch.active_edges {
        let start = span_start.max(0.0);
        let end = intersection.x.min(width as f32);
        if start >= end {
            winding += i32::from(intersection.winding);
            span_start = intersection.x;
            continue;
        }
        let inside = match fill_rule {
            CoverageFillRule::NonZero => winding != 0,
            CoverageFillRule::EvenOdd => winding % 2 != 0,
        };
        if inside {
            has_coverage |= fill_coverage_span(row_bitmap, start, end, sample_grid);
        }
        winding += i32::from(intersection.winding);
        span_start = intersection.x;
    }

    has_coverage
}

fn horizontal_coverage_samples(start: f32, end: f32, column_start: f32, sample_grid: u32) -> u32 {
    let grid = sample_grid as f32;
    let first = (((start - column_start) * grid) - 0.5)
        .ceil()
        .clamp(0.0, grid) as u32;
    let last = (((end - column_start) * grid) - 0.5)
        .ceil()
        .clamp(0.0, grid) as u32;
    last.saturating_sub(first)
}

#[cfg(test)]
mod tests;
