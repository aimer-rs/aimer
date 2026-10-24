use std::ops::Range;
use std::rc::Rc;

use crate::flex::children_source::{ChildrenSource, EagerChildren};
use crate::flex::flex_layout::{FlexLayout, FlexLayoutCache, LayerOrder};
use crate::flex::flex_list::FlexList;
use crate::flex::layout_transition::{FlexGeometrySnapshot, FlexItemGeometry, FlexSnapshotError};
use crate::flex::{BoxAlignment, FlexDirection, JustifyContent, OverflowBehavior};
use aimer_animation::layout::LayoutGeometry;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_attribute::{BoxConstraint, Dimension};
use aimer_style::LayoutSpacing;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutCache,
    LayoutElement, Rebuildable, VisitorElement, Widget,
};

/// Arranges a homogeneous collection of children along a configurable main
/// axis.
///
/// [`FlexDirection::Row`] uses a horizontal main axis and
/// [`FlexDirection::Column`] a vertical one. Alignment controls placement on
/// each physical axis, while [`OverflowBehavior`] clips, exposes, or wraps
/// children that exceed the available constraints. Spacing is expressed as
/// logical pixels through [`LayoutSpacing`].
///
/// `Flex::new()` defaults to [`FlexDirection::Inherit`], start alignment on
/// both axes, zero gaps, [`OverflowBehavior::Hidden`], and no children. Supply
/// children with [`Flex::children`] or append to an existing erased collection
/// with [`Flex::add_child`].
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_flex::flex::Flex", schema_only)]
#[allow(dead_code)]
pub struct Flex<W: Widget + 'static = AnyWidget> {
    #[portable_skip]
    pub(crate) direction: FlexDirection,
    #[portable_skip]
    pub(crate) vertical_alignment: BoxAlignment,
    #[portable_skip]
    pub(crate) horizontal_alignment: BoxAlignment,
    #[portable_skip]
    pub(crate) justify_content: Option<JustifyContent>,
    #[portable_skip]
    pub(crate) gaps: LayoutSpacing,
    #[portable_skip]
    pub(crate) overflow: OverflowBehavior,
    #[portable_children]
    pub(crate) children: Vec<W>,
}

impl Default for Flex {
    /// Creates an empty flex layout with inherited direction.
    ///
    /// Both alignments default to [`BoxAlignment::Start`], gaps to zero, and
    /// overflow to [`OverflowBehavior::Hidden`]. An empty flex is already a
    /// valid [`Widget`].
    fn default() -> Self {
        Self::new()
    }
}

impl Flex {
    /// Creates an empty flex layout with inherited direction.
    ///
    /// Both alignments default to [`BoxAlignment::Start`], gaps to zero, and
    /// overflow to [`OverflowBehavior::Hidden`]. An empty flex is already a
    /// valid [`Widget`].
    #[inline]
    pub fn new() -> Self {
        Self {
            direction: FlexDirection::default(),
            vertical_alignment: BoxAlignment::default(),
            horizontal_alignment: BoxAlignment::default(),
            justify_content: None,
            gaps: LayoutSpacing::default(),
            overflow: OverflowBehavior::default(),
            children: Vec::new(),
        }
    }
}

impl<W: Widget + 'static> Flex<W> {
    /// Sets the main-axis direction used to place children.
    ///
    /// The default is [`FlexDirection::Inherit`]. Use
    /// [`FlexDirection::Row`] for left-to-right placement or
    /// [`FlexDirection::Column`] for top-to-bottom placement.
    #[inline]
    pub fn direction(mut self, direction: FlexDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Sets alignment on the physical vertical axis.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn vertical_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.vertical_alignment = alignment;
        self
    }

    /// Sets alignment on the physical horizontal axis.
    ///
    /// The default is [`BoxAlignment::Start`].
    #[inline]
    pub fn horizontal_alignment(mut self, alignment: BoxAlignment) -> Self {
        self.horizontal_alignment = alignment;
        self
    }

    /// Sets placement along the flex container's main axis.
    ///
    /// The default preserves the physical-axis alignment methods. Once set,
    /// this semantic value takes precedence over the old main-axis alignment:
    /// [`FlexDirection::Row`] uses the horizontal axis and
    /// [`FlexDirection::Column`] uses the vertical axis.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use aimer_flex::{Flex, FlexDirection, JustifyContent};
    ///
    /// let flex = Flex::new()
    ///     .direction(FlexDirection::Row)
    ///     .justify_content(JustifyContent::SpaceEvenly);
    /// ```
    #[inline]
    pub fn justify_content(mut self, justify_content: JustifyContent) -> Self {
        self.justify_content = Some(justify_content);
        self
    }

    /// Sets the spacing between adjacent children.
    ///
    /// Values are logical pixels represented by [`LayoutSpacing`]; the default
    /// is zero spacing. Horizontal sides contribute to row gaps and vertical
    /// sides contribute to column gaps.
    #[inline]
    pub fn gaps(mut self, gaps: impl Into<LayoutSpacing>) -> Self {
        self.gaps = gaps.into();
        self
    }

    /// Sets how children exceeding the available constraints are handled.
    ///
    /// [`OverflowBehavior::Hidden`] is the default and clips to the flex
    /// bounds. [`OverflowBehavior::Visible`] paints outside them, while
    /// [`OverflowBehavior::Wrap`] creates additional rows or columns.
    #[inline]
    pub fn overflow(mut self, overflow: OverflowBehavior) -> Self {
        self.overflow = overflow;
        self
    }

    /// Supplies a data source instead of a widget collection.
    ///
    /// The returned [`FlexList`] is not yet a widget: pair it with
    /// [`FlexList::builder`] to map each datum to a child. Prefer this over
    /// [`Flex::children`] for long lists — the container then retains the data
    /// rather than one widget per item. Children supplied earlier are replaced,
    /// exactly as [`Flex::children`] replaces them.
    ///
    /// # Example
    ///
    /// ```rust
    /// use aimer_container::SizedBox;
    /// use aimer_flex::{Flex, FlexDirection};
    ///
    /// let flex = Flex::new().direction(FlexDirection::Column)
    ///                       .list(0..120_000)
    ///                       .builder(|i| SizedBox::new().height(*i % 40));
    /// ```
    #[inline]
    pub fn list<T>(self, items: impl IntoIterator<Item = T>) -> FlexList<T> {
        FlexList::new(
            self.direction,
            self.vertical_alignment,
            self.horizontal_alignment,
            self.justify_content,
            self.gaps,
            self.overflow,
            items,
        )
    }

    /// Replaces all children with the supplied homogeneous collection.
    ///
    /// This is not an append operation. The returned [`Flex`] adopts the item
    /// type of the iterator and is immediately a valid [`Widget`], including
    /// when the iterator is empty.
    #[inline]
    pub fn children<C: Widget>(self, children: impl IntoIterator<Item = C>) -> Flex<C> {
        Flex {
            direction: self.direction,
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            overflow: self.overflow,
            children: children.into_iter().collect(),
        }
    }

    /// Appends one child to the existing collection.
    ///
    /// The child must have the same type as the collection's existing items.
    /// Use [`Flex::children`] to replace the collection or establish a new item
    /// type.
    #[inline]
    pub fn add_child(mut self, child: W) -> Self {
        self.children.push(child);
        self
    }
}

