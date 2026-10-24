use std::cell::RefCell;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Element, Widget};

use super::raw_grid::{
    GridPlacement, GridTrack, RawGrid, RawGridItem, resolve_placements, resolve_tracks,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_grid::grid::GridAlignment",
    max_encoded_bytes = 16
)]
pub enum GridAlignment {
    #[portable_value(tag = 0)]
    Start,
    #[portable_value(tag = 1)]
    Center,
    #[portable_value(tag = 2)]
    End,
    #[default]
    #[portable_value(tag = 3)]
    Stretch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_grid::grid::GridOverflow",
    max_encoded_bytes = 16
)]
pub enum GridOverflow {
    #[default]
    #[portable_value(tag = 0)]
    Clip,
    #[portable_value(tag = 1)]
    Visible,
}

/// The portable placement and item-local alignment settings for one Grid item.
#[derive(Clone, Debug, PartialEq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_grid::grid::GridItemConfig",
    max_encoded_bytes = 256,
    max_depth = 8,
    max_entries = 32
)]
pub struct GridItemConfig {
    /// The explicit or auto-flow placement of the item.
    pub placement: GridPlacement,
    /// An optional alignment overriding the Grid default on the horizontal axis.
    pub horizontal_alignment: Option<GridAlignment>,
    /// An optional alignment overriding the Grid default on the vertical axis.
    pub vertical_alignment: Option<GridAlignment>,
}

/// Bounded, versioned layout state carried by a portable Grid node.
#[derive(Clone, Debug, PartialEq, aimer_macro::PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_grid::grid::GridPortableConfig",
    max_encoded_bytes = 32_768,
    max_depth = 16,
    max_entries = 512
)]
pub struct GridPortableConfig {
    /// The column track definitions.
    pub columns: Vec<GridTrack>,
    /// The explicit row track definitions.
    pub rows: Vec<GridTrack>,
    /// The gap between columns in logical pixels.
    pub column_gap: f32,
    /// The gap between rows in logical pixels.
    pub row_gap: f32,
    /// The default horizontal item alignment.
    pub horizontal_alignment: GridAlignment,
    /// The default vertical item alignment.
    pub vertical_alignment: GridAlignment,
    /// The painting overflow policy.
    pub overflow: GridOverflow,
    /// Layout metadata corresponding one-to-one with the structural children.
    pub items: Vec<GridItemConfig>,
}

impl Default for GridPortableConfig {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            column_gap: 0.0,
            row_gap: 0.0,
            horizontal_alignment: GridAlignment::Stretch,
            vertical_alignment: GridAlignment::Stretch,
            overflow: GridOverflow::Clip,
            items: Vec::new(),
        }
    }
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
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_grid::grid::Grid", schema_only, manual_lowering)]
pub struct Grid<W: Widget + 'static = AnyWidget> {
    #[portable_skip]
    columns: Vec<GridTrack>,
    #[portable_skip]
    rows: Vec<GridTrack>,
    #[portable_skip]
    column_gap: f32,
    #[portable_skip]
    row_gap: f32,
    #[portable_skip]
    horizontal_alignment: GridAlignment,
    #[portable_skip]
    vertical_alignment: GridAlignment,
    #[portable_skip]
    overflow: GridOverflow,
    config: GridPortableConfig,
    #[portable_children]
    children: Vec<GridItem<W>>,
}

