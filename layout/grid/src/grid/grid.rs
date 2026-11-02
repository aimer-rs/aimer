use std::cell::RefCell;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, Widget};

use super::raw_grid::{
    GridPlacement, GridTrack, RawGrid, RawGridItem, resolve_placements, resolve_tracks,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridAlignment {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GridOverflow {
    #[default]
    Clip,
    Visible,
}

pub struct GridItem<W: Widget + 'static> {
    pub(crate) child: W,
    pub(crate) placement: GridPlacement,
    pub(crate) horizontal_alignment: Option<GridAlignment>,
    pub(crate) vertical_alignment: Option<GridAlignment>,
}

impl<W: Widget + 'static> GridItem<W> {
    pub fn new(child: W) -> Self {
        Self {
            child,
            placement: GridPlacement::default(),
            horizontal_alignment: None,
            vertical_alignment: None,
        }
    }

    pub fn row(mut self, row: usize) -> Self {
        self.placement.row = Some(row);
        self
    }

    pub fn column(mut self, column: usize) -> Self {
        self.placement.column = Some(column);
        self
    }

    pub fn at(mut self, row: usize, column: usize) -> Self {
        self.placement.row = Some(row);
        self.placement.column = Some(column);
        self
    }

    pub fn row_span(mut self, span: usize) -> Self {
        self.placement.row_span = span;
        self
    }

    pub fn column_span(mut self, span: usize) -> Self {
        self.placement.column_span = span;
        self
    }

    pub fn horizontal_alignment(mut self, alignment: GridAlignment) -> Self {
        self.horizontal_alignment = Some(alignment);
        self
    }

    pub fn vertical_alignment(mut self, alignment: GridAlignment) -> Self {
        self.vertical_alignment = Some(alignment);
        self
    }
}

/// Lays out positioned items in explicit rows and columns.
///
/// Tracks may use fixed logical pixels, fractional shares, or intrinsic
/// [`GridTrack::Auto`] sizing. Items are placed explicitly or auto-flow into
/// the first available cell; spans and overlaps are validated when the widget
/// is built. Fractional tracks require a bounded constraint on their axis.
/// Invalid layouts render an error widget rather than panicking.
///
/// `Grid::new()` has no tracks or children, zero gaps, stretch alignment, and
/// [`GridOverflow::Clip`]. Configure at least one column and finish the
/// contents with [`Grid::children`].
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_grid::{Grid, GridItem, GridTrack};
///
/// let grid = Grid::new().columns([GridTrack::Px(120.0), GridTrack::Fr(1.0)])
///                       .rows([GridTrack::Auto, GridTrack::Px(40.0)])
///                       .gap(8.0)
///                       .children([GridItem::new(SizedBox::new()).at(0, 0),
///                                  GridItem::new(SizedBox::new()).at(0, 1)]);
/// ```
pub struct Grid<W: Widget + 'static = AnyWidget> {
    columns: Vec<GridTrack>,
    rows: Vec<GridTrack>,
    column_gap: f32,
    row_gap: f32,
    horizontal_alignment: GridAlignment,
    vertical_alignment: GridAlignment,
    overflow: GridOverflow,
    children: Vec<GridItem<W>>,
}

impl Default for Grid<AnyWidget> {
    fn default() -> Self {
        Self::new()
    }
}

impl Grid<AnyWidget> {
    /// Creates an empty grid with no tracks.
    ///
    /// Gaps default to `0.0` logical pixels, both alignments to
    /// [`GridAlignment::Stretch`], and overflow to [`GridOverflow::Clip`].
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            children: Vec::new(),
        }
    }
}

