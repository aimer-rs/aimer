//! The default content of a menu: one row per verb.
//!
//! `ContextMenuRows` is what [`crate::ContextMenu`] builds when it was given
//! items rather than a child, and it is a perfectly ordinary widget — so a menu
//! that wants something else can put the rows inside its own layout, or leave
//! them out entirely.
//!
//! A parent element measures the labels and routes hits, while each row owns a
//! retained paint node. Measuring the labels once for the whole panel decides
//! its width; a press is answered from the same row rectangles used for paint.

mod geometry;

use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use aimer_attribute::{Bounds, ResolvedSize, Vec2d};
use aimer_cupid::canvas::TextMetrics;
use aimer_events::element::ElementEvent;
use aimer_macro::{PortableWidget, Rebuildable};
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, LayoutElement, PointerKey,
    VisitorElement, Widget,
};

use crate::dismiss::ContextMenuDismiss;
use crate::item::ContextMenuItem;
use crate::portable::PortableMenuItems;
use crate::shape::ContextMenuShape;
use crate::style::ContextMenuStyle;

/// The rows of a context menu, laid out in the menu's shape.
///
/// # Examples
///
/// ```
/// use aimer_ctxmenu::{ContextMenuItem, ContextMenuRows, ContextMenuShape};
///
/// let rows = ContextMenuRows::new()
///     .shape(ContextMenuShape::List)
///     .items(vec![
///         ContextMenuItem::new("Copy"),
///         ContextMenuItem::new("Select All"),
///     ])
///     .on_select(|index| println!("chose row {index}"));
///
/// assert_eq!(rows.len(), 2);
/// ```
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_ctxmenu::ContextMenuRows",
    materializer = materialize_portable_context_menu_rows
)]
pub struct ContextMenuRows {
    items: PortableMenuItems,
    shape: ContextMenuShape,
    style: ContextMenuStyle,
    #[portable_skip]
    on_select: Option<Rc<dyn Fn(usize)>>,
    #[portable_skip]
    dismiss: ContextMenuDismiss,
    dismiss_on_select: bool,
}

fn materialize_portable_context_menu_rows(
    document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
    node: aimer_widget::portable::__anteros::WidgetNodeView<'_>,
    children: Vec<aimer_widget::AnyWidget>,
) -> Result<aimer_widget::AnyWidget, aimer_widget::portable::PortableMaterializeError> {
    if !children.is_empty() {
        return Err(
            aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                expected: 0,
                actual: children.len(),
            },
        );
    }

    let shape: ContextMenuShape = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenuRows:shape",
        ),
    )?;
    let style: ContextMenuStyle = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenuRows:style",
        ),
    )?;
    let items: PortableMenuItems = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenuRows:items",
        ),
    )?;
    let dismiss_on_select: bool = aimer_widget::portable::required_materialized_property(
        document,
        &node,
        aimer_widget::portable::__anteros::PropertyId::from_canonical_name(
            "aimer.property:aimer_ctxmenu::ContextMenuRows:dismiss_on_select",
        ),
    )?;

    Ok(ContextMenuRows::new()
        .shape(shape)
        .style(style)
        .items(items.into_vec())
        .dismiss_on_select(dismiss_on_select)
        .boxed())
}

impl Default for ContextMenuRows {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenuRows {
    /// Creates an empty set of rows in the default shape and look.
    #[inline]
    pub fn new() -> Self {
        Self {
            items: PortableMenuItems::default(),
            shape: ContextMenuShape::default(),
            style: ContextMenuStyle::default(),
            on_select: None,
            dismiss: ContextMenuDismiss::new(),
            dismiss_on_select: true,
        }
    }

    /// Sets the shape, and with it the default look of that shape.
    ///
    /// Call [`ContextMenuRows::style`] *after* this to keep a custom look.
    #[inline]
    pub fn shape(mut self, shape: ContextMenuShape) -> Self {
        self.shape = shape;
        self.style = ContextMenuStyle::for_shape(shape);
        self
    }