impl<W: Widget + 'static> Widget for Flex<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let elements = self
            .children
            .into_iter()
            .map(|c| c.to_element(ctx))
            .collect();
        RawFlex {
            direction: self.direction,
            vertical_alignment: self.vertical_alignment,
            horizontal_alignment: self.horizontal_alignment,
            justify_content: self.justify_content,
            gaps: self.gaps,
            children: Box::new(EagerChildren(elements)),
            cache: LayoutCache::new(),
            layout: FlexLayoutCache::new(),
            item_extent: None,
            overflow_behavior: self.overflow,
            debug_name: "Flex",
        }
        .boxed()
    }
}

/// #### lower level flex container also the base of the flex layout such as
///
/// - Flex: layout that aligns children in horizontal and vertical
///
/// - Column: layout that always aligns children in a vertical direction
///
/// - Row: layout that always aligns children in a horizontal direction
#[allow(dead_code)]
pub struct RawFlex {
    pub(crate) direction: FlexDirection,
    pub(crate) vertical_alignment: BoxAlignment,
    pub(crate) horizontal_alignment: BoxAlignment,
    pub(crate) justify_content: Option<JustifyContent>,
    pub(crate) gaps: LayoutSpacing,
    /// Supplies the children by index.
    ///
    /// A widget collection produces
    /// [`EagerChildren`](crate::flex::children_source::EagerChildren), which
    /// simply owns the vector. A data source with a declared item extent
    /// produces a windowed one that materializes a viewport at a time — see the
    /// [sparse-children contract](crate::flex::children_source).
    pub(crate) children: Box<dyn ChildrenSource>,
    pub(crate) cache: LayoutCache,
    pub(crate) layout: FlexLayoutCache,
    /// Main-axis extent every child is declared to occupy, when known.
    ///
    /// Set through [`FlexList::item_extent`], it replaces the measuring pass
    /// with arithmetic.
    pub(crate) item_extent: Option<Dimension>,
    pub(crate) overflow_behavior: OverflowBehavior,
    pub(crate) debug_name: &'static str,
}

impl RawFlex {
    /// Creates a low-level flex element with default alignment, spacing, and
    /// clipping.
    #[doc(hidden)]
    #[inline]
    pub fn new(
        direction: FlexDirection,
        children: Vec<AnyElement>,
        debug_name: &'static str,
    ) -> Self {
        Self {
            direction,
            vertical_alignment: Default::default(),
            horizontal_alignment: Default::default(),
            justify_content: None,
            gaps: Default::default(),
            children: Box::new(EagerChildren(children)),
            cache: Default::default(),
            layout: Default::default(),
            item_extent: None,
            overflow_behavior: Default::default(),
            debug_name,
        }
    }

    pub(crate) fn render_child(widget: &dyn Element, ctx: &BuildContext) {
        ctx.canvas.save();
        widget.update(ctx);
        ctx.canvas.restore();
    }