impl<W: Widget + 'static> Grid<W> {
    /// Replaces the column track definitions.
    ///
    /// [`GridTrack::Px`] values are logical pixels, [`GridTrack::Fr`] values
    /// divide bounded remaining width by weight, and [`GridTrack::Auto`] uses
    /// item content. At least one column is required for a valid layout.
    pub fn columns(mut self, tracks: impl IntoIterator<Item = GridTrack>) -> Self {
        self.columns = tracks.into_iter().collect();
        self
    }

    /// Replaces the explicit row track definitions.
    ///
    /// Track units behave as in [`Grid::columns`]. Additional implicit rows may
    /// be created by auto-placement when the supplied rows are exhausted.
    pub fn rows(mut self, tracks: impl IntoIterator<Item = GridTrack>) -> Self {
        self.rows = tracks.into_iter().collect();
        self
    }

    /// Sets both column and row gaps in logical pixels.
    ///
    /// The default is `0.0`; negative values are clamped to `0.0`. Calling this
    /// replaces both axis-specific gap values.
    pub fn gap(mut self, gap: f32) -> Self {
        self.column_gap = gap.max(0.0);
        self.row_gap = gap.max(0.0);
        self
    }

    /// Sets the horizontal gap between columns in logical pixels.
    ///
    /// Negative values are clamped to `0.0`. This replaces only the column gap.
    pub fn column_gap(mut self, gap: f32) -> Self {
        self.column_gap = gap.max(0.0);
        self
    }

    /// Sets the vertical gap between rows in logical pixels.
    ///
    /// Negative values are clamped to `0.0`. This replaces only the row gap.
    pub fn row_gap(mut self, gap: f32) -> Self {
        self.row_gap = gap.max(0.0);
        self
    }

    /// Sets the default horizontal alignment of items within their grid areas.
    ///
    /// The default is [`GridAlignment::Stretch`]. An alignment configured on an
    /// individual [`GridItem`] overrides this value.
    pub fn horizontal_alignment(mut self, alignment: GridAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self
    }

    /// Sets the default vertical alignment of items within their grid areas.
    ///
    /// The default is [`GridAlignment::Stretch`]. An alignment configured on an
    /// individual [`GridItem`] overrides this value.
    pub fn vertical_alignment(mut self, alignment: GridAlignment) -> Self {
        self.vertical_alignment = alignment;
        self
    }

    /// Sets whether painting is clipped to the grid bounds.
    ///
    /// [`GridOverflow::Clip`] is the default. [`GridOverflow::Visible`] permits
    /// item painting outside the grid's constrained area; it does not change
    /// track sizing or placement.
    pub fn overflow(mut self, overflow: GridOverflow) -> Self {
        self.overflow = overflow;
        self
    }

    /// Replaces all grid items with the supplied homogeneous collection.
    ///
    /// This is not an append operation. The returned grid adopts the concrete
    /// child type inside each [`GridItem`] and preserves all track, gap,
    /// alignment, and overflow settings.
    pub fn children<C: Widget + 'static>(
        self,
        children: impl IntoIterator<Item = GridItem<C>>,
    ) -> Grid<C> {
        let children = children.into_iter().collect::<Vec<_>>();
        Grid {
            columns: self.columns,
            rows: self.rows,
            column_gap: self.column_gap,
            row_gap: self.row_gap,
            horizontal_alignment: self.horizontal_alignment,
            vertical_alignment: self.vertical_alignment,
            overflow: self.overflow,
            children,
        }
    }
}

impl<W: Widget + 'static> Widget for Grid<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let placements = self
            .children
            .iter()
            .map(|item| item.placement)
            .collect::<Vec<_>>();
        let validation = resolve_placements(&placements, self.columns.len(), self.rows.len())
            .and_then(|_| {
                resolve_tracks(&self.columns, 1.0, self.column_gap, &[], "columns").map(|_| ())
            })
            .and_then(|_| resolve_tracks(&self.rows, 1.0, self.row_gap, &[], "rows").map(|_| ()));

        if let Err(error) = validation {
            return aimer_widget::ErrorWidget::new(format!("Grid layout error: {error}"))
                .to_element(ctx);
        }

        let children = self
            .children
            .into_iter()
            .map(|item| RawGridItem {
                child: item.child.to_element(ctx),
                placement: item.placement,
                horizontal_alignment: item.horizontal_alignment,
                vertical_alignment: item.vertical_alignment,
            })
            .collect();

        RawGrid {
            columns: self.columns.clone(),
            rows: self.rows.clone(),
            column_gap: self.column_gap,
            row_gap: self.row_gap,
            horizontal_alignment: self.horizontal_alignment,
            vertical_alignment: self.vertical_alignment,
            overflow: self.overflow,
            children,
            layout_cache: RefCell::new(Vec::new()),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Grid"
    }
}