    /// Sets the look of the rows.
    #[inline]
    pub fn style(mut self, style: ContextMenuStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the rows, in the order they are drawn.
    #[inline]
    pub fn items(mut self, items: Vec<ContextMenuItem>) -> Self {
        self.items = PortableMenuItems::new(items);
        self
    }

    /// Appends one row.
    #[inline]
    pub fn item(mut self, item: ContextMenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Sets what happens when a row is chosen, by its index.
    ///
    /// This runs *after* the chosen item's own action, so a menu may use either
    /// or both.
    #[inline]
    pub fn on_select(mut self, on_select: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Closes the menu identified by `dismiss` when a row is chosen.
    #[inline]
    pub fn dismiss_with(mut self, dismiss: ContextMenuDismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// Controls whether choosing a row closes the menu.
    #[inline]
    pub fn dismiss_on_select(mut self, dismiss_on_select: bool) -> Self {
        self.dismiss_on_select = dismiss_on_select;
        self
    }

    /// How many rows there are.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no rows at all.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Widget for ContextMenuRows {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let items = self.items.into_vec();
        let style = Rc::new(self.style);
        let paint_state = Rc::new(ContextMenuRowPaintState::default());
        let row_count = items.len();
        let row_elements = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                RawContextMenuRow {
                    index,
                    row_count,
                    shape: self.shape,
                    label: Arc::from(item.label()),
                    enabled: item.is_enabled(),
                    style: Rc::clone(&style),
                    paint_state: Rc::clone(&paint_state),
                    painted_highlight: Cell::new(false),
                }
                .boxed()
            })
            .collect();
        RawContextMenuRows {
            items,
            shape: self.shape,
            style,
            on_select: self.on_select.clone(),
            dismiss: self.dismiss.clone(),
            dismiss_on_select: self.dismiss_on_select,
            row_elements,
            rows: RefCell::new(Vec::new()),
            bounds: InteractionBounds::new(),
            label_widths: RefCell::new(Vec::new()),
            measured_scale: Cell::new(0.0),
            local_rows: RefCell::new(Vec::new()),
            geometry_scale: Cell::new(0.0),
            pressed: Cell::new(None),
            pressed_by: Cell::new(None),
            hovered: Cell::new(None),
            paint_state,
            window: Some(ctx.window.clone()),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ContextMenuRows"
    }
}

/// The element behind [`ContextMenuRows`].
#[derive(Rebuildable)]
pub(crate) struct RawContextMenuRows {
    items: Vec<ContextMenuItem>,
    shape: ContextMenuShape,
    style: Rc<ContextMenuStyle>,
    on_select: Option<Rc<dyn Fn(usize)>>,
    dismiss: ContextMenuDismiss,
    dismiss_on_select: bool,
    row_elements: Vec<AnyElement>,
    /// Where each row was last painted, in absolute logical coordinates —
    /// the space a pointer event arrives in.
    rows: RefCell<Vec<Bounds>>,
    /// The rows' whole area. Once the render tree supplies it, each row's
    /// absolute rectangle is its local rectangle plus this area's origin, and
    /// `rows` is only the canvas-derived fallback.
    bounds: InteractionBounds,
    /// The measured label widths, in logical pixels, and the scale they were
    /// measured at, so a frame that changes neither measures nothing.
    label_widths: RefCell<Vec<f32>>,
    measured_scale: Cell<f32>,
    /// The local logical rectangles shared by paint, child geometry and hits.
    local_rows: RefCell<Vec<Bounds>>,
    geometry_scale: Cell<f32>,
    pressed: Cell<Option<usize>>,
    pressed_by: Cell<Option<PointerKey>>,
    hovered: Cell<Option<usize>>,
    paint_state: Rc<ContextMenuRowPaintState>,
    window: Option<WindowHandle>,
}

#[derive(Default)]
struct ContextMenuRowPaintState {
    highlighted: Cell<Option<usize>>,
    panel_size: Cell<ResolvedSize>,
    text_metrics: Cell<Option<TextMetrics>>,
    metrics_scale: Cell<f32>,
}

/// One menu item and the only retained paint owner for its visuals.
#[derive(Rebuildable)]
struct RawContextMenuRow {
    index: usize,
    row_count: usize,
    shape: ContextMenuShape,
    label: Arc<str>,
    enabled: bool,
    style: Rc<ContextMenuStyle>,
    paint_state: Rc<ContextMenuRowPaintState>,
    painted_highlight: Cell<bool>,
}

impl RawContextMenuRows {
    /// The label widths in logical pixels, measured at most once per scale.
    fn label_widths(&self, ctx: &BuildContext) -> Vec<f32> {
        let scale = scale_of(ctx);
        if self.measured_scale.get() == scale {
            let cached = self.label_widths.borrow();
            if cached.len() == self.items.len() {
                return cached.clone();
            }
        }
        let font_size = self.style.label.font_size as f32 * scale;
        let widths = self
            .items
            .iter()
            .map(|item| {
                ctx.canvas.measure_text_styled(
                    item.label(),
                    font_size,
                    self.style.label.font_family,
                    self.style.label.font_style,
                    self.style.label.font_weight.numeric(),
                ) / scale
            })
            .collect::<Vec<_>>();
        *self.label_widths.borrow_mut() = widths.clone();
        self.measured_scale.set(scale);
        widths
    }

    /// The room the rows need, in physical pixels.
    fn intrinsic_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let scale = scale_of(ctx);
        let (width, height) =
            geometry::content_size(self.shape, &self.style, &self.label_widths(ctx));
        ResolvedSize {
            width: width * scale,
            height: height * scale,
        }
    }

    /// Local logical row rectangles, cached for the current scale and item set.
    fn local_row_rects(&self, ctx: &BuildContext) -> Ref<'_, Vec<Bounds>> {
        let scale = scale_of(ctx);
        let needs_update = self.geometry_scale.get() != scale
            || self.local_rows.borrow().len() != self.items.len();
        if needs_update {
            let widths = self.label_widths(ctx);
            *self.local_rows.borrow_mut() =
                geometry::row_rects(self.shape, &self.style, Vec2d { x: 0.0, y: 0.0 }, &widths);
            self.geometry_scale.set(scale);
        }
        self.local_rows.borrow()
    }