    /// Captures the geometry this Flex engine resolved for its materialized
    /// children.
    ///
    /// The adapter deliberately follows the normal painting window. A lazy
    /// [`FlexList`] therefore contributes only its visible/overscanned rows,
    /// while a wrapped layout follows the existing eager wrapping contract.
    /// Every captured child must publish a stable reconciliation key; an
    /// unkeyed child is reported instead of being guessed by index.
    pub fn layout_transition_snapshot(
        &self,
        ctx: &BuildContext,
    ) -> Result<FlexGeometrySnapshot, FlexSnapshotError> {
        let (gap_x, gap_y) = self.resole_gaps(ctx);
        let is_row = self.is_row();

        if self.overflow_behavior == OverflowBehavior::Wrap {
            let (sizes, layout) = self.wrapped_layout(ctx, gap_x, gap_y);
            let extra_width = (ctx.box_constraint.max_width - layout.size.width).max(0.0);
            let extra_height = (ctx.box_constraint.max_height - layout.size.height).max(0.0);
            let base_x = if matches!(self.direction, FlexDirection::Column) {
                align_offset(self.horizontal_alignment, extra_width)
            } else {
                0.0
            };
            let base_y = if matches!(self.direction, FlexDirection::Column) {
                0.0
            } else {
                align_offset(self.vertical_alignment, extra_height)
            };

            let children = sizes
                .into_iter()
                .enumerate()
                .map(|(index, size)| {
                    let offset = layout.offset(index);
                    let geometry = LayoutGeometry::try_new(
                        offset.0 + base_x,
                        offset.1 + base_y,
                        size.width,
                        size.height,
                    )
                    .map_err(|error| FlexSnapshotError::Keyed(error.into()))?;
                    let child = self
                        .children
                        .get(index)
                        .ok_or(FlexSnapshotError::MissingKey)?;
                    Ok(child
                        .reconciliation_key()
                        .cloned()
                        .map(|key| FlexItemGeometry::new(key, geometry)))
                })
                .collect::<Result<Vec<_>, FlexSnapshotError>>()?;
            return FlexGeometrySnapshot::try_new_optional(
                LayoutGeometry::try_new(0.0, 0.0, layout.size.width, layout.size.height)
                    .map_err(|error| FlexSnapshotError::Keyed(error.into()))?,
                children,
            );
        }

        let (layout, distribution) = self
            .prepare_paint_partition(ctx)
            .expect("a non-wrapping Flex always has a paint partition");
        let range = self
            .layout
            .painted()
            .unwrap_or_else(|| self.painted_range(ctx, &layout, distribution));
        let max_w = ctx.box_constraint.max_width;
        let max_h = ctx.box_constraint.max_height;
        let children = range
            .map(
                |index| -> Result<Option<FlexItemGeometry>, FlexSnapshotError> {
                    let child = self
                        .children
                        .get(index)
                        .ok_or(FlexSnapshotError::MissingKey)?;
                    let size = layout.size(index);
                    let main =
                        distribution.0 + layout.offset_f32(index) + distribution.1 * index as f32;
                    let (x, y) = if is_row {
                        (
                            main,
                            align_offset(self.vertical_alignment, (max_h - size.height).max(0.0)),
                        )
                    } else {
                        (
                            align_offset(self.horizontal_alignment, (max_w - size.width).max(0.0)),
                            main,
                        )
                    };
                    let geometry = LayoutGeometry::try_new(x, y, size.width, size.height)
                        .map_err(|error| FlexSnapshotError::Keyed(error.into()))?;
                    Ok(child
                        .reconciliation_key()
                        .cloned()
                        .map(|key| FlexItemGeometry::new(key, geometry)))
                },
            )
            .collect::<Result<Vec<_>, FlexSnapshotError>>()?;

        FlexGeometrySnapshot::try_new_optional(
            LayoutGeometry::try_new(0.0, 0.0, layout.total().width, layout.total().height)
                .map_err(|error| FlexSnapshotError::Keyed(error.into()))?,
            children,
        )
    }
}

impl RawFlex {
    /// Prepares the same layout table used by ordinary painting, without
    /// tying the result to one visible rectangle. Dynamic-island painting is
    /// only enabled for eager children, so the complete order can be reused
    /// for the static prefix while the live context still culls dynamic rows.
    fn prepare_paint_partition(&self, ctx: &BuildContext) -> Option<(Rc<FlexLayout>, (f32, f32))> {
        let mut layout = self.flex_layout(ctx);
        let mut distribution = self.main_distribution(ctx, layout.total(), layout.len());
        let mut range = self.painted_range(ctx, &layout, distribution);

        self.children.window(range.clone(), ctx);

        for _ in 0..RECONCILE_PASSES {
            let outcome = self.reconcile(ctx, &layout, &range);
            if matches!(outcome, Reconciled::Matched) {
                break;
            }
            let rebuilt = matches!(outcome, Reconciled::Stale);
            if rebuilt {
                layout = self.measure_layout(ctx);
            } else {
                ctx.window.request_redraw();
            }
            distribution = self.main_distribution(ctx, layout.total(), layout.len());
            range = self.painted_range(ctx, &layout, distribution);
            self.children.window(range.clone(), ctx);
            if rebuilt {
                break;
            }
        }

        self.cache
            .set_computed(ctx.box_constraint, scale_bits_of(ctx), layout.total());
        self.layout.set_painted(&range);
        Some((layout, distribution))
    }
}

/// How often one frame reconciles its table with the children it is about to
/// paint.
///
/// Correcting a prediction moves the children after the correction, which can
/// bring another one into the viewport, and that one has to be checked too. Two
/// rounds settle any single correction; the third only exists so a cascade
/// terminates rather than being chased forever.
const RECONCILE_PASSES: usize = 3;

/// Outcome of comparing the children about to be painted with the cached table.
enum Reconciled {
    /// The table describes them, so the frame can paint straight away.
    Matched,
    /// A prediction absorbed their exact extents. Offsets after the first
    /// correction moved, so the painted range has to be resolved again.
    Refined,
    /// The table cannot be corrected in place and has to be rebuilt.
    Stale,
}

/// Whether `value` is a real extent rather than an "unbounded" sentinel.
#[inline]
fn is_bounded(value: f32) -> bool {
    value.is_finite() && value < f32::MAX
}

/// Cache key component derived from the context's device scale.
#[inline]
fn scale_bits_of(ctx: &BuildContext) -> u32 {
    ctx.scale.to_bits()
}

/// Distance a child is pushed along one axis to satisfy `alignment`.
///
/// `extra` is the unused space on that axis and is never negative.
#[inline]
fn align_offset(alignment: BoxAlignment, extra: f32) -> f32 {
    match alignment {
        BoxAlignment::Start => 0.0,
        BoxAlignment::Center => extra / 2.0,
        BoxAlignment::End => extra,
    }
}

/// Returns the leading space and additional space between adjacent children.
///
/// `free_space` excludes the gaps already recorded in the layout table. A
/// non-positive or unbounded amount cannot be distributed, so children remain
/// at their measured positions.
#[inline]
pub(crate) fn justify_distribution(
    justify_content: JustifyContent,
    free_space: f32,
    child_count: usize,
) -> (f32, f32) {
    if child_count == 0 || !free_space.is_finite() || free_space <= 0.0 {
        return (0.0, 0.0);
    }

    match justify_content {
        JustifyContent::Start => (0.0, 0.0),
        JustifyContent::Center => (free_space / 2.0, 0.0),
        JustifyContent::End => (free_space, 0.0),
        JustifyContent::SpaceBetween if child_count > 1 => {
            (0.0, free_space / (child_count - 1) as f32)
        }
        JustifyContent::SpaceAround => {
            let between = free_space / child_count as f32;
            (between / 2.0, between)
        }
        JustifyContent::SpaceEvenly => {
            let space = free_space / (child_count + 1) as f32;
            (space, space)
        }
        JustifyContent::SpaceBetween => (0.0, 0.0),
    }
}

