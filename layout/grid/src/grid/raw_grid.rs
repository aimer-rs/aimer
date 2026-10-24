use std::cell::RefCell;
use std::fmt::{Display, Formatter};
use std::rc::Rc;

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, Drawable, Element, ErrorElement, EventElement, LayoutElement, Rebuildable,
    VisitorElement, detect_overflow,
};

use super::implementation::{GridAlignment, GridOverflow};

#[derive(Clone, Copy, Debug, PartialEq, aimer_macro::PortableValue)]
#[portable_value(id = "aimer.value:aimer_grid::grid::GridTrack", max_encoded_bytes = 32)]
pub enum GridTrack {
    #[portable_value(tag = 0)]
    Px(f32),
    #[portable_value(tag = 1)]
    Fr(f32),
    #[portable_value(tag = 2)]
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_grid::grid::GridPlacement",
    max_encoded_bytes = 128
)]
pub struct GridPlacement {
    pub row: Option<usize>,
    pub column: Option<usize>,
    pub row_span: usize,
    pub column_span: usize,
}

impl Default for GridPlacement {
    fn default() -> Self {
        Self {
            row: None,
            column: None,
            row_span: 1,
            column_span: 1,
        }
    }
}

impl GridPlacement {
    pub fn at(mut self, row: usize, column: usize) -> Self {
        self.row = Some(row);
        self.column = Some(column);
        self
    }

    pub fn row(mut self, row: usize) -> Self {
        self.row = Some(row);
        self
    }

    pub fn column(mut self, column: usize) -> Self {
        self.column = Some(column);
        self
    }

    pub fn row_span(mut self, span: usize) -> Self {
        self.row_span = span;
        self
    }

    pub fn column_span(mut self, span: usize) -> Self {
        self.column_span = span;
        self
    }