    /// Text metrics shared by all rows at the current scale.
    fn text_metrics(&self, ctx: &BuildContext) -> TextMetrics {
        let scale = scale_of(ctx);
        if self.paint_state.metrics_scale.get() == scale
            && let Some(metrics) = self.paint_state.text_metrics.get()
        {
            return metrics;
        }
        let Some(first) = self.items.first() else {
            return TextMetrics::default();
        };
        let metrics = ctx.canvas.measure_text_metrics_styled(
            first.label(),
            self.style.label.font_size as f32 * scale,
            0.0,
            self.style.label.font_family,
            self.style.label.font_style,
            self.style.label.font_weight.numeric(),
        );
        self.paint_state.text_metrics.set(Some(metrics));
        self.paint_state.metrics_scale.set(scale);
        metrics
    }

    /// A change to the shared highlight schedules row nodes to repaint.
    fn sync_highlight(&self) -> bool {
        let highlighted = self.pressed.get().or(self.hovered.get());
        self.paint_state.highlighted.replace(highlighted) != highlighted
    }

    fn request_row_repaint(&self, changed: bool) -> bool {
        if !changed {
            return false;
        }
        if let Some(window) = &self.window {
            window.request_animation_frame();
            false
        } else {
            true
        }
    }

    /// The rows in absolute logical coordinates, the space a pointer event
    /// arrives in: from the render tree once it supplies the menu's origin,
    /// otherwise where the canvas last painted them.
    fn hit_rows(&self) -> Vec<Bounds> {
        if self.bounds.is_adopted()
            && let Some(origin) = self.bounds.get_bounds()
        {
            return self
                .local_rows
                .borrow()
                .iter()
                .map(|row| Bounds::new(row.x + origin.x, row.y + origin.y, row.width, row.height))
                .collect();
        }
        self.rows.borrow().clone()
    }

    /// The index of the *choosable* row under an absolute logical position.
    fn enabled_at(&self, pos: Vec2d) -> Option<usize> {
        let rows = self.hit_rows();
        let index = geometry::row_at(self.shape, &rows, pos.x, pos.y)?;
        self.items
            .get(index)
            .filter(|item| item.is_enabled())
            .map(|_| index)
    }

    /// Whether an absolute logical position landed on the rows at all.
    fn contains(&self, pos: Vec2d) -> bool {
        geometry::union(&self.hit_rows())
            .is_some_and(|bounds| geometry::contains(bounds, pos.x, pos.y))
    }

    /// Runs the chosen row and closes the menu unless told not to.
    fn choose(&self, index: usize) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        item.run();
        if let Some(on_select) = &self.on_select {
            on_select(index);
        }
        if self.dismiss_on_select {
            self.dismiss.dismiss();
        }
    }

    #[cfg(test)]
    pub(crate) fn place_for_test(&self, rows: Vec<Bounds>) {
        *self.rows.borrow_mut() = rows;
    }
}