impl RawFlex {
    #[inline]
    pub(crate) fn resole_gaps(&self, ctx: &BuildContext) -> (f32, f32) {
        let max_width = ctx.box_constraint.max_width;
        let max_height = ctx.box_constraint.max_height;
        let gap_x = self.gaps.left.value(max_width, ctx.scale)
            + self.gaps.right.value(max_width, ctx.scale);
        let gap_y = self.gaps.top.value(max_height, ctx.scale)
            + self.gaps.bottom.value(max_height, ctx.scale);

        (gap_x, gap_y)
    }

    /// `true` when children run along the horizontal axis.
    #[inline]
    fn is_row(&self) -> bool {
        !matches!(self.direction, FlexDirection::Column)
    }

    /// Picks the gap that separates two adjacent children.
    #[inline]
    fn gap_main(&self, gap_x: f32, gap_y: f32) -> f32 {
        if self.is_row() { gap_x } else { gap_y }
    }

    /// Returns the main-axis table for `ctx`, measuring the children only when
    /// the constraint or the scale changed since the last pass.
    ///
    /// Measuring and painting share the result, so a scroll — which changes the
    /// visible rectangle and nothing else — never re-measures the child list.
    fn flex_layout(&self, ctx: &BuildContext) -> Rc<FlexLayout> {
        if let Some(layout) = self.layout.get(ctx.box_constraint, scale_bits_of(ctx)) {
            return layout;
        }
        if let Some(layout) = self
            .layout
            .get_stale_stable(ctx.box_constraint, scale_bits_of(ctx))
            && layout.stable_children_match(self.children.as_ref())
        {
            self.layout
                .set(ctx.box_constraint, scale_bits_of(ctx), Rc::clone(&layout));
            return layout;
        }
        // Most of a list is usually untouched by the rebuild that advanced the
        // generation, so ask only the children that cannot vouch for themselves.
        if let Some(layout) = self
            .layout
            .get_stale_measured(ctx.box_constraint, scale_bits_of(ctx))
            && layout.revalidate_measured(self.children.as_ref(), ctx)
        {
            self.layout
                .set(ctx.box_constraint, scale_bits_of(ctx), Rc::clone(&layout));
            return layout;
        }
        self.measure_layout(ctx)
    }

    /// Rebuilds the cached table, measuring the children only when their extent
    /// can be neither declared nor predicted.
    ///
    /// The three sources are tried in order of what they cost: a declared extent
    /// is free, a prediction costs one child, and measuring costs all of them.
    ///
    /// Measuring is the one pass that needs every child at once, so a windowed
    /// source is asked to materialize its whole range first. That is the escape
    /// hatch for a declared extent that cannot be resolved under the current
    /// constraints — the container reports an honest size instead of a total
    /// derived from the handful of rows it happens to hold.
    fn measure_layout(&self, ctx: &BuildContext) -> Rc<FlexLayout> {
        let (gap_x, gap_y) = self.resole_gaps(ctx);
        let gap_main = self.gap_main(gap_x, gap_y);
        let known = self
            .declared_layout(ctx, gap_main)
            .or_else(|| self.estimated_layout(ctx, gap_main));
        let layout = Rc::new(match known {
            Some(table) => table,
            None => {
                self.materialize_all(ctx);
                FlexLayout::build(self.direction, self.children.as_ref(), ctx, gap_main)
            }
        });
        self.layout
            .set(ctx.box_constraint, scale_bits_of(ctx), Rc::clone(&layout));
        layout
    }

    /// Builds the main-axis table from [`RawFlex::item_extent`] alone.
    ///
    /// A declared extent turns the whole pass into arithmetic: the total is
    /// `(extent + gap) * len - gap`, so a list of a hundred thousand rows
    /// reports its scroll extent without a single child being measured, and the
    /// first frame only touches the handful of children it paints. Children are
    /// laid out across the container's full cross-axis maximum, since nothing
    /// measured them.
    ///
    /// Returns `None` — falling back to measuring — when the extent states
    /// nothing ([`Dimension::Auto`]) or cannot be resolved, which happens for a
    /// percentage extent under an unbounded main axis and when the cross axis
    /// itself is unbounded.
    fn declared_layout(&self, ctx: &BuildContext, gap_main: f32) -> Option<FlexLayout> {
        let is_row = self.is_row();
        let extent = self.item_extent?;

        let (max_main, max_cross) = if is_row {
            (ctx.box_constraint.max_width, ctx.box_constraint.max_height)
        } else {
            (ctx.box_constraint.max_height, ctx.box_constraint.max_width)
        };

        let main = match extent {
            // `Auto` declares nothing, so there is no table to build.
            Dimension::Auto => return None,
            Dimension::Px(value) => value * ctx.scale,
            // A share of an unbounded axis is not a size.
            Dimension::Percent(_) if !is_bounded(max_main) => return None,
            percent => percent.resolve(max_main, ctx.scale),
        };
        if !is_bounded(main) || main < 0.0 || !is_bounded(max_cross) {
            return None;
        }

        Some(FlexLayout::declared(
            self.children.len(),
            main,
            max_cross.max(0.0),
            is_row,
            gap_main,
        ))
    }