    pub(crate) fn resolved(row: usize, column: usize, row_span: usize, column_span: usize) -> Self {
        Self {
            row: Some(row),
            column: Some(column),
            row_span,
            column_span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GridError {
    MissingColumns,
    ZeroSpan {
        item: usize,
    },
    ColumnOutOfRange {
        item: usize,
        column: usize,
        span: usize,
        columns: usize,
    },
    OverlappingItems {
        first: usize,
        second: usize,
    },
    InvalidPixels {
        axis: &'static str,
        index: usize,
        value: f32,
    },
    InvalidFraction {
        axis: &'static str,
        index: usize,
        value: f32,
    },
    UnboundedFractionalTrack {
        axis: &'static str,
    },
}

impl Display for GridError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingColumns => formatter.write_str("Grid requires at least one column"),
            Self::ZeroSpan { item } => {
                write!(formatter, "Grid item {item} has a zero row or column span")
            }
            Self::ColumnOutOfRange {
                item,
                column,
                span,
                columns,
            } => write!(
                formatter,
                "Grid item {item} starts at column {column} with span {span}, outside {columns} columns"
            ),
            Self::OverlappingItems { first, second } => {
                write!(formatter, "Grid items {first} and {second} overlap")
            }
            Self::InvalidPixels { axis, index, value } => {
                write!(
                    formatter,
                    "Grid {axis} track {index} has invalid pixel size {value}"
                )
            }
            Self::InvalidFraction { axis, index, value } => {
                write!(
                    formatter,
                    "Grid {axis} track {index} has invalid fraction {value}"
                )
            }
            Self::UnboundedFractionalTrack { axis } => {
                write!(
                    formatter,
                    "Grid cannot resolve fractional {axis} tracks on an unbounded axis"
                )
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPlacements {
    pub items: Vec<GridPlacement>,
    pub row_count: usize,
}

fn ensure_rows(occupied: &mut Vec<Vec<Option<usize>>>, rows: usize, columns: usize) {
    occupied.resize_with(rows, || vec![None; columns]);
}

fn fits(
    occupied: &[Vec<Option<usize>>],
    row: usize,
    column: usize,
    row_span: usize,
    column_span: usize,
    columns: usize,
) -> bool {
    column + column_span <= columns
        && (row..row + row_span).all(|r| {
            occupied.get(r).is_none_or(|cells| {
                cells[column..column + column_span]
                    .iter()
                    .all(Option::is_none)
            })
        })
}

fn occupy(
    occupied: &mut Vec<Vec<Option<usize>>>,
    placement: GridPlacement,
    item: usize,
    columns: usize,
) {
    let row = placement.row.unwrap();
    let column = placement.column.unwrap();
    ensure_rows(occupied, row + placement.row_span, columns);
    for cells in occupied.iter_mut().skip(row).take(placement.row_span) {
        for slot in cells.iter_mut().skip(column).take(placement.column_span) {
            *slot = Some(item);
        }
    }
}

pub(crate) fn resolve_placements(
    items: &[GridPlacement],
    columns: usize,
    explicit_rows: usize,
) -> Result<ResolvedPlacements, GridError> {
    if columns == 0 {
        return Err(GridError::MissingColumns);
    }

    let items = items.to_vec();
    let mut occupied = Vec::new();
    ensure_rows(&mut occupied, explicit_rows, columns);
    let mut resolved = vec![GridPlacement::default(); items.len()];

    for (index, placement) in items.iter().copied().enumerate() {
        if placement.row_span == 0 || placement.column_span == 0 {
            return Err(GridError::ZeroSpan { item: index });
        }
        if let Some(column) = placement.column
            && column + placement.column_span > columns
        {
            return Err(GridError::ColumnOutOfRange {
                item: index,
                column,
                span: placement.column_span,
                columns,
            });
        }
        if placement.row.is_none() && placement.column.is_none() {
            continue;
        }

        let (row, column) = match (placement.row, placement.column) {
            (Some(row), Some(column)) => {
                ensure_rows(&mut occupied, row + placement.row_span, columns);
                if !fits(
                    &occupied,
                    row,
                    column,
                    placement.row_span,
                    placement.column_span,
                    columns,
                ) {
                    let first = occupied
                        .iter()
                        .skip(row)
                        .take(placement.row_span)
                        .flat_map(|cells| cells.iter().skip(column).take(placement.column_span))
                        .find_map(|slot| *slot)
                        .unwrap();
                    return Err(GridError::OverlappingItems {
                        first,
                        second: index,
                    });
                }
                (row, column)
            }
            (Some(row), None) => {
                ensure_rows(&mut occupied, row + placement.row_span, columns);
                let column = (0..columns)
                    .find(|column| {
                        fits(
                            &occupied,
                            row,
                            *column,
                            placement.row_span,
                            placement.column_span,
                            columns,
                        )
                    })
                    .ok_or(GridError::ColumnOutOfRange {
                        item: index,
                        column: 0,
                        span: placement.column_span,
                        columns,
                    })?;
                (row, column)
            }
            (None, Some(column)) => {
                let row = (0..)
                    .find(|row| {
                        fits(
                            &occupied,
                            *row,
                            column,
                            placement.row_span,
                            placement.column_span,
                            columns,
                        )
                    })
                    .unwrap();
                (row, column)
            }
            (None, None) => unreachable!(),
        };
        let placement =
            GridPlacement::resolved(row, column, placement.row_span, placement.column_span);
        occupy(&mut occupied, placement, index, columns);
        resolved[index] = placement;
    }

    let mut cursor = 0;
    for (index, placement) in items.iter().copied().enumerate() {
        if placement.row.is_some() || placement.column.is_some() {
            continue;
        }
        let (row, column) = (cursor..)
            .map(|cell| (cell / columns, cell % columns))
            .find(|(row, column)| {
                fits(
                    &occupied,
                    *row,
                    *column,
                    placement.row_span,
                    placement.column_span,
                    columns,
                )
            })
            .unwrap();
        let placement =
            GridPlacement::resolved(row, column, placement.row_span, placement.column_span);
        occupy(&mut occupied, placement, index, columns);
        resolved[index] = placement;
        cursor = row * columns + column + 1;
    }

    Ok(ResolvedPlacements {
        items: resolved,
        row_count: occupied.len().max(explicit_rows),
    })
}

pub(crate) fn resolve_tracks(
    tracks: &[GridTrack],
    available: f32,
    gap: f32,
    auto_minima: &[f32],
    axis: &'static str,
) -> Result<Vec<f32>, GridError> {
    let mut resolved = vec![0.0; tracks.len()];
    let mut consumed = gap.max(0.0) * tracks.len().saturating_sub(1) as f32;
    let mut fraction_sum = 0.0;

    for (index, track) in tracks.iter().copied().enumerate() {
        match track {
            GridTrack::Px(value) if !value.is_finite() || value < 0.0 => {
                return Err(GridError::InvalidPixels { axis, index, value });
            }
            GridTrack::Px(value) => {
                resolved[index] = value;
                consumed += value;
            }
            GridTrack::Fr(value) if !value.is_finite() || value <= 0.0 => {
                return Err(GridError::InvalidFraction { axis, index, value });
            }
            GridTrack::Fr(value) => fraction_sum += value,
            GridTrack::Auto => {
                let value = auto_minima.get(index).copied().unwrap_or(0.0).max(0.0);
                resolved[index] = value;
                consumed += value;
            }
        }
    }

    if fraction_sum > 0.0 {
        if available == f32::MAX || !available.is_finite() {
            return Err(GridError::UnboundedFractionalTrack { axis });
        }
        let unit = (available - consumed).max(0.0) / fraction_sum;
        for (index, track) in tracks.iter().enumerate() {
            if let GridTrack::Fr(value) = track {
                resolved[index] = unit * value;
            }
        }
    }

    Ok(resolved)
}

fn apply_auto_minimum(
    tracks: &[GridTrack],
    minima: &mut [f32],
    start: usize,
    span: usize,
    desired: f32,
    gap: f32,
) {
    let covered = &tracks[start..start + span];
    if covered
        .iter()
        .any(|track| matches!(track, GridTrack::Fr(_)))
    {
        return;
    }
    let fixed = covered
        .iter()
        .filter_map(|track| match track {
            GridTrack::Px(value) => Some(*value),
            GridTrack::Fr(_) | GridTrack::Auto => None,
        })
        .sum::<f32>();
    let auto_count = covered
        .iter()
        .filter(|track| **track == GridTrack::Auto)
        .count();
    if auto_count == 0 {
        return;
    }
    let contribution =
        (desired - fixed - gap * span.saturating_sub(1) as f32).max(0.0) / auto_count as f32;
    for index in start..start + span {
        if tracks[index] == GridTrack::Auto {
            minima[index] = minima[index].max(contribution);
        }
    }
}

pub(crate) struct RawGridItem {
    pub child: AnyElement,
    pub placement: GridPlacement,
    pub horizontal_alignment: Option<GridAlignment>,
    pub vertical_alignment: Option<GridAlignment>,
}

pub(crate) struct RawGrid {
    pub columns: Vec<GridTrack>,
    pub rows: Vec<GridTrack>,
    pub column_gap: f32,
    pub row_gap: f32,
    pub horizontal_alignment: GridAlignment,
    pub vertical_alignment: GridAlignment,
    pub overflow: GridOverflow,
    pub children: Vec<RawGridItem>,
    pub layout_cache: RefCell<Vec<(BoxConstraint, u32, u64, Rc<GridLayout>)>>,
}

#[derive(Clone)]
pub(crate) struct GridLayout {
    placements: Vec<GridPlacement>,
    columns: Vec<f32>,
    rows: Vec<f32>,
    size: ResolvedSize,
}

impl RawGrid {
    fn layout_grid(&self, ctx: &BuildContext) -> Result<Rc<GridLayout>, GridError> {
        let layout_generation = aimer_widget::layout_invalidation_generation();
        if let Some((_, _, _, layout)) =
            self.layout_cache
                .borrow()
                .iter()
                .find(|(constraint, scale_bits, generation, _)| {
                    *constraint == ctx.box_constraint
                        && *scale_bits == ctx.scale.to_bits()
                        && *generation == layout_generation
                })
        {
            return Ok(Rc::clone(layout));
        }
        let placements = self
            .children
            .iter()
            .map(|item| item.placement)
            .collect::<Vec<_>>();
        let resolved_placements =
            resolve_placements(&placements, self.columns.len(), self.rows.len())?;
        let mut rows = self.rows.clone();
        rows.resize(resolved_placements.row_count, GridTrack::Auto);

        let mut column_minima = vec![0.0_f32; self.columns.len()];
        if self.columns.contains(&GridTrack::Auto) {
            for (item, placement) in self.children.iter().zip(&resolved_placements.items) {
                let mut child_ctx = ctx.clone();
                child_ctx.parent_size = ResolvedSize::default();
                child_ctx.box_constraint = BoxConstraint {
                    min_width: 0.0,
                    min_height: 0.0,
                    max_width: f32::MAX,
                    max_height: f32::MAX,
                };
                let size = item.child.computed_size(&child_ctx);
                let start = placement.column.unwrap();
                apply_auto_minimum(
                    &self.columns,
                    &mut column_minima,
                    start,
                    placement.column_span,
                    size.width,
                    self.column_gap,
                );
            }
        }
        let columns = resolve_tracks(
            &self.columns,
            ctx.box_constraint.max_width,
            self.column_gap,
            &column_minima,
            "columns",
        )?;

        let mut row_minima = vec![0.0_f32; rows.len()];
        for (item, placement) in self.children.iter().zip(&resolved_placements.items) {
            let cell_width = span_size(
                &columns,
                placement.column.unwrap(),
                placement.column_span,
                self.column_gap,
            );
            let mut child_ctx = ctx.clone();
            child_ctx.parent_size = ResolvedSize {
                width: cell_width,
                height: 0.0,
            };
            child_ctx.box_constraint = BoxConstraint {
                min_width: 0.0,
                min_height: 0.0,
                max_width: cell_width,
                max_height: f32::MAX,
            };
            let size = item.child.computed_size(&child_ctx);
            let start = placement.row.unwrap();
            apply_auto_minimum(
                &rows,
                &mut row_minima,
                start,
                placement.row_span,
                size.height,
                self.row_gap,
            );
        }
        let rows = resolve_tracks(
            &rows,
            ctx.box_constraint.max_height,
            self.row_gap,
            &row_minima,
            "rows",
        )?;
        let size = ResolvedSize {
            width: tracks_size(&columns, self.column_gap),
            height: tracks_size(&rows, self.row_gap),
        };

        let layout = Rc::new(GridLayout {
            placements: resolved_placements.items,
            columns,
            rows,
            size,
        });
        let mut cache = self.layout_cache.borrow_mut();
        cache.retain(|(_, _, generation, _)| *generation == layout_generation);
        cache.push((
            ctx.box_constraint,
            ctx.scale.to_bits(),
            layout_generation,
            Rc::clone(&layout),
        ));
        Ok(layout)
    }

    fn draw_item(
        &self,
        ctx: &BuildContext,
        item: &RawGridItem,
        placement: GridPlacement,
        layout: &GridLayout,
    ) {
        let cell_pos = Vec2d {
            x: track_offset(&layout.columns, placement.column.unwrap(), self.column_gap),
            y: track_offset(&layout.rows, placement.row.unwrap(), self.row_gap),
        };
        let cell_size = ResolvedSize {
            width: span_size(
                &layout.columns,
                placement.column.unwrap(),
                placement.column_span,
                self.column_gap,
            ),
            height: span_size(
                &layout.rows,
                placement.row.unwrap(),
                placement.row_span,
                self.row_gap,
            ),
        };
        let horizontal = item
            .horizontal_alignment
            .unwrap_or(self.horizontal_alignment);
        let vertical = item.vertical_alignment.unwrap_or(self.vertical_alignment);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = cell_size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: if horizontal == GridAlignment::Stretch {
                cell_size.width
            } else {
                0.0
            },
            min_height: if vertical == GridAlignment::Stretch {
                cell_size.height
            } else {
                0.0
            },
            max_width: cell_size.width,
            max_height: cell_size.height,
        };
        let child_size =
            if horizontal == GridAlignment::Stretch && vertical == GridAlignment::Stretch {
                cell_size
            } else {
                item.child.computed_size(&child_ctx)
            };
        let offset = Vec2d {
            x: alignment_offset(horizontal, cell_size.width, child_size.width),
            y: alignment_offset(vertical, cell_size.height, child_size.height),
        };
        child_ctx.visible_rect = ctx.visible_rect.map(|(x, y, width, height)| {
            (
                x - cell_pos.x - offset.x,
                y - cell_pos.y - offset.y,
                width,
                height,
            )
        });
        let overflow = detect_overflow(child_size, cell_size, offset);

        ctx.canvas.save();
        ctx.canvas.translate(cell_pos);
        ctx.canvas.save();
        ctx.canvas.translate(offset);
        item.child.update(&child_ctx);
        ctx.canvas.restore();
        ctx.canvas.restore();
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        Vec2d,
        Vec2d,
        BuildContext<'a>,
        ResolvedSize,
        ResolvedSize,
        ResolvedSize,
        aimer_widget::OverflowEdges,
    )> {
        let item = self.children.get(child_index)?;
        if item.child.id() != child.id() {
            return None;
        }
        let layout = self.layout_grid(ctx).ok()?;
        let placement = *layout.placements.get(child_index)?;
        let cell_pos = Vec2d {
            x: track_offset(&layout.columns, placement.column?, self.column_gap),
            y: track_offset(&layout.rows, placement.row?, self.row_gap),
        };
        let cell_size = ResolvedSize {
            width: span_size(
                &layout.columns,
                placement.column?,
                placement.column_span,
                self.column_gap,
            ),
            height: span_size(
                &layout.rows,
                placement.row?,
                placement.row_span,
                self.row_gap,
            ),
        };
        let horizontal = item
            .horizontal_alignment
            .unwrap_or(self.horizontal_alignment);
        let vertical = item.vertical_alignment.unwrap_or(self.vertical_alignment);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = cell_size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: if horizontal == GridAlignment::Stretch {
                cell_size.width
            } else {
                0.0
            },
            min_height: if vertical == GridAlignment::Stretch {
                cell_size.height
            } else {
                0.0
            },
            max_width: cell_size.width,
            max_height: cell_size.height,
        };
        let child_size =
            if horizontal == GridAlignment::Stretch && vertical == GridAlignment::Stretch {
                cell_size
            } else {
                child.computed_size(&child_ctx)
            };
        let offset = Vec2d {
            x: alignment_offset(horizontal, cell_size.width, child_size.width),
            y: alignment_offset(vertical, cell_size.height, child_size.height),
        };
        child_ctx.visible_rect = ctx.visible_rect.map(|(x, y, width, height)| {
            (
                x - cell_pos.x - offset.x,
                y - cell_pos.y - offset.y,
                width,
                height,
            )
        });
        let overflow = detect_overflow(child_size, cell_size, offset);
        let render_size = child
            .retained_v2_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        Some((
            cell_pos,
            offset,
            child_ctx,
            cell_size,
            child_size,
            render_size,
            overflow,
        ))
    }
}

impl Drawable for RawGrid {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return false;
        }
        let layout = match self.layout_grid(ctx) {
            Ok(layout) => layout,
            // A grid that cannot be laid out shows the reason in its own list.
            Err(_) => return ErrorElement::can_record_message(ctx),
        };
        if !layout.size.width.is_finite()
            || !layout.size.height.is_finite()
            || layout.size.width < 0.0
            || layout.size.height < 0.0
            || layout.size.width > 1_000_000.0
            || layout.size.height > 1_000_000.0
        {
            return false;
        }
        self.children.iter().enumerate().all(|(index, item)| {
            let Some((cell_pos, offset, child_ctx, cell_size, child_size, render_size, overflow)) =
                self.retained_child_layout(ctx, item.child.as_ref(), index)
            else {
                return false;
            };
            let position = item.child.pos().unwrap_or_default();
            [
                cell_pos.x,
                cell_pos.y,
                offset.x,
                offset.y,
                position.x,
                position.y,
                cell_size.width,
                cell_size.height,
                child_size.width,
                child_size.height,
                render_size.width,
                render_size.height,
            ]
            .into_iter()
            .all(f32::is_finite)
                && cell_size.width >= 0.0
                && cell_size.height >= 0.0
                && cell_size.width <= 1_000_000.0
                && cell_size.height <= 1_000_000.0
                && child_size.width >= 0.0
                && child_size.height >= 0.0
                && render_size.width >= 0.0
                && render_size.height >= 0.0
                && render_size.width <= 1_000_000.0
                && render_size.height <= 1_000_000.0
                && !overflow.has_overflow()
                && item.child.retained_clip(&child_ctx).is_none_or(|clip| {
                    clip.x.is_finite()
                        && clip.y.is_finite()
                        && clip.width.is_finite()
                        && clip.height.is_finite()
                        && clip.width >= 0.0
                        && clip.height >= 0.0
                })
        })
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        if let Err(error) = self.layout_grid(ctx) {
            ErrorElement::record_message(ctx, &format!("Grid layout error: {error}"));
        }
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.retained_child_layout(ctx, child, child_index)
            .map(|(_, _, child_ctx, _, _, _, _)| child_ctx)
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        let (cell_pos, offset, child_ctx, cell_size, _, render_size, _) =
            self.retained_child_layout(ctx, child, child_index)?;
        let position = child.pos().unwrap_or_default();
        let scale = ctx.scale;
        let bounds = aimer_cupid::draw_cmd_v2::Rect::new(
            (cell_pos.x + offset.x + position.x) / scale,
            (cell_pos.y + offset.y + position.y) / scale,
            render_size.width / scale,
            render_size.height / scale,
        );
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
        {
            return None;
        }

        let grid_clip = (self.overflow == GridOverflow::Clip).then(|| {
            aimer_cupid::draw_cmd_v2::Rect::new(
                -(offset.x + position.x) / scale,
                -(offset.y + position.y) / scale,
                cell_size.width / scale,
                cell_size.height / scale,
            )
        });
        let child_clip = child.retained_clip(&child_ctx);
        let clip = match (grid_clip, child_clip) {
            (Some(grid), Some(child)) => Some(intersect_retained_clips(grid, child)),
            (Some(grid), None) => Some(grid),
            (None, child) => child,
        };
        Some((bounds, clip))
    }