impl Drawable for RawContextMenuRows {
    /// The menu hit-tests the rows' whole area, which is also its node.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("ContextMenuRows")
    }

    fn update(&self, ctx: &BuildContext) {
        // The rows' washes, separators and labels are recorded by the row
        // elements themselves. This traversal publishes where the rows are and
        // visits them.
        self.sync_paint_geometry(ctx);
        self.draw_local_v2_compatibility(ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        !self.items.is_empty() && scale_of(ctx).is_finite() && scale_of(ctx) > 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        let scale = scale_of(ctx);
        let local = self.local_row_rects(ctx);
        for (index, row) in self.row_elements.iter().enumerate() {
            let Some(rect) = local.get(index).copied() else {
                continue;
            };
            let mut row_ctx = ctx.clone();
            row_ctx.parent_size = ResolvedSize {
                width: rect.width * scale,
                height: rect.height * scale,
            };
            row_ctx.box_constraint.max_width = row_ctx.parent_size.width;
            row_ctx.box_constraint.max_height = row_ctx.parent_size.height;
            ctx.canvas.save();
            ctx.canvas.translate(Vec2d {
                x: rect.x * scale,
                y: rect.y * scale,
            });
            row.update(&row_ctx);
            ctx.canvas.restore();
        }
    }

    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        false
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        if self.items.is_empty() {
            self.rows.borrow_mut().clear();
            return;
        }
        let scale = scale_of(ctx);
        let local = self.local_row_rects(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let area = self.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, area.width, area.height);
        *self.rows.borrow_mut() = local
            .iter()
            .map(|rect| {
                Bounds::new(
                    rect.x + abs_x / scale,
                    rect.y + abs_y / scale,
                    rect.width,
                    rect.height,
                )
            })
            .collect();
        self.paint_state.panel_size.set(self.intrinsic_size(ctx));
        let _ = self.text_metrics(ctx);
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        let rect = self.child_row_rect(ctx, child, child_index)?;
        let scale = scale_of(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = ResolvedSize {
            width: rect.width * scale,
            height: rect.height * scale,
        };
        child_ctx.box_constraint.max_width = child_ctx.parent_size.width;
        child_ctx.box_constraint.max_height = child_ctx.parent_size.height;
        Some(child_ctx)
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
        let rect = self.child_row_rect(ctx, child, child_index)?;
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(rect.x, rect.y, rect.width, rect.height),
            None,
        ))
    }
}

impl RawContextMenuRows {
    fn child_row_rect(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<Bounds> {
        let expected = self.row_elements.get(child_index)?;
        if !std::ptr::eq(child, expected.as_ref()) {
            return None;
        }
        self.local_row_rects(ctx).get(child_index).copied()
    }
}

impl RawContextMenuRow {
    fn text_metrics(&self, ctx: &BuildContext) -> TextMetrics {
        let scale = scale_of(ctx);
        if self.paint_state.metrics_scale.get() == scale
            && let Some(metrics) = self.paint_state.text_metrics.get()
        {
            return metrics;
        }
        ctx.canvas.measure_text_metrics_styled(
            &self.label,
            self.style.label.font_size as f32 * scale,
            0.0,
            self.style.label.font_family,
            self.style.label.font_style,
            self.style.label.font_weight.numeric(),
        )
    }