    /// Predicts the main-axis table from a single probed child.
    ///
    /// Without a declared extent the container would have to measure every child
    /// before it could report its own size, which is what makes a long list hang
    /// on its first frame. Measuring *one* child and assuming the rest match it
    /// costs one measure instead of `len`, and it is exact for the shape a long
    /// scrolled list almost always has: uniform rows. The guess is not taken on
    /// faith — [`RawFlex::reconcile`] re-measures the children each frame paints,
    /// and a child that disagrees has its exact extent recorded in the table, so
    /// a list of varying rows converges on its true extent as it is scrolled
    /// without ever having to be measured whole.
    ///
    /// Returns `None`, leaving the exact measuring pass in charge, when:
    ///
    /// - the children are not built on demand, so measuring them is what the
    ///   container already paid for, and a wrong guess would only add a visible
    ///   correction to an ordinary [`Row`](crate::Row) or
    ///   [`Column`](crate::Column);
    /// - the main axis is bounded, which means the container is not inside a
    ///   scroll viewport and its total is not what a scroll extent is derived
    ///   from — there is nothing to win and a correction to lose;
    /// - the probe is a flex child, whose size comes from distributing the
    ///   leftover space rather than from the child itself.
    ///
    /// The probe is taken from the live window rather than from index zero: at a
    /// deep scroll offset row zero does not exist, so probing it would build a
    /// row only to drop it, and a first row is often the atypical one.
    fn estimated_layout(&self, ctx: &BuildContext, gap_main: f32) -> Option<FlexLayout> {
        let len = self.children.len();
        if len == 0 || !self.children.is_windowed() {
            return None;
        }

        let is_row = self.is_row();
        let (max_main, max_cross) = if is_row {
            (ctx.box_constraint.max_width, ctx.box_constraint.max_height)
        } else {
            (ctx.box_constraint.max_height, ctx.box_constraint.max_width)
        };
        if is_bounded(max_main) {
            return None;
        }

        // Prefer the representative row retained by a previous window. If a
        // list is starting cold at a deep offset, row zero is not a useful
        // representative either: it may carry one-off leading spacing. The
        // first viewport row is therefore the fallback in that case.
        let cold_probe = || {
            let main_start = ctx
                .visible_rect
                .map(|(x, y, _, _)| if is_row { x } else { y });
            (main_start.is_some_and(|start| start > 0.0) && len > 1).then_some(1)
        };
        let index = self
            .children
            .probe_index()
            .or_else(cold_probe)
            .unwrap_or(0)
            .min(len - 1);
        if self.children.get(index).is_none() {
            self.children.window(index..index + 1, ctx);
        }
        let probe = self.children.get(index)?;
        if probe.flex().is_some() {
            // Distribution decides the size, so there is nothing to extrapolate
            // from — and every share depends on the other children anyway.
            self.children.materialize_all(ctx);
            return None;
        }

        // Measured exactly as `FlexLayout::build` would, so a later
        // revalidation compares like with like.
        let mut child_ctx = ctx.clone();
        if is_row {
            child_ctx.box_constraint.max_width = f32::MAX;
            child_ctx.box_constraint.max_height = max_cross;
        } else {
            child_ctx.box_constraint.max_height = f32::MAX;
            child_ctx.box_constraint.max_width = max_cross;
        }
        let probed = probe.computed_size(&child_ctx);

        let layout = FlexLayout::estimated(len, probed, is_row, gap_main);
        // Asked under another constraint than the one the held table was made
        // for, which a layout pass and the render-tree synchronization do to one
        // container in the same frame, the held table cannot answer. What it
        // learned still stands, so the replacement starts from it.
        if let Some(previous) = self.layout.learned() {
            layout.inherit_learned(&previous);
        }
        Some(layout)
    }

    /// Compares the children about to be painted with the cached table, and
    /// corrects the table where it can.
    ///
    /// Nothing invalidates a flex container when a descendant resizes itself —
    /// an implicitly animated child rebuilds inside its own `draw` — so the
    /// children that are about to be painted are re-measured and compared. That
    /// keeps the check proportional to what is on screen: a child that changed
    /// size while off-screen cannot be seen to be mispositioned, and it is
    /// picked up as soon as it scrolls into range.
    ///
    /// A *predicted* table absorbs a disagreement instead of being thrown away:
    /// [`FlexLayout::refine`] records the child's exact extent, the children
    /// before it keep the offsets they were painted at, and the container stays
    /// windowed. A long list of genuinely varying rows therefore converges on its
    /// true extent as it is scrolled, rather than paying for measuring all of it
    /// the moment one row disagrees.
    ///
    /// A table containing flex children is never trusted across frames, because
    /// each share depends on what every other child consumed.
    fn reconcile(
        &self,
        ctx: &BuildContext,
        layout: &FlexLayout,
        range: &Range<usize>,
    ) -> Reconciled {
        // A declared extent is authoritative, so nothing is re-measured.
        if layout.is_declared() {
            return Reconciled::Matched;
        }

        if layout.has_flex() {
            return Reconciled::Stale;
        }

        let is_row = self.is_row();
        let mut child_ctx = ctx.clone();
        if is_row {
            child_ctx.box_constraint.max_width = f32::MAX;
            child_ctx.box_constraint.max_height = ctx.box_constraint.max_height;
        } else {
            child_ctx.box_constraint.max_height = f32::MAX;
            child_ctx.box_constraint.max_width = ctx.box_constraint.max_width;
        }

        let predicted = layout.is_estimated();
        let mut refined = false;
        for index in range.clone() {
            let Some(child) = self.children.get(index) else {
                continue;
            };
            // A flex child's size comes from distributing the leftover space, so
            // no per-child correction can describe it and the whole list has to
            // be measured together.
            if child.flex().is_some() {
                self.materialize_all(ctx);
                return Reconciled::Stale;
            }
            let size = child.computed_size(&child_ctx);
            if size == layout.size(index) {
                continue;
            }
            if !predicted {
                return Reconciled::Stale;
            }
            layout.refine(index, size);
            refined = true;
        }

        if refined {
            Reconciled::Refined
        } else {
            Reconciled::Matched
        }
    }

    /// Forces the source to hold every child, permanently.
    ///
    /// Only the measuring and wrapping passes need this: both derive a result
    /// from the whole list, so a partial window would produce a wrong size. The
    /// source stops windowing rather than merely widening its range, so the next
    /// frame cannot drop what the measured table describes — and a container
    /// that had to measure once never goes back to predicting.
    pub(crate) fn materialize_all(&self, ctx: &BuildContext) {
        self.children.materialize_all(ctx);
    }