    fn update(&self, ctx: &BuildContext) {
        match self.layout_grid(ctx) {
            Ok(layout) => {
                for (item, placement) in self.children.iter().zip(&layout.placements) {
                    let cell_pos = Vec2d {
                        x: track_offset(
                            &layout.columns,
                            placement.column.unwrap(),
                            self.column_gap,
                        ),
                        y: track_offset(&layout.rows, placement.row.unwrap(), self.row_gap),
                    };
                    let cell_size = ResolvedSize {
                        width: span_size(
                            &layout.columns,
                            placement.column.unwrap(),
                            placement.column_span,
                            self.column_gap,
                        ),
                        height: span_size(
                            &layout.rows,
                            placement.row.unwrap(),
                            placement.row_span,
                            self.row_gap,
                        ),
                    };
                    if self.overflow == GridOverflow::Clip
                        && !rect_intersects_visible_rect(cell_pos, cell_size, ctx.visible_rect)
                    {
                        continue;
                    }
                    self.draw_item(ctx, item, *placement, &layout);
                }
            }
            // The message is recorded by `paint_local_v2`; there is no child to
            // visit when the layout failed.
            Err(_) => {}
        }
    }
}

impl EventElement for RawGrid {
    /// The event and visual child views expose the same grid-item children.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for item in &self.children {
            visitor(item.child.as_ref());
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for item in &self.children {
            visitor(item.child.as_ref());
        }
    }

    /// Visits grid items in reverse retained order so routed pointer events do
    /// not allocate a temporary sibling buffer for large grids.
    #[inline]
    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for item in self.children.iter().rev() {
            visitor(item.child.as_ref());
        }
    }

    /// Offers only grid children whose retained bounds contain `pos`.
    ///
    /// A child that has not published bounds remains a candidate; unknown
    /// geometry must not make a custom grid child unreachable.
    #[inline]
    fn hit_test_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for item in &self.children {
            if hit_test_contains(item.child.as_ref(), pos) {
                visitor(item.child.as_ref());
            }
        }
    }

    /// Offers bounded grid children in the topmost-first order used by routed
    /// pointer dispatch.
    #[inline]
    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        for item in self.children.iter().rev() {
            if hit_test_contains(item.child.as_ref(), pos) {
                visitor(item.child.as_ref());
            }
        }
    }
}