    fn radii(&self, scale: f32, row_size: ResolvedSize) -> [f32; 4] {
        if !self.shape.is_horizontal() {
            return [0.0; 4];
        }
        let mut panel = self.paint_state.panel_size.get();
        if panel.width <= 0.0 || panel.height <= 0.0 {
            panel = row_size;
        }
        let [tl, tr, br, bl] = self.style.panel_radius(panel.width, panel.height, scale);
        let first = self.index == 0;
        let last = self.index + 1 == self.row_count;
        [
            if first { tl } else { 0.0 },
            if last { tr } else { 0.0 },
            if last { br } else { 0.0 },
            if first { bl } else { 0.0 },
        ]
    }
}

impl Drawable for RawContextMenuRow {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let size = ctx.parent_size;
        scale_of(ctx).is_finite()
            && scale_of(ctx) > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let scale = scale_of(ctx);
        let size = ResolvedSize {
            width: ctx.parent_size.width / scale,
            height: ctx.parent_size.height / scale,
        };
        let metrics = self.text_metrics(ctx);
        let lit = self.paint_state.highlighted.get() == Some(self.index);
        let canvas = aimer_canvas::Canvas::of(ctx);
        if lit {
            let (red, green, blue, alpha) = self.style.highlight_color.to_rgba();
            let [tl, tr, br, bl] = self
                .radii(scale, ctx.parent_size)
                .map(|radius| radius / scale);
            canvas.fill_rect_styled(
                aimer_cupid::utilities::Rect::new(0.0, 0.0, size.width, size.height),
                aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
                [tl, tr, br, bl],
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
                [0.0; 4],
                aimer_cupid::utilities::Color::transparent(),
            );
        }
        if self.shape.is_horizontal() && self.index > 0 {
            let (red, green, blue, alpha) = self.style.separator_color.to_rgba();
            canvas.fill_rect(
                aimer_cupid::utilities::Rect::new(0.0, size.height * 0.25, 1.0, size.height * 0.5),
                [red, green, blue, alpha],
            );
        }
        let color = if self.enabled {
            self.style.label.color
        } else {
            self.style.disabled_label_color
        };
        let (red, green, blue, alpha) = color.to_rgba();
        canvas.draw_text_styled(
            Arc::clone(&self.label),
            aimer_cupid::utilities::Vec2d::new(
                self.style.item_padding,
                (size.height - (metrics.ascent - metrics.descent) / scale) * 0.5
                    + metrics.ascent / scale,
            ),
            self.style.label.font_size as f32,
            aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
            None,
            None,
            aimer_cupid::text_pipeline::TextOverflowMode::Clip,
            aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Left,
            self.style.label.font_family,
            self.style.label.font_style,
            self.style.label.font_weight.numeric(),
            None,
            true,
        );
        canvas.finish();
        self.painted_highlight.set(lit);
    }

    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        (self.paint_state.highlighted.get() == Some(self.index)) != self.painted_highlight.get()
    }
}

impl EventElement for RawContextMenuRow {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}
}

impl LayoutElement for RawContextMenuRow {}

impl VisitorElement for RawContextMenuRow {
    fn debug_name(&self) -> &'static str {
        "ContextMenuRow"
    }
}

impl EventElement for RawContextMenuRows {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::PointerDown(info) => {
                if !self.contains(info.pos) {
                    return EventResult::ignored();
                }
                let pointer = PointerKey::new(info.source, info.id);
                self.pressed.set(self.enabled_at(info.pos));
                self.pressed_by.set(Some(pointer));
                let needs_redraw = self.request_row_repaint(self.sync_highlight());
                let result = EventResult::consumed().with_pointer_capture(pointer);
                if needs_redraw {
                    result.with_redraw()
                } else {
                    result
                }
            }
            ElementEvent::PointerMove(info) => {
                let over = self.enabled_at(info.pos);
                self.hovered.set(over);
                let needs_redraw = self.request_row_repaint(self.sync_highlight());
                // A finger sliding off the row it pressed must un-arm it, the
                // way every button does.
                if self.pressed_by.get() == Some(PointerKey::new(info.source, info.id)) {
                    let result = EventResult::consumed();
                    return if needs_redraw {
                        result.with_redraw()
                    } else {
                        result
                    };
                }
                let result = if self.contains(info.pos) {
                    EventResult::consumed()
                } else {
                    EventResult::ignored()
                };
                if needs_redraw {
                    result.with_redraw()
                } else {
                    result
                }
            }
            ElementEvent::PointerUp(info) => {
                let pointer = PointerKey::new(info.source, info.id);
                if self.pressed_by.get() != Some(pointer) {
                    return EventResult::ignored();
                }
                self.pressed_by.set(None);
                let pressed = self.pressed.take();
                let _ = self.request_row_repaint(self.sync_highlight());
                if let Some(index) =
                    pressed.filter(|index| self.enabled_at(info.pos) == Some(*index))
                {
                    self.choose(index);
                }
                EventResult::consumed()
                    .with_redraw()
                    .with_pointer_release(pointer)
            }
            ElementEvent::Cancel => {
                self.pressed.set(None);
                self.pressed_by.set(None);
                self.hovered.set(None);
                let _ = self.request_row_repaint(self.sync_highlight());
                EventResult::ignored()
            }
            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn aimer_widget::Element)) {}
}

impl LayoutElement for RawContextMenuRows {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.intrinsic_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.intrinsic_size(ctx)
    }
}