    /// Main-axis leading and inter-child space added by the container's
    /// justification.
    #[inline]
    fn main_distribution(
        &self,
        ctx: &BuildContext,
        total: ResolvedSize,
        child_count: usize,
    ) -> (f32, f32) {
        let (max_main, total_main, legacy_alignment) = if self.is_row() {
            (
                ctx.box_constraint.max_width,
                total.width,
                self.horizontal_alignment,
            )
        } else {
            (
                ctx.box_constraint.max_height,
                total.height,
                self.vertical_alignment,
            )
        };
        let justify_content = self.justify_content.unwrap_or(match legacy_alignment {
            BoxAlignment::Start => JustifyContent::Start,
            BoxAlignment::Center => JustifyContent::Center,
            BoxAlignment::End => JustifyContent::End,
        });
        justify_distribution(
            justify_content,
            (max_main - total_main).max(0.0),
            child_count,
        )
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_ordinal: usize,
    ) -> Option<(Vec2d, BuildContext<'a>)> {
        if self.overflow_behavior == OverflowBehavior::Wrap {
            return self.retained_wrapped_child_layout(ctx, child, child_ordinal);
        }
        let index = self
            .children
            .live_start()
            .unwrap_or(0)
            .checked_add(child_ordinal)?;
        if self.children.get(index)?.id() != child.id() {
            return None;
        }

        let layout = self.flex_layout(ctx);
        if index >= layout.len() {
            return None;
        }
        let child_size = layout.size(index);
        let distribution = self.main_distribution(ctx, layout.total(), layout.len());
        let main = distribution.0 + layout.offset_f32(index) + distribution.1 * index as f32;
        let (offset_x, offset_y) = if self.is_row() {
            (
                main,
                align_offset(
                    self.vertical_alignment,
                    (ctx.box_constraint.max_height - child_size.height).max(0.0),
                ),
            )
        } else {
            (
                align_offset(
                    self.horizontal_alignment,
                    (ctx.box_constraint.max_width - child_size.width).max(0.0),
                ),
                main,
            )
        };
        let scale = ctx.scale.max(1.0);
        let offset = Vec2d {
            x: (offset_x * scale).round() / scale,
            y: (offset_y * scale).round() / scale,
        };
        let child_ctx = BuildContext {
            parent_size: child_size,
            box_constraint: BoxConstraint {
                min_width: 0.0,
                min_height: 0.0,
                max_width: child_size.width,
                max_height: child_size.height,
            },
            visible_rect: ctx
                .visible_rect
                .map(|(vx, vy, vw, vh)| (vx - offset.x, vy - offset.y, vw, vh)),
            ..ctx.clone()
        };
        Some((offset, child_ctx))
    }

    /// Resolves the children that `ctx.visible_rect` exposes.
    ///
    /// `base_main` is the main-axis shift the container's own alignment applies.
    /// Without a visible rectangle every child is in range, which keeps an
    /// unclipped flex behaving exactly as before.
    fn painted_range(
        &self,
        ctx: &BuildContext,
        layout: &FlexLayout,
        distribution: (f32, f32),
    ) -> Range<usize> {
        let (start, extent) = match ctx.visible_rect {
            Some((vx, vy, vw, vh)) => {
                if self.is_row() {
                    (vx, vw)
                } else {
                    (vy, vh)
                }
            }
            None if self.overflow_behavior == OverflowBehavior::Hidden => {
                // Hidden overflow establishes a finite painting box even at
                // the root, where no ancestor has supplied visible_rect. Keep
                // the child window aligned with that box so painting and hit
                // testing do not walk eager children that the clip discards.
                let extent = if self.is_row() {
                    ctx.box_constraint.max_width
                } else {
                    ctx.box_constraint.max_height
                };
                (0.0, extent)
            }
            None => return 0..layout.len(),
        };
        let (leading, between) = distribution;
        if between == 0.0 {
            let start = (start - leading) as f64;
            return layout.visible_range(start, start + extent as f64);
        }
        layout.visible_range_with_extra_space(
            start as f64,
            (start + extent) as f64,
            leading as f64,
            between as f64,
        )
    }
}

impl Drawable for RawFlex {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && ctx.box_constraint.max_width.is_finite()
            && ctx.box_constraint.max_height.is_finite()
            && ctx.box_constraint.max_width >= 0.0
            && ctx.box_constraint.max_height >= 0.0
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        if !self.can_paint_local_v2(ctx) || self.overflow_behavior != OverflowBehavior::Hidden {
            return None;
        }