impl Rebuildable for RawGrid {}

impl VisitorElement for RawGrid {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for item in &self.children {
            visitor(item.child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "Grid"
    }
}

impl LayoutElement for RawGrid {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.layout_grid(ctx)
            .map_or_else(|_| fallback_size(ctx), |layout| layout.size)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    /// Only a size this grid was given: its extent comes from tracks over all of
    /// its cells, which the default — the largest child — would understate.
    fn get_size_from_child(&self) -> Option<aimer_attribute::Size> {
        self.size()
    }

    fn invalidate_layout(&self) {
        self.layout_cache.borrow_mut().clear();
        for item in &self.children {
            item.child.invalidate_layout();
        }
    }
}

fn rect_intersects_visible_rect(
    position: Vec2d,
    size: ResolvedSize,
    visible_rect: Option<(f32, f32, f32, f32)>,
) -> bool {
    let Some((visible_x, visible_y, visible_width, visible_height)) = visible_rect else {
        return true;
    };
    position.x + size.width >= visible_x
        && position.x <= visible_x + visible_width
        && position.y + size.height >= visible_y
        && position.y <= visible_y + visible_height
}

#[inline]
fn hit_test_contains(element: &dyn Element, pos: Vec2d) -> bool {
    element.pos_start_end().is_none_or(|(start, end)| {
        pos.x >= start.x && pos.x <= end.x && pos.y >= start.y && pos.y <= end.y
    })
}

fn tracks_size(tracks: &[f32], gap: f32) -> f32 {
    tracks.iter().sum::<f32>() + gap * tracks.len().saturating_sub(1) as f32
}

fn span_size(tracks: &[f32], start: usize, span: usize, gap: f32) -> f32 {
    tracks[start..start + span].iter().sum::<f32>() + gap * span.saturating_sub(1) as f32
}

fn track_offset(tracks: &[f32], index: usize, gap: f32) -> f32 {
    tracks[..index].iter().sum::<f32>() + gap * index as f32
}

fn intersect_retained_clips(
    first: aimer_cupid::draw_cmd_v2::Rect,
    second: aimer_cupid::draw_cmd_v2::Rect,
) -> aimer_cupid::draw_cmd_v2::Rect {
    let x = first.x.max(second.x);
    let y = first.y.max(second.y);
    let right = (first.x + first.width).min(second.x + second.width);
    let bottom = (first.y + first.height).min(second.y + second.height);
    aimer_cupid::draw_cmd_v2::Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

fn alignment_offset(alignment: GridAlignment, available: f32, child: f32) -> f32 {
    match alignment {
        GridAlignment::Start | GridAlignment::Stretch => 0.0,
        GridAlignment::Center => (available - child) / 2.0,
        GridAlignment::End => available - child,
    }
}

fn fallback_size(ctx: &BuildContext) -> ResolvedSize {
    ResolvedSize {
        width: if ctx.box_constraint.max_width == f32::MAX {
            ctx.parent_size.width
        } else {
            ctx.box_constraint.max_width
        },
        height: if ctx.box_constraint.max_height == f32::MAX {
            ctx.parent_size.height
        } else {
            ctx.box_constraint.max_height
        },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::OnceLock;

    use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
    use aimer_canvas::{FrameCanvas, InnerCanvas};
    use aimer_container::ZeroSizedBox;
    use aimer_cupid::draw_cmd::DrawCommand;
    use aimer_widget::base::{BuildContext, WindowHandle};
    use aimer_widget::{
        AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
    };

    use super::{
        GridAlignment, GridError, GridOverflow, GridPlacement, GridTrack, RawGrid, RawGridItem,
        apply_auto_minimum, resolve_placements, resolve_tracks,
    };

    type RecordedVisibleRect = Rc<RefCell<Option<(f32, f32, f32, f32)>>>;

    struct VisibleRectRecorder {
        visible_rect: RecordedVisibleRect,
    }

    struct WorkRecorder {
        computed_count: Rc<RefCell<usize>>,
        draw_count: Rc<RefCell<usize>>,
        size: ResolvedSize,
    }

    struct HitTestChild {
        bounds: (Vec2d, Vec2d),
    }

    impl HitTestChild {
        fn boxed(bounds: (Vec2d, Vec2d)) -> AnyElement {
            Self { bounds }.boxed()
        }
    }

    impl Drawable for HitTestChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for HitTestChild {}
    impl LayoutElement for HitTestChild {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some(self.bounds)
        }
    }
    impl Rebuildable for HitTestChild {}
    impl VisitorElement for HitTestChild {
        fn debug_name(&self) -> &'static str {
            "HitTestChild"
        }
    }

    impl Drawable for WorkRecorder {
        fn update(&self, _ctx: &BuildContext) {
            *self.draw_count.borrow_mut() += 1;
        }
    }

    impl EventElement for WorkRecorder {}
    impl LayoutElement for WorkRecorder {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            *self.computed_count.borrow_mut() += 1;
            self.size
        }
    }
    impl Rebuildable for WorkRecorder {}
    impl VisitorElement for WorkRecorder {
        fn debug_name(&self) -> &'static str {
            "WorkRecorder"
        }
    }

    impl Drawable for VisibleRectRecorder {
        fn update(&self, ctx: &BuildContext) {
            *self.visible_rect.borrow_mut() = ctx.visible_rect;
        }
    }

    impl EventElement for VisibleRectRecorder {}
    impl LayoutElement for VisibleRectRecorder {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 20.0,
                height: 20.0,
            }
        }
    }
    impl Rebuildable for VisibleRectRecorder {}
    impl VisitorElement for VisibleRectRecorder {
        fn debug_name(&self) -> &'static str {
            "VisibleRectRecorder"
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
            })
            .handle()
            .clone()
    }

    fn build_context(canvas: &'static InnerCanvas) -> BuildContext<'static> {
        let mut context = BuildContext::new(
            FrameCanvas::new(canvas),
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(winit::dpi::PhysicalSize::new(200, 100), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 200.0,
            max_height: 100.0,
        };
        context
    }

    #[test]
    fn a_grid_that_cannot_be_laid_out_records_the_reason_in_its_own_list() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let mut ctx = build_context(canvas);
        // A fractional column has nothing to divide in an unbounded axis.
        ctx.box_constraint.max_width = f32::MAX;
        let grid = RawGrid {
            columns: vec![GridTrack::Fr(1.0)],
            rows: vec![GridTrack::Auto],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Visible,
            children: vec![RawGridItem {
                child: Element::boxed(ZeroSizedBox),
                placement: GridPlacement::default(),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };
        assert!(grid.layout_grid(&ctx).is_err());

        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 200.0, 100.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            assert!(grid.can_paint_local_v2(ctx));
            grid.paint_local_v2(ctx);
        });

        let message = tree
            .draw_list_snapshot(root)
            .unwrap()
            .commands
            .iter()
            .find_map(|command| match command {
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { text, .. } => {
                    Some(text.to_string())
                }
                _ => None,
            });
        // Release builds replace the detail with a generic line.
        assert!(message.is_some(), "the failure must be visible");
        #[cfg(debug_assertions)]
        assert!(message.unwrap().contains("Grid layout error"));
    }

    #[test]
    fn grid_shifts_visible_rect_into_child_cell_coordinates() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let mut ctx = build_context(canvas);
        ctx.visible_rect = Some((0.0, 60.0, 200.0, 20.0));
        let visible_rect = Rc::new(RefCell::new(None));
        let grid = RawGrid {
            columns: vec![GridTrack::Px(200.0)],
            rows: vec![GridTrack::Px(50.0), GridTrack::Px(50.0)],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Center,
            vertical_alignment: GridAlignment::Center,
            overflow: GridOverflow::Clip,
            children: vec![RawGridItem {
                child: VisibleRectRecorder {
                    visible_rect: visible_rect.clone(),
                }
                .boxed(),
                placement: GridPlacement::default().at(1, 0),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.update(&ctx);

        assert_eq!(*visible_rect.borrow(), Some((-90.0, -5.0, 200.0, 20.0)));
    }

    #[test]
    fn grid_reuses_layout_between_measurement_and_draw() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let ctx = build_context(canvas);
        let computed_count = Rc::new(RefCell::new(0));
        let grid = RawGrid {
            columns: vec![GridTrack::Auto],
            rows: vec![GridTrack::Auto],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: vec![RawGridItem {
                child: WorkRecorder {
                    computed_count: computed_count.clone(),
                    draw_count: Rc::new(RefCell::new(0)),
                    size: ResolvedSize {
                        width: 80.0,
                        height: 30.0,
                    },
                }
                .boxed(),
                placement: GridPlacement::default(),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.computed_size(&ctx);
        assert_eq!(*computed_count.borrow(), 2);
        grid.update(&ctx);

        assert_eq!(*computed_count.borrow(), 2);

        grid.invalidate_layout();
        grid.computed_size(&ctx);
        assert_eq!(*computed_count.borrow(), 4);
    }

    #[test]
    fn fractional_columns_skip_unused_intrinsic_width_measurement() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let ctx = build_context(canvas);
        let computed_count = Rc::new(RefCell::new(0));
        let grid = RawGrid {
            columns: vec![GridTrack::Fr(1.0)],
            rows: vec![GridTrack::Auto],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: vec![RawGridItem {
                child: WorkRecorder {
                    computed_count: computed_count.clone(),
                    draw_count: Rc::new(RefCell::new(0)),
                    size: ResolvedSize {
                        width: 80.0,
                        height: 30.0,
                    },
                }
                .boxed(),
                placement: GridPlacement::default(),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.computed_size(&ctx);
        grid.update(&ctx);

        assert_eq!(*computed_count.borrow(), 1);
    }

    #[test]
    fn grid_reuses_each_constraint_variant_within_a_frame() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let ctx = build_context(canvas);
        let mut narrower_ctx = ctx.clone();
        narrower_ctx.box_constraint.max_width = 160.0;
        let computed_count = Rc::new(RefCell::new(0));
        let grid = RawGrid {
            columns: vec![GridTrack::Auto],
            rows: vec![GridTrack::Auto],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: vec![RawGridItem {
                child: WorkRecorder {
                    computed_count: computed_count.clone(),
                    draw_count: Rc::new(RefCell::new(0)),
                    size: ResolvedSize {
                        width: 80.0,
                        height: 30.0,
                    },
                }
                .boxed(),
                placement: GridPlacement::default(),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.computed_size(&ctx);
        grid.computed_size(&narrower_ctx);
        grid.computed_size(&ctx);

        assert_eq!(*computed_count.borrow(), 4);
    }

    #[test]
    fn grid_skips_cells_outside_visible_rect() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let mut ctx = build_context(canvas);
        ctx.visible_rect = Some((0.0, 0.0, 200.0, 40.0));
        let visible_draws = Rc::new(RefCell::new(0));
        let hidden_draws = Rc::new(RefCell::new(0));
        let child = |draw_count: Rc<RefCell<usize>>, row| RawGridItem {
            child: WorkRecorder {
                computed_count: Rc::new(RefCell::new(0)),
                draw_count,
                size: ResolvedSize {
                    width: 200.0,
                    height: 50.0,
                },
            }
            .boxed(),
            placement: GridPlacement::default().at(row, 0),
            horizontal_alignment: None,
            vertical_alignment: None,
        };
        let grid = RawGrid {
            columns: vec![GridTrack::Px(200.0)],
            rows: vec![GridTrack::Px(50.0), GridTrack::Px(50.0)],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: vec![
                child(visible_draws.clone(), 0),
                child(hidden_draws.clone(), 1),
            ],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.update(&ctx);

        assert_eq!(*visible_draws.borrow(), 1);
        assert_eq!(*hidden_draws.borrow(), 0);
    }

    #[test]
    fn position_aware_hit_testing_prunes_offscreen_grid_children() {
        let grid = RawGrid {
            columns: vec![GridTrack::Px(100.0)],
            rows: vec![
                GridTrack::Px(20.0),
                GridTrack::Px(20.0),
                GridTrack::Px(20.0),
            ],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: vec![
                RawGridItem {
                    child: HitTestChild::boxed((
                        Vec2d { x: 0.0, y: 0.0 },
                        Vec2d { x: 100.0, y: 20.0 },
                    )),
                    placement: GridPlacement::default().at(0, 0),
                    horizontal_alignment: None,
                    vertical_alignment: None,
                },
                RawGridItem {
                    child: HitTestChild::boxed((
                        Vec2d { x: 0.0, y: 20.0 },
                        Vec2d { x: 100.0, y: 40.0 },
                    )),
                    placement: GridPlacement::default().at(1, 0),
                    horizontal_alignment: None,
                    vertical_alignment: None,
                },
                RawGridItem {
                    child: HitTestChild::boxed((
                        Vec2d { x: 0.0, y: 40.0 },
                        Vec2d { x: 100.0, y: 60.0 },
                    )),
                    placement: GridPlacement::default().at(2, 0),
                    horizontal_alignment: None,
                    vertical_alignment: None,
                },
            ],
            layout_cache: RefCell::new(Vec::new()),
        };

        let pos = Vec2d { x: 50.0, y: 30.0 };
        let mut forward = 0;
        grid.hit_test_children_at(pos, &mut |_| forward += 1);
        assert_eq!(forward, 1);

        let mut reversed = 0;
        grid.hit_test_children_at_reversed(pos, &mut |_| reversed += 1);
        assert_eq!(reversed, 1);
    }

    #[test]
    fn visible_overflow_keeps_offscreen_cells_drawable() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let mut ctx = build_context(canvas);
        ctx.visible_rect = Some((0.0, 0.0, 200.0, 40.0));
        let draw_count = Rc::new(RefCell::new(0));
        let grid = RawGrid {
            columns: vec![GridTrack::Px(200.0)],
            rows: vec![GridTrack::Px(50.0), GridTrack::Px(50.0)],
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Visible,
            children: vec![RawGridItem {
                child: WorkRecorder {
                    computed_count: Rc::new(RefCell::new(0)),
                    draw_count: draw_count.clone(),
                    size: ResolvedSize {
                        width: 200.0,
                        height: 50.0,
                    },
                }
                .boxed(),
                placement: GridPlacement::default().at(1, 0),
                horizontal_alignment: None,
                vertical_alignment: None,
            }],
            layout_cache: RefCell::new(Vec::new()),
        };

        grid.update(&ctx);

        assert_eq!(*draw_count.borrow(), 1);
    }

    #[test]
    fn explicit_items_reserve_cells_before_sparse_auto_placement() {
        let items = [
            GridPlacement::default(),
            GridPlacement::default().at(0, 1),
            GridPlacement::default(),
            GridPlacement::default(),
        ];

        let layout = resolve_placements(&items, 2, 1).unwrap();

        assert_eq!(layout.row_count, 2);
        assert_eq!(layout.items[0], GridPlacement::resolved(0, 0, 1, 1));
        assert_eq!(layout.items[1], GridPlacement::resolved(0, 1, 1, 1));
        assert_eq!(layout.items[2], GridPlacement::resolved(1, 0, 1, 1));
        assert_eq!(layout.items[3], GridPlacement::resolved(1, 1, 1, 1));
    }

    #[test]
    fn sparse_auto_placement_does_not_backfill_holes_before_cursor() {
        let items = [
            GridPlacement::default().column_span(2),
            GridPlacement::default(),
            GridPlacement::default().at(0, 1),
        ];

        let layout = resolve_placements(&items, 3, 1).unwrap();

        assert_eq!(layout.items[0], GridPlacement::resolved(1, 0, 1, 2));
        assert_eq!(layout.items[1], GridPlacement::resolved(1, 2, 1, 1));
        assert_eq!(layout.items[2], GridPlacement::resolved(0, 1, 1, 1));
    }

    #[test]
    fn invalid_spans_and_explicit_overlaps_are_reported() {
        let zero_span = resolve_placements(&[GridPlacement::default().column_span(0)], 2, 1);
        assert_eq!(zero_span, Err(GridError::ZeroSpan { item: 0 }));

        let overlap = resolve_placements(
            &[
                GridPlacement::default().at(0, 0),
                GridPlacement::default().at(0, 0),
            ],
            2,
            1,
        );
        assert_eq!(
            overlap,
            Err(GridError::OverlappingItems {
                first: 0,
                second: 1
            })
        );

        let outside = resolve_placements(&[GridPlacement::default().at(0, 1).column_span(2)], 2, 1);
        assert_eq!(
            outside,
            Err(GridError::ColumnOutOfRange {
                item: 0,
                column: 1,
                span: 2,
                columns: 2
            })
        );
    }

    #[test]
    fn tracks_resolve_fixed_auto_and_fractional_space() {
        let tracks = [
            GridTrack::Px(20.0),
            GridTrack::Auto,
            GridTrack::Fr(1.0),
            GridTrack::Fr(2.0),
        ];

        let resolved =
            resolve_tracks(&tracks, 140.0, 5.0, &[0.0, 30.0, 0.0, 0.0], "columns").unwrap();

        assert_eq!(resolved, vec![20.0, 30.0, 25.0, 50.0]);
    }

    #[test]
    fn fractional_tracks_require_a_bounded_axis() {
        let result = resolve_tracks(&[GridTrack::Fr(1.0)], f32::MAX, 0.0, &[0.0], "columns");

        assert_eq!(
            result,
            Err(GridError::UnboundedFractionalTrack { axis: "columns" })
        );
    }

    #[test]
    fn track_values_must_be_valid() {
        assert_eq!(
            resolve_tracks(&[GridTrack::Fr(0.0)], 100.0, 0.0, &[0.0], "columns"),
            Err(GridError::InvalidFraction {
                axis: "columns",
                index: 0,
                value: 0.0
            })
        );
        assert_eq!(
            resolve_tracks(&[GridTrack::Px(-1.0)], 100.0, 0.0, &[0.0], "columns"),
            Err(GridError::InvalidPixels {
                axis: "columns",
                index: 0,
                value: -1.0
            })
        );
    }

    #[test]
    fn spanning_content_subtracts_fixed_tracks_before_sizing_auto_tracks() {
        let tracks = [GridTrack::Px(20.0), GridTrack::Auto, GridTrack::Auto];
        let mut minima = vec![0.0; tracks.len()];

        apply_auto_minimum(&tracks, &mut minima, 0, 3, 120.0, 5.0);

        assert_eq!(minima, vec![0.0, 45.0, 45.0]);
    }
}