impl VisitorElement for RawContextMenuRows {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn aimer_widget::Element)) {
        for row in &self.row_elements {
            visitor(row.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "ContextMenuRows"
    }
}

/// The device pixel ratio, never zero.
#[inline]
fn scale_of(ctx: &BuildContext) -> f32 {
    if ctx.scale > 0.0 { ctx.scale } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};

    use super::*;

    fn rows(items: Vec<ContextMenuItem>, dismiss: ContextMenuDismiss) -> RawContextMenuRows {
        let element = RawContextMenuRows {
            items,
            shape: ContextMenuShape::List,
            style: Rc::new(ContextMenuStyle::list()),
            on_select: None,
            dismiss,
            dismiss_on_select: true,
            row_elements: Vec::new(),
            rows: RefCell::new(Vec::new()),
            bounds: InteractionBounds::new(),
            label_widths: RefCell::new(Vec::new()),
            measured_scale: Cell::new(0.0),
            local_rows: RefCell::new(Vec::new()),
            geometry_scale: Cell::new(0.0),
            pressed: Cell::new(None),
            pressed_by: Cell::new(None),
            hovered: Cell::new(None),
            paint_state: Rc::new(ContextMenuRowPaintState::default()),
            window: None,
        };
        element.place_for_test(vec![
            Bounds::new(10.0, 10.0, 150.0, 28.0),
            Bounds::new(10.0, 38.0, 150.0, 28.0),
        ]);
        element
    }

    fn press(x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerDown(pointer_at(x, y))
    }

    fn release(x: f32, y: f32) -> ElementEvent {
        ElementEvent::PointerUp(pointer_at(x, y))
    }