#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
#[aimer_widget::portable::__linkme::distributed_slice(
    aimer_widget::portable::materializer::PORTABLE_NATIVE_WIDGET_SCHEMAS
)]
#[linkme(crate = aimer_widget::portable::__linkme)]
#[allow(non_upper_case_globals)]
static __AIMER_PORTABLE_NATIVE_SCHEMA_FOR_GRID:
    aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata<'static> =
    <Grid as aimer_widget::portable::PortableWidgetSchema>::SCHEMA;

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
            config: GridPortableConfig::default(),
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
        self.config.columns = self.columns.clone();
        self
    }

    /// Replaces the explicit row track definitions.
    ///
    /// Track units behave as in [`Grid::columns`]. Additional implicit rows may
    /// be created by auto-placement when the supplied rows are exhausted.
    pub fn rows(mut self, tracks: impl IntoIterator<Item = GridTrack>) -> Self {
        self.rows = tracks.into_iter().collect();
        self.config.rows = self.rows.clone();
        self
    }

    /// Sets both column and row gaps in logical pixels.
    ///
    /// The default is `0.0`; negative values are clamped to `0.0`. Calling this
    /// replaces both axis-specific gap values.
    pub fn gap(mut self, gap: f32) -> Self {
        self.column_gap = gap.max(0.0);
        self.row_gap = gap.max(0.0);
        self.config.column_gap = self.column_gap;
        self.config.row_gap = self.row_gap;
        self
    }

    /// Sets the horizontal gap between columns in logical pixels.
    ///
    /// Negative values are clamped to `0.0`. This replaces only the column gap.
    pub fn column_gap(mut self, gap: f32) -> Self {
        self.column_gap = gap.max(0.0);
        self.config.column_gap = self.column_gap;
        self
    }

    /// Sets the vertical gap between rows in logical pixels.
    ///
    /// Negative values are clamped to `0.0`. This replaces only the row gap.
    pub fn row_gap(mut self, gap: f32) -> Self {
        self.row_gap = gap.max(0.0);
        self.config.row_gap = self.row_gap;
        self
    }

    /// Sets the default horizontal alignment of items within their grid areas.
    ///
    /// The default is [`GridAlignment::Stretch`]. An alignment configured on an
    /// individual [`GridItem`] overrides this value.
    pub fn horizontal_alignment(mut self, alignment: GridAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self.config.horizontal_alignment = alignment;
        self
    }

    /// Sets the default vertical alignment of items within their grid areas.
    ///
    /// The default is [`GridAlignment::Stretch`]. An alignment configured on an
    /// individual [`GridItem`] overrides this value.
    pub fn vertical_alignment(mut self, alignment: GridAlignment) -> Self {
        self.vertical_alignment = alignment;
        self.config.vertical_alignment = alignment;
        self
    }

    /// Sets whether painting is clipped to the grid bounds.
    ///
    /// [`GridOverflow::Clip`] is the default. [`GridOverflow::Visible`] permits
    /// item painting outside the grid's constrained area; it does not change
    /// track sizing or placement.
    pub fn overflow(mut self, overflow: GridOverflow) -> Self {
        self.overflow = overflow;
        self.config.overflow = overflow;
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
        let mut config = self.config;
        config.items = children
            .iter()
            .map(|item| GridItemConfig {
                placement: item.placement,
                horizontal_alignment: item.horizontal_alignment,
                vertical_alignment: item.vertical_alignment,
            })
            .collect();
        Grid {
            columns: self.columns,
            rows: self.rows,
            column_gap: self.column_gap,
            row_gap: self.row_gap,
            horizontal_alignment: self.horizontal_alignment,
            vertical_alignment: self.vertical_alignment,
            overflow: self.overflow,
            config,
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

impl<W: Widget + 'static> aimer_widget::PortableWidget for Grid<W> {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut aimer_widget::portable::PortableBuildContext,
        source: aimer_widget::portable::SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, aimer_widget::portable::PortableBuildError>
    {
        let schema = <Self as aimer_widget::portable::PortableWidgetSchema>::SCHEMA;
        let property_id = aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_grid::grid::Grid:config",
        );
        let config = ctx.encode_property_named(
            property_id,
            "aimer.property:aimer_grid::grid::Grid:config",
            source.child(aimer_widget::portable::__anteros::stable_schema_hash64(
                "aimer.source:aimer_grid::grid::Grid:config",
            )),
            self.config,
        )?;
        let properties = [aimer_widget::portable::__anteros::WidgetProperty::new(
            property_id,
            config,
        )];
        let children = self
            .children
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                aimer_widget::PortableWidget::to_portable_node(
                    item.child,
                    ctx,
                    source
                        .child(aimer_widget::portable::__anteros::stable_schema_hash64(
                            "aimer.source:aimer_grid::grid::Grid:children",
                        ))
                        .child(index as u64),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        ctx.push_node(
            schema.widget().id(),
            schema.widget().min_version(),
            None,
            source,
            &properties,
            &children,
        )
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_layout_tests {
    use aimer_widget::base::BuildContext;
    use aimer_widget::portable::__anteros::{Version, WIDGET_SIZED_BOX, WidgetDocumentView};
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableWidgetLimits, PortableWidgetSchema,
        SourceFingerprint, StableId128,
    };
    use aimer_widget::{AnyElement, ErrorWidget, PortableWidget, Widget};

    use super::{Grid, GridItem, GridTrack};

    struct Leaf;

    impl Widget for Leaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            ErrorWidget::new("portable leaf").to_element(ctx)
        }
    }

    impl PortableWidget for Leaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            ctx.push_node(WIDGET_SIZED_BOX, Version::new(1, 0), None, source, &[], &[])
        }
    }

    #[test]
    fn grid_lowers_nested_grid_item_children() {
        let mut ctx = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(16, 16, 16, 16, 1_024, 8_192).with_max_blob_bytes(128),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let root = Grid::new()
            .columns([GridTrack::Px(100.0)])
            .rows([GridTrack::Px(40.0)])
            .children([GridItem::new(Leaf).at(0, 0)])
            .to_portable_node(
                &mut ctx,
                SourceFingerprint::new(StableId128::from_bytes([0x22; 16])),
            )
            .unwrap();
        let schema = <Grid<Leaf> as PortableWidgetSchema>::SCHEMA;
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(node.widget_type(), schema.widget().id());
        assert_eq!(node.properties().count(), 1);
        assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
    }
}