        Some(aimer_cupid::draw_cmd_v2::Rect::new(
            0.0,
            0.0,
            ctx.box_constraint.max_width / ctx.scale,
            ctx.box_constraint.max_height / ctx.scale,
        ))
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.retained_child_layout(ctx, child, child_index)
            .map(|(_, child_ctx)| child_ctx)
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
        let (offset, child_ctx) = self.retained_child_layout(ctx, child, child_index)?;
        let size = child
            .retained_v2_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        let position = child.pos().unwrap_or_default();
        let scale = ctx.scale;
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                (offset.x + position.x) / scale,
                (offset.y + position.y) / scale,
                size.width / scale,
                size.height / scale,
            ),
            child.retained_clip(&child_ctx),
        ))
    }

    #[doc(hidden)]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        if self.overflow_behavior != OverflowBehavior::Wrap {
            self.prepare_paint_partition(ctx).is_some()
        } else {
            false
        }
    }

    fn update(&self, ctx: &BuildContext) {
        self.layout.invalidate_hit_test_index();
        let (gap_x, gap_y) = self.resole_gaps(ctx);
        let max_w = ctx.box_constraint.max_width;
        let max_h = ctx.box_constraint.max_height;

        ctx.canvas.save();

        if self.overflow_behavior == OverflowBehavior::Wrap {
            self.draw_wrapped(ctx, gap_x, gap_y);
            ctx.canvas.restore();
            return;
        }

        let is_row = self.is_row();
        let mut layout = self.flex_layout(ctx);
        // The container's justification distributes free space along the main
        // axis; the cross axis is aligned per child.
        let mut distribution = self.main_distribution(ctx, layout.total(), layout.len());
        let mut range = self.painted_range(ctx, &layout, distribution);

        // Bring the slice about to be painted into existence before anything
        // borrows a child. A windowed source drops what left the range here, so
        // this must stay the last point at which children can be dropped.
        self.children.window(range.clone(), ctx);

        for _ in 0..RECONCILE_PASSES {
            let outcome = self.reconcile(ctx, &layout, &range);
            if matches!(outcome, Reconciled::Matched) {
                break;
            }
            let rebuilt = matches!(outcome, Reconciled::Stale);
            if rebuilt {
                layout = self.measure_layout(ctx);
            } else {
                // The container's own size changed underneath a parent that has
                // already asked for it this frame — a scroll view reads
                // `content_size` before it draws its child. Asking for another
                // frame is what lets the scroll range catch up. It cannot loop:
                // a recorded child agrees with the table from now on.
                ctx.window.request_redraw();
            }
            distribution = self.main_distribution(ctx, layout.total(), layout.len());
            range = self.painted_range(ctx, &layout, distribution);
            self.children.window(range.clone(), ctx);
            if rebuilt {
                // A rebuilt table measured every child, so there is nothing left
                // to disagree with.
                break;
            }
        }

        // Painting resolved the whole table, so let a later `computed_size`
        // read the result straight back.
        self.cache
            .set_computed(ctx.box_constraint, scale_bits_of(ctx), layout.total());
        self.layout.set_painted(&range);

        // Children paint in layer order. The order is structural for this
        // element generation and the current painted range, so cached frames
        // reuse it without rebuilding or sorting a vector.
        let order = self.layout.cached_layer_order(range.clone(), |index| {
            self.children.get(index).map(|child| child.layer())
        });

        // Round child positions to device pixels so that adjacent backgrounds
        // always tile without sub-pixel seams.  Without this, a fractional
        // scroll offset combined with a float main-axis offset can place two
        // sibling rectangles on fractional device-pixel boundaries, and the
        // GPU anti-aliasing blends the gap with the parent background (white).
        let scale = ctx.scale.max(1.0);

        // A frame requested by scrolling alone cannot have changed any child
        // that was already recorded, so the ones the cache window prepared
        // beyond the screen are left until they reach it. Everything else, and
        // every other kind of frame, visits exactly as before.
        let on_screen = if aimer_widget::may_skip_settled_offscreen() {
            aimer_widget::retained_visible_window_of_current(ctx.scale)
        } else {
            None
        };
        order.visit(range.clone(), |index| {
            let Some(child) = self.children.get(index) else {
                return;
            };
            let child_size = layout.size(index);
            let c_w = child_size.width;
            let c_h = child_size.height;
            let main = distribution.0 + layout.offset_f32(index) + distribution.1 * index as f32;

            let (offset_x, offset_y) = if is_row {
                (
                    main,
                    align_offset(self.vertical_alignment, (max_h - c_h).max(0.0)),
                )
            } else {
                (
                    align_offset(self.horizontal_alignment, (max_w - c_w).max(0.0)),
                    main,
                )
            };

            // The index range only bounds the main axis, so a child can still
            // sit outside the viewport across it.
            if !ctx.is_rect_visible(offset_x, offset_y, c_w, c_h) {
                return;
            }
            if let Some(window) = &on_screen
                && !window.intersects(offset_x, offset_y, c_w, c_h)
                && aimer_widget::retained_element_settled(child.id())
            {
                return;
            }

            let draw_ctx = BuildContext {
                parent_size: child_size,
                box_constraint: BoxConstraint {
                    min_width: 0.0,
                    min_height: 0.0,
                    max_width: c_w,
                    max_height: c_h,
                },
                visible_rect: ctx
                    .visible_rect
                    .map(|(vx, vy, vw, vh)| (vx - offset_x, vy - offset_y, vw, vh)),
                ..ctx.clone()
            };

            let rx = (offset_x * scale).round() / scale;
            let ry = (offset_y * scale).round() / scale;

            draw_ctx.canvas.save();
            draw_ctx.canvas.translate(Vec2d { x: rx, y: ry });
            Self::render_child(child, &draw_ctx);
            draw_ctx.canvas.restore();
        });

        self.rebuild_hit_test_index(range);

        ctx.canvas.restore();
    }
}

impl VisitorElement for RawFlex {
    /// Visits the children that exist.
    ///
    /// An eager container holds all of them. A windowed one holds the rows the
    /// last frame needed, which is the [sparse-children
    /// contract](crate::flex::children_source): a row outside the window has no
    /// element yet, so there is nothing to visit.
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.children.visit(visitor);
    }

    fn debug_name(&self) -> &'static str {
        self.debug_name
    }
}

impl EventElement for RawFlex {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedHitTestBoundary
    }

    /// The event and visual child views are the same retained source.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.children.visit(visitor);
    }

    /// Offers every child that exists, painted or not.
    ///
    /// Focus and broadcast delivery use this, so an off-screen input field of an
    /// eager container still receives keys. A windowed container can only offer
    /// its live window — see the [sparse-children
    /// contract](crate::flex::children_source).
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.children.visit(visitor);
    }

    /// Offers only the children the last frame actually painted.
    ///
    /// A pointer can only land on something that was drawn, so a clipped list
    /// hit-tests its viewport slice instead of its whole child vector — the
    /// difference between a few dozen and a few hundred thousand visits per
    /// pointer move. Keyboard focus and broadcasts keep using
    /// [`EventElement::event_children`], so an off-screen field still receives
    /// them.
    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let Some(range) = self.layout.painted() else {
            return self.event_children(visitor);
        };
        for index in range.start..range.end.min(self.children.len()) {
            if let Some(child) = self.children.get(index) {
                visitor(child);
            }
        }
    }

    /// Visits the painted range in the order pointer routing consumes it, so a
    /// large sibling group does not materialize a temporary reverse-order
    /// buffer for every event.
    #[inline]
    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        let Some(range) = self.layout.painted() else {
            return self.children.visit_reversed(visitor);
        };
        for index in (range.start..range.end.min(self.children.len())).rev() {
            if let Some(child) = self.children.get(index) {
                visitor(child);
            }
        }
    }

    /// Visits painted children whose retained bounds contain `pos` in forward
    /// paint order. Unknown bounds remain candidates so a child that has not
    /// published geometry cannot be accidentally made non-interactive.
    #[inline]
    fn hit_test_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.visit_painted_children_at(pos, false, visitor);
    }

    /// Visits painted children whose retained bounds contain `pos` in routed
    /// topmost-first order.
    #[inline]
    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.visit_painted_children_at(pos, true, visitor);
    }
}