    fn pointer_at(x: f32, y: f32) -> PointerInfo {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Mouse,
            0,
            PointerButton::Primary,
        )
    }

    #[test]
    fn choosing_a_row_runs_its_action() {
        let ran = Rc::new(Cell::new(None));
        let first = Rc::clone(&ran);
        let second = Rc::clone(&ran);
        let element = rows(
            vec![
                ContextMenuItem::new("Copy").on_select(move || first.set(Some(0))),
                ContextMenuItem::new("Paste").on_select(move || second.set(Some(1))),
            ],
            ContextMenuDismiss::new(),
        );

        assert!(element.on_event(&press(20.0, 45.0)).is_consumed());
        assert!(element.on_event(&release(20.0, 45.0)).is_consumed());

        assert_eq!(ran.get(), Some(1));
    }

    #[test]
    fn a_release_that_slid_off_the_row_runs_nothing() {
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        let element = rows(
            vec![ContextMenuItem::new("Copy").on_select(move || flag.set(true))],
            ContextMenuDismiss::new(),
        );

        let _ = element.on_event(&press(20.0, 20.0));
        let _ = element.on_event(&release(400.0, 400.0));

        assert!(!ran.get());
    }

    #[test]
    fn a_disabled_row_takes_the_press_but_runs_nothing() {
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        let element = rows(
            vec![
                ContextMenuItem::new("Paste")
                    .enabled(false)
                    .on_select(move || flag.set(true)),
            ],
            ContextMenuDismiss::new(),
        );

        assert!(
            element.on_event(&press(20.0, 20.0)).is_consumed(),
            "the panel still swallows it"
        );
        let _ = element.on_event(&release(20.0, 20.0));

        assert!(!ran.get());
    }

    #[test]
    fn a_press_that_missed_the_rows_is_left_to_the_barrier() {
        let element = rows(
            vec![ContextMenuItem::new("Copy")],
            ContextMenuDismiss::new(),
        );

        assert!(
            !element.on_event(&press(400.0, 400.0)).is_consumed(),
            "so an outside press still dismisses the menu"
        );
    }

    #[test]
    fn only_the_pointer_that_pressed_can_choose() {
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        let element = rows(
            vec![ContextMenuItem::new("Copy").on_select(move || flag.set(true))],
            ContextMenuDismiss::new(),
        );
        let _ = element.on_event(&press(20.0, 20.0));

        let other = ElementEvent::PointerUp(PointerInfo::new(
            Vec2d { x: 20.0, y: 20.0 },
            PointerSource::Touch,
            7,
            PointerButton::Primary,
        ));

        assert!(!element.on_event(&other).is_consumed());
        assert!(!ran.get(), "a second finger cannot finish someone's press");
    }

    #[test]
    fn a_cancelled_gesture_forgets_the_press() {
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        let element = rows(
            vec![ContextMenuItem::new("Copy").on_select(move || flag.set(true))],
            ContextMenuDismiss::new(),
        );

        let _ = element.on_event(&press(20.0, 20.0));
        let _ = element.on_event(&ElementEvent::Cancel);
        let _ = element.on_event(&release(20.0, 20.0));

        assert!(!ran.get());
    }

    #[test]
    fn choosing_a_row_closes_the_menu_by_default() {
        let dismiss = ContextMenuDismiss::new();
        let element = rows(vec![ContextMenuItem::new("Copy")], dismiss.clone());

        let _ = element.on_event(&press(20.0, 20.0));
        let _ = element.on_event(&release(20.0, 20.0));

        assert!(dismiss.was_asked_to_dismiss());
    }

    #[test]
    fn a_menu_told_to_stay_open_is_not_dismissed_by_a_choice() {
        let dismiss = ContextMenuDismiss::new();
        let element = RawContextMenuRows {
            dismiss_on_select: false,
            ..rows(vec![ContextMenuItem::new("Select All")], dismiss.clone())
        };

        let _ = element.on_event(&press(20.0, 20.0));
        let _ = element.on_event(&release(20.0, 20.0));

        assert!(!dismiss.was_asked_to_dismiss());
    }

    #[test]
    fn the_index_callback_hears_which_row_was_chosen() {
        let chosen = Rc::new(Cell::new(None));
        let seen = Rc::clone(&chosen);
        let element = RawContextMenuRows {
            on_select: Some(Rc::new(move |index| seen.set(Some(index)))),
            ..rows(
                vec![
                    ContextMenuItem::new("Copy"),
                    ContextMenuItem::new("Select All"),
                ],
                ContextMenuDismiss::new(),
            )
        };

        let _ = element.on_event(&press(20.0, 45.0));
        let _ = element.on_event(&release(20.0, 45.0));

        assert_eq!(chosen.get(), Some(1));
    }

    #[test]
    fn context_menu_rows_publishes_a_derived_leaf_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <ContextMenuRows as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::none()
        );
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use aimer_container::ZeroSizedBox;
    use aimer_widget::portable::{
        __anteros::WidgetDocumentView, PortableBuildContext, PortableLimits, PortableNativeWidget,
        PortableWidgetLimits, PortableWidgetSchema, SourceFingerprint, StableId128,
        linked_portable_native_widget_registrations,
    };
    use aimer_widget::{PortableWidget, Widget};

    use super::{ContextMenuItem, ContextMenuRows, ContextMenuShape, ContextMenuStyle};

    fn source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            7,
            11,
            PortableWidgetLimits::new(16, 32, 16, 16, 2_048, 16_384).with_max_blob_bytes(8_192),
            PortableLimits::new(16, 512, 2_048, 8_192, 16_384),
        )
        .unwrap()
    }

    #[test]
    fn rows_lower_round_trip_and_register_a_checked_host_materializer() {
        let mut context = context();
        let root = ContextMenuRows::new()
            .shape(ContextMenuShape::List)
            .style(ContextMenuStyle::list().row_height(31.0))
            .items(vec![
                ContextMenuItem::new("Copy"),
                ContextMenuItem::new("Paste").enabled(false),
            ])
            .dismiss_on_select(false)
            .to_portable_node(&mut context, source(1))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();

        assert_eq!(node.children().count(), 0);
        assert_eq!(node.properties().count(), 4);
        let materialized =
            <ContextMenuRows as PortableNativeWidget>::materialize_widget(&view, node, Vec::new())
                .unwrap();
        assert_eq!(materialized.debug_name(), "ContextMenuRows");

        let schema = <ContextMenuRows as PortableWidgetSchema>::SCHEMA;
        assert!(
            linked_portable_native_widget_registrations()
                .iter()
                .any(|registration| registration
                    .supports(schema.widget().id(), schema.widget().min_version()))
        );
    }

    #[test]
    fn rows_materialization_rejects_unexpected_children_before_construction() {
        let mut context = context();
        let root = ContextMenuRows::new()
            .to_portable_node(&mut context, source(2))
            .unwrap();
        let document = context.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();

        assert!(matches!(
            <ContextMenuRows as PortableNativeWidget>::materialize_widget(
                &view,
                node,
                vec![ZeroSizedBox.boxed()],
            ),
            Err(
                aimer_widget::portable::PortableMaterializeError::InvalidChildCount {
                    expected: 0,
                    actual: 1,
                }
            )
        ));
    }
}