impl RawFlex {
    /// Rebuilds the sparse main-axis index from the bounds children retained
    /// while painting. Bounds are read once per painted child here, instead of
    /// once per pointer event in the routed walk.
    #[inline]
    fn rebuild_hit_test_index(&self, range: Range<usize>) {
        let children = self.children.as_ref();
        self.layout
            .rebuild_hit_test_index(range, self.is_row(), |index| {
                children.get(index).and_then(|child| child.pos_start_end())
            });
    }

    #[inline]
    fn visit_painted_children_at<'a>(
        &'a self,
        pos: Vec2d,
        reversed: bool,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        let Some(range) = self.layout.painted() else {
            if reversed {
                return self.children.visit_reversed(visitor);
            }
            return self.event_children(visitor);
        };

        let range = range.start..range.end.min(self.children.len());
        if let Some(candidates) = self.layout.hit_test_candidates(&range, pos) {
            if reversed {
                for &index in candidates.iter().rev() {
                    if let Some(child) = self.children.get(index)
                        && hit_test_contains(child, pos)
                    {
                        visitor(child);
                    }
                }
            } else {
                for &index in candidates {
                    if let Some(child) = self.children.get(index)
                        && hit_test_contains(child, pos)
                    {
                        visitor(child);
                    }
                }
            }
            return;
        }

        if reversed {
            for index in range.rev() {
                if let Some(child) = self.children.get(index)
                    && hit_test_contains(child, pos)
                {
                    visitor(child);
                }
            }
        } else {
            for index in range {
                if let Some(child) = self.children.get(index)
                    && hit_test_contains(child, pos)
                {
                    visitor(child);
                }
            }
        }
    }
}

#[inline]
fn hit_test_contains(element: &dyn Element, pos: Vec2d) -> bool {
    element.pos_start_end().is_none_or(|(start, end)| {
        pos.x >= start.x && pos.x <= end.x && pos.y >= start.y && pos.y <= end.y
    })
}

impl Rebuildable for RawFlex {
    /// Only hands a mark on to its child, so placing it again leaves the
    /// dirty-path index as it was; see [`Rebuildable::forwards_marks_only`].
    #[inline]
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Reaches only the children a dirty path runs through, by position.
    ///
    /// A list of hundreds of rows with one animating row would otherwise enter
    /// every row on every frame just to prune it.
    fn rebuild_indexed_children(&self, ctx: &BuildContext) -> bool {
        aimer_widget::rebuild_dirty_children(
            self.children.len(),
            |index| self.children.get(index),
            ctx,
        )
    }

    /// Takes over the rows and the measured table of the container being
    /// replaced.
    ///
    /// Both escape ordinary reconciliation. A windowed container starts life
    /// empty, so the positional walk finds no rows to pair — see the
    /// [sparse-children contract](crate::flex::children_source). The table is not
    /// a child at all, and it holds every extent the list learned by painting, so
    /// dropping it would snap a predicted scroll extent back to its prediction.
    ///
    /// The rows are claimed by identity, so they transfer even when the data
    /// changed. The table describes positions, so it only transfers when nothing
    /// that decides them did: the same number of children, laid out under the
    /// same rules.
    ///
    /// The rows the replacement builds are new elements either way, so only the
    /// part of the table that never described a particular child survives the
    /// handover — see [`FlexLayoutCache::adopt`].
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|any| any.downcast_ref::<RawFlex>())
        else {
            return;
        };

        self.children.adopt_retained(old.children.take_retained());

        if old.children.len() == self.children.len()
            && old.direction == self.direction
            && old.gaps == self.gaps
            && old.overflow_behavior == self.overflow_behavior
            && old.item_extent == self.item_extent
        {
            // The replacement may already have been measured before this
            // handoff. Drop that provisional table so the learned table from
            // the old container can be adopted instead of being ignored by
            // `FlexLayoutCache::adopt`'s empty-slot guard.
            self.layout.invalidate();
            self.layout.adopt(&old.layout);
        }

        // A replacement can be measured once before reconciliation reaches
        // this element. That measurement describes the replacement's initial
        // window, not the retained rows/table adopted above, so keeping it
        // would make the next scrollable preflight report the stale extent.
        // Retain the flex table itself, but force computed_size to consult it.
        self.cache.invalidate();
    }
}

impl LayoutElement for RawFlex {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_computed(ctx.box_constraint, scale_bits) {
            return cached;
        }

        if self.overflow_behavior == OverflowBehavior::Wrap {
            return self.wrapped_layout_cached(ctx).1.size;
        }

        let layout = self.flex_layout(ctx);
        let result = layout.total();
        self.cache
            .set_computed(ctx.box_constraint, scale_bits, result);
        result
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    /// Only a size this flex was given: its extent follows from all of its
    /// children laid out along an axis, which the default — the largest child —
    /// would understate, and a window of lazy children has no elements to ask.
    fn get_size_from_child(&self) -> Option<aimer_attribute::size::Size> {
        self.size()
    }

    /// Drops this container's tables and those of every child that exists.
    ///
    /// The live window itself is kept: a resize invalidates layout on every
    /// frame it produces, and rebuilding the visible rows each time would reset
    /// their state while the user is still dragging the window edge.
    fn invalidate_layout(&self) {
        self.cache.invalidate();
        self.layout.invalidate();
        self.children.visit(&mut |child| child.invalidate_layout());
    }
}
