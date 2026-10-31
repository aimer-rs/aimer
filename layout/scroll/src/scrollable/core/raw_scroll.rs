use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::InteractionBounds;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, Element, EventDispatcher, Rebuildable, element_tree_generation,
    layout_invalidation_generation,
};

pub use crate::scrollable::controller::DragMode;
use crate::scrollable::controller::ScrollState;
use crate::scrollable::scroll_bar::ScrollBarPlacement;

#[derive(Clone, Copy, PartialEq)]
struct ScrollLayoutKey {
    constraint: BoxConstraint,
    parent_size: ResolvedSize,
    scale_bits: u32,
    axis: crate::ScrollAxis,
    scroll_bar_placement: ScrollBarPlacement,
    viewport_w: f32,
    viewport_h: f32,
    vertical_bar_width: f32,
    horizontal_bar_height: f32,
    tree_generation: u64,
    layout_generation: u64,
}

#[derive(Clone, Copy)]
struct CachedScrollLayout {
    key: ScrollLayoutKey,
    extents: ((f32, f32), (f32, f32)),
    content_size: Option<ResolvedSize>,
}

/// Memoizes the scroll viewport and content measurements independently of the
/// live scroll offset.
///
/// The key includes the parent size as well as the child-facing constraint:
/// an unbounded scroll axis can resolve its viewport from the parent's size.
/// Scroll offset is deliberately absent, so a frame that only translates the
/// content keeps the same layout snapshot. Tree and layout generations retire
/// the snapshot when the content or its constraints can have changed.
#[derive(Default)]
pub(crate) struct ScrollLayoutCache {
    snapshot: Cell<Option<CachedScrollLayout>>,
}

pub struct RawScrollableContainer<E: Element> {
    pub(crate) child: E,
    /// The live scroll engine. Held behind an `Rc` so an app-supplied
    /// [`ScrollController`](crate::ScrollController) can share the very same
    /// state and drive it programmatically across rebuilds.
    pub(crate) ctrl: Rc<ScrollState>,
    pub(crate) vertical_scroll_bar: Option<AnyElement>,
    pub(crate) horizontal_scroll_bar: Option<AnyElement>,
    pub(crate) viewport_w: f32,
    pub(crate) viewport_h: f32,
    pub(crate) vertical_bar_width: f32,
    pub(crate) horizontal_bar_height: f32,
    pub(crate) bounds: InteractionBounds,
    pub(crate) event_dispatcher: RefCell<EventDispatcher>,
    /// Routes mouse moves and exits to the active scroll bar.
    ///
    /// The bar is not part of the content subtree `event_dispatcher` is rooted
    /// at, so without this its [`MouseRegion`](aimer_input::mouse_region::MouseRegion)
    /// would never hear the pointer. It is a separate dispatcher because
    /// alternating roots on one would rebuild the content's path index on every
    /// move that crosses between them.
    pub(crate) bar_dispatcher: RefCell<EventDispatcher>,
    pub(crate) layout_cache: ScrollLayoutCache,
}

impl<E: Element + 'static> Rebuildable for RawScrollableContainer<E> {
    #[inline]
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Claims the pointer the element being replaced was routing.
    ///
    /// This container dispatches to its child itself, so the capture a pressed
    /// child took lives in the dispatcher *beside* the children rather than in
    /// them — precisely the state reconciliation's positional walk cannot reach.
    /// A press that rebuilds the subtree while the pointer is still down — a
    /// button in the list darkening under the finger — would leave the
    /// replacement owning nothing, and
    /// [`child_route_allowed`](crate::scrollable::handle_scroll) gates child
    /// routing on exactly that capture: the release would be dismissed as
    /// landing outside the viewport and the captured child would be stranded
    /// mid-gesture, never hearing the pointer lift.
    ///
    /// The dispatcher is *moved* out of `old`, which reconciliation drops
    /// immediately afterwards. Two containers both believing they own the
    /// pointer would deliver every remaining event twice. It names the old
    /// subtree's identities, and those identities are transferred onto the new
    /// elements in the pass that follows this one, so the carried capture
    /// resolves once reconciliation completes.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old
            .option_any()
            .and_then(|value| value.downcast_ref::<Self>())
        else {
            return;
        };

        *self.event_dispatcher.borrow_mut() =
            std::mem::take(&mut *old.event_dispatcher.borrow_mut());
    }
}

impl<E: Element> RawScrollableContainer<E> {
    #[inline]
    fn layout_key(&self, ctx: &BuildContext) -> ScrollLayoutKey {
        ScrollLayoutKey {
            constraint: ctx.box_constraint,
            parent_size: ctx.parent_size,
            scale_bits: ctx.scale.to_bits(),
            axis: self.ctrl.axis,
            scroll_bar_placement: self.ctrl.scroll_bar_placement,
            viewport_w: self.viewport_w,
            viewport_h: self.viewport_h,
            vertical_bar_width: self.vertical_bar_width,
            horizontal_bar_height: self.horizontal_bar_height,
            tree_generation: element_tree_generation(),
            layout_generation: layout_invalidation_generation(),
        }
    }

    /// Resolves the scroll-axis extent against the active constraints.
    #[inline]
    fn constrained_extent(
        viewport: f32,
        bar_extent: f32,
        min: f32,
        max: f32,
        parent: f32,
    ) -> (f32, f32) {
        let total = if max.is_finite() && max < f32::MAX {
            max
        } else if parent.is_finite() && parent < f32::MAX {
            parent.max(min)
        } else {
            viewport + bar_extent
        }
        .clamp(min, max);
        let bar_extent = bar_extent.min(total).max(0.0);
        (total, (total - bar_extent).max(0.0))
    }

    /// Resolves the cross-axis extent — the axis this viewport never scrolls.
    ///
    /// A bounded maximum is filled, exactly like any other box handed a
    /// definite extent. An *unbounded* maximum means the surrounding layout is
    /// asking the viewport how much space it needs — a `Column` measuring its
    /// children inside a vertical scroll viewport does exactly that — so the
    /// honest answer is the child's own extent plus the bar reserved on this
    /// axis. Falling back to the parent's resolved size here is what used to
    /// stretch a horizontal code-block scroller to the full height of the
    /// outer viewport.
    ///
    /// `content` is only invoked on the unbounded path, so the common bounded
    /// case never measures the child.
    #[inline]
    fn cross_extent(
        content: impl FnOnce() -> f32,
        bar_extent: f32,
        min: f32,
        max: f32,
    ) -> (f32, f32) {
        let total = if max.is_finite() && max < f32::MAX {
            max
        } else {
            content() + bar_extent
        }
        .clamp(min, max);
        let bar_extent = bar_extent.min(total).max(0.0);
        (total, (total - bar_extent).max(0.0))
    }

    /// Measures the child's extent across the scroll axis.
    ///
    /// The scroll axis is left unbounded exactly as the child is measured
    /// everywhere else in this container, so the result comes from the same
    /// per-constraint layout cache the draw pass resolves and costs nothing on
    /// a settled frame.
    fn content_cross_extent(&self, ctx: &BuildContext) -> f32 {
        let mut child_ctx = ctx.clone();
        match self.ctrl.axis {
            crate::ScrollAxis::Vertical => child_ctx.box_constraint.max_height = f32::MAX,
            crate::ScrollAxis::Horizontal => child_ctx.box_constraint.max_width = f32::MAX,
        }
        let size = self.child.computed_size(&child_ctx);
        match self.ctrl.axis {
            crate::ScrollAxis::Vertical => size.width,
            crate::ScrollAxis::Horizontal => size.height,
        }
    }

    /// Resolves both extents this scrollable occupies under `ctx`.
    ///
    /// Returns `((width, inner_width), (height, inner_height))`, where the
    /// first value of each pair includes any inline bar reservation and the
    /// second is the content viewport that remains. Floating bars leave the
    /// viewport at the full available extent. The scroll axis fills the space
    /// it was given; the cross axis wraps the child when its constraint is
    /// unbounded — see [`RawScrollableContainer::cross_extent`].
    #[inline]
    fn resolved_extents(&self, ctx: &BuildContext) -> ((f32, f32), (f32, f32)) {
        let constraint = &ctx.box_constraint;
        let vertical_bar_extent = if self.ctrl.scroll_bar_placement.reserves_space() {
            self.vertical_bar_width
        } else {
            0.0
        };
        let horizontal_bar_extent = if self.ctrl.scroll_bar_placement.reserves_space() {
            self.horizontal_bar_height
        } else {
            0.0
        };
        match self.ctrl.axis {
            crate::ScrollAxis::Vertical => (
                Self::cross_extent(
                    || self.content_cross_extent(ctx),
                    vertical_bar_extent,
                    constraint.min_width,
                    constraint.max_width,
                ),
                Self::constrained_extent(
                    self.viewport_h,
                    0.0,
                    constraint.min_height,
                    constraint.max_height,
                    ctx.parent_size.height,
                ),
            ),
            crate::ScrollAxis::Horizontal => (
                Self::constrained_extent(
                    self.viewport_w,
                    0.0,
                    constraint.min_width,
                    constraint.max_width,
                    ctx.parent_size.width,
                ),
                Self::cross_extent(
                    || self.content_cross_extent(ctx),
                    horizontal_bar_extent,
                    constraint.min_height,
                    constraint.max_height,
                ),
            ),
        }
    }

    /// Resolves extents once for the current layout inputs.
    ///
    /// The live scroll offset is intentionally not part of the key: it changes
    /// only the canvas translation and scrollbar thumb position, never the
    /// child constraints or intrinsic content size.
    #[inline]
    fn cached_extents(&self, ctx: &BuildContext) -> ((f32, f32), (f32, f32)) {
        let key = self.layout_key(ctx);
        if let Some(snapshot) = self.layout_cache.snapshot.get()
            && snapshot.key == key
        {
            return snapshot.extents;
        }

        let extents = self.resolved_extents(ctx);
        self.layout_cache.snapshot.set(Some(CachedScrollLayout {
            key,
            extents,
            content_size: None,
        }));
        extents
    }

    /// Measures the child with the exact constraints used by the draw pass.
    #[inline]
    fn measure_content_size(
        &self,
        ctx: &BuildContext,
        viewport_w: f32,
        viewport_h: f32,
    ) -> ResolvedSize {
        let mut child_ctx = ctx.clone();
        child_ctx.box_constraint.min_width = child_ctx.box_constraint.min_width.min(viewport_w);
        child_ctx.box_constraint.min_height = child_ctx.box_constraint.min_height.min(viewport_h);
        child_ctx.box_constraint.max_width = viewport_w;
        child_ctx.box_constraint.max_height = viewport_h;
        child_ctx.parent_size = ResolvedSize {
            width: viewport_w,
            height: viewport_h,
        };
        match self.ctrl.axis {
            crate::ScrollAxis::Vertical => child_ctx.box_constraint.max_height = f32::MAX,
            crate::ScrollAxis::Horizontal => child_ctx.box_constraint.max_width = f32::MAX,
        }
        self.child.computed_size(&child_ctx)
    }

    /// Returns the content extent without remeasuring it after an offset-only
    /// scroll frame.
    #[inline]
    pub(crate) fn cached_content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let extents = self.cached_extents(ctx);
        let key = self.layout_key(ctx);
        if let Some(snapshot) = self.layout_cache.snapshot.get()
            && snapshot.key == key
            && let Some(content_size) = snapshot.content_size
        {
            return content_size;
        }

        let ((_, viewport_w), (_, viewport_h)) = extents;
        let content_size = self.measure_content_size(ctx, viewport_w, viewport_h);
        let snapshot = self
            .layout_cache
            .snapshot
            .get()
            .filter(|snapshot| snapshot.key == key)
            .unwrap_or(CachedScrollLayout {
                key,
                extents,
                content_size: None,
            });
        self.layout_cache.snapshot.set(Some(CachedScrollLayout {
            content_size: Some(content_size),
            ..snapshot
        }));
        content_size
    }

    /// Rechecks the child's extent after painting has had a chance to refine
    /// windowed layout. A predicted flex table can change its total while it
    /// materializes visible rows; keeping that prediction in this container's
    /// cache leaves the scroll range one frame behind the content.
    #[inline]
    pub(crate) fn refresh_content_size_after_draw(
        &self,
        ctx: &BuildContext,
        viewport_w: f32,
        viewport_h: f32,
        initial_content_size: ResolvedSize,
        provisional_extent: bool,
        preserve_content_end: bool,
    ) {
        let content_size = self.measure_content_size(ctx, viewport_w, viewport_h);
        if !provisional_extent && content_size == initial_content_size {
            return;
        }

        // What the frame just painted was drawn against this range. Only the
        // axis the container scrolls along can show a difference, so a
        // correction that leaves it as it was needs no frame to display it.
        let axis = self.ctrl.axis;
        let painted_range = self.ctrl.cached_content_size_valid.get().then(|| {
            (
                Self::along(axis, self.ctrl.cached_content_size.get()),
                Self::along(axis, self.ctrl.cached_max_scroll.get()),
            )
        });

        self.ctrl.cached_content_size.set(content_size);
        self.ctrl.cached_content_size_valid.set(true);

        let mut max_scroll = Vec2d {
            x: (content_size.width - viewport_w).max(0.0),
            y: (content_size.height - viewport_h).max(0.0),
        };
        let user_max = self.ctrl.scroll_behavior.max_scroll;
        if user_max.x != f32::MAX {
            max_scroll.x = max_scroll.x.max(user_max.x * ctx.scale);
        }
        if user_max.y != f32::MAX {
            max_scroll.y = max_scroll.y.max(user_max.y * ctx.scale);
        }
        self.ctrl.cached_max_scroll.set(max_scroll);

        if preserve_content_end {
            let mut offset = self.ctrl.scroll_offset.get();
            match self.ctrl.axis {
                crate::ScrollAxis::Vertical => offset.y = -max_scroll.y,
                crate::ScrollAxis::Horizontal => offset.x = -max_scroll.x,
            }
            self.ctrl.set_scroll_offset(offset);
        }

        let key = self.layout_key(ctx);
        if let Some(snapshot) = self.layout_cache.snapshot.get()
            && snapshot.key == key
        {
            self.layout_cache.snapshot.set(Some(CachedScrollLayout {
                content_size: Some(content_size),
                ..snapshot
            }));
        } else {
            self.layout_cache.snapshot.set(None);
        }

        let corrected_range = (
            Self::along(axis, content_size),
            Self::along(axis, max_scroll),
        );
        if preserve_content_end || painted_range != Some(corrected_range) {
            // The child may have changed the extent without owning a redraw source
            // (RawFlex requests one itself, but custom elements need this fallback).
            ctx.window.request_redraw();
        }
    }

    /// The component of `value` along the scroll axis.
    #[inline]
    fn along<T: AxisComponents>(axis: crate::ScrollAxis, value: T) -> f32 {
        match axis {
            crate::ScrollAxis::Vertical => value.vertical(),
            crate::ScrollAxis::Horizontal => value.horizontal(),
        }
    }

    /// Computes the total size this scrollable occupies under `ctx`.
    #[inline]
    pub(crate) fn layout_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let ((width, _), (height, _)) = self.cached_extents(ctx);
        ResolvedSize { width, height }
    }

    /// Computes the content viewport from the active layout constraints.
    pub(crate) fn viewport_size(&self, ctx: &BuildContext) -> (f32, f32) {
        let ((_, width), (_, height)) = self.cached_extents(ctx);
        (width, height)
    }
}

/// A two-dimensional value whose components can be read by axis.
trait AxisComponents: Copy {
    fn horizontal(self) -> f32;
    fn vertical(self) -> f32;
}

impl AxisComponents for ResolvedSize {
    #[inline]
    fn horizontal(self) -> f32 {
        self.width
    }

    #[inline]
    fn vertical(self) -> f32 {
        self.height
    }
}

impl AxisComponents for Vec2d {
    #[inline]
    fn horizontal(self) -> f32 {
        self.x
    }

    #[inline]
    fn vertical(self) -> f32 {
        self.y
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use aimer_events::element::{ElementEvent, ScrollDeltaKind, TouchPhase};
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::{
        AnyElement, CaptureRequest, Drawable, EventElement, EventResult, LayoutElement, PointerKey,
        VisitorElement,
    };

    use super::*;

    mod redraw_after_refresh;

    struct CapturingChild {
        events: Rc<Cell<usize>>,
    }

    impl VisitorElement for CapturingChild {
        fn debug_name(&self) -> &'static str {
            "CapturingChild"
        }
    }

    impl EventElement for CapturingChild {
        fn on_event(&self, event: &ElementEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            match event {
                ElementEvent::PointerDown(pointer) => EventResult::consumed()
                    .with_pointer_capture(PointerKey::new(pointer.source, pointer.id)),
                ElementEvent::PointerUp(pointer) => EventResult::consumed()
                    .with_pointer_release(PointerKey::new(pointer.source, pointer.id)),
                _ => EventResult::consumed(),
            }
        }
    }

    impl LayoutElement for CapturingChild {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
        }
    }

    impl Drawable for CapturingChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for CapturingChild {}

    /// A child that reports the same intrinsic size under every constraint,
    /// standing in for wrapped content such as a code block's text.
    struct FixedSizeChild {
        size: ResolvedSize,
    }

    impl VisitorElement for FixedSizeChild {
        fn debug_name(&self) -> &'static str {
            "FixedSizeChild"
        }
    }

    impl EventElement for FixedSizeChild {}

    impl LayoutElement for FixedSizeChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size
        }
    }

    impl Drawable for FixedSizeChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for FixedSizeChild {}

    struct ScrollBlockingWrapper {
        child: AnyElement,
    }

    impl VisitorElement for ScrollBlockingWrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn debug_name(&self) -> &'static str {
            "ScrollBlockingWrapper"
        }
    }

    impl EventElement for ScrollBlockingWrapper {
        fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(self.child.as_ref());
        }

        fn on_event(&self, event: &ElementEvent) -> EventResult {
            matches!(event, ElementEvent::Scroll { .. }).into()
        }
    }

    impl LayoutElement for ScrollBlockingWrapper {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::ZERO, Vec2d { x: 100.0, y: 100.0 }))
        }
    }

    impl Drawable for ScrollBlockingWrapper {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for ScrollBlockingWrapper {}

    struct CountingChild {
        size: ResolvedSize,
        measures: Rc<Cell<usize>>,
    }

    impl VisitorElement for CountingChild {
        fn debug_name(&self) -> &'static str {
            "CountingChild"
        }
    }

    impl EventElement for CountingChild {}

    impl LayoutElement for CountingChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.measures.set(self.measures.get() + 1);
            self.size
        }
    }

    impl Drawable for CountingChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for CountingChild {}

    struct DrawingChild {
        draws: Rc<Cell<usize>>,
        size: ResolvedSize,
    }

    impl VisitorElement for DrawingChild {
        fn debug_name(&self) -> &'static str {
            "DrawingChild"
        }
    }

    impl EventElement for DrawingChild {}

    impl LayoutElement for DrawingChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size
        }
    }

    impl Drawable for DrawingChild {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.set(self.draws.get() + 1);
        }
    }

    impl Rebuildable for DrawingChild {}

    /// A child whose layout discovers a larger extent while it is painted.
    /// Windowed flex content has the same shape: its estimated table is refined
    /// while drawing the visible rows.
    struct RefiningChild {
        size: Rc<Cell<ResolvedSize>>,
        refined_size: ResolvedSize,
    }

    impl VisitorElement for RefiningChild {
        fn debug_name(&self) -> &'static str {
            "RefiningChild"
        }
    }

    impl EventElement for RefiningChild {}

    impl LayoutElement for RefiningChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size.get()
        }
    }

    impl Drawable for RefiningChild {
        fn update(&self, _ctx: &BuildContext) {
            self.size.set(self.refined_size);
        }
    }

    impl Rebuildable for RefiningChild {}

    struct RefiningPaintChild {
        size: Rc<Cell<ResolvedSize>>,
        refined_size: ResolvedSize,
        painted_y: Rc<Cell<Option<f32>>>,
    }

    impl VisitorElement for RefiningPaintChild {
        fn debug_name(&self) -> &'static str {
            "RefiningPaintChild"
        }
    }

    impl EventElement for RefiningPaintChild {}

    impl LayoutElement for RefiningPaintChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size.get()
        }
    }

    impl Drawable for RefiningPaintChild {
        fn prepare_layout(&self, _ctx: &BuildContext) -> bool {
            self.size.set(self.refined_size);
            true
        }

        fn update(&self, ctx: &BuildContext) {
            self.painted_y
                .set(Some(ctx.canvas.get_transform_translation().1));
            self.size.set(self.refined_size);
        }
    }

    impl Rebuildable for RefiningPaintChild {}

    impl aimer_widget::PortableWidget for RefiningPaintChild {}

    impl aimer_widget::Widget for RefiningPaintChild {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            aimer_widget::Element::boxed(self)
        }
    }

    struct PrepareBeforeMeasureChild {
        prepared: Rc<Cell<bool>>,
        measured_before_prepare: Rc<Cell<usize>>,
        unprepared_size: ResolvedSize,
        prepared_size: ResolvedSize,
    }

    impl VisitorElement for PrepareBeforeMeasureChild {
        fn debug_name(&self) -> &'static str {
            "PrepareBeforeMeasureChild"
        }
    }

    impl EventElement for PrepareBeforeMeasureChild {}

    impl LayoutElement for PrepareBeforeMeasureChild {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            if !self.prepared.get() {
                self.measured_before_prepare
                    .set(self.measured_before_prepare.get() + 1);
                self.unprepared_size
            } else {
                self.prepared_size
            }
        }
    }

    impl Drawable for PrepareBeforeMeasureChild {
        fn prepare_layout(&self, _ctx: &BuildContext) -> bool {
            self.prepared.set(true);
            true
        }

        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for PrepareBeforeMeasureChild {}

    fn drawing_scrollable(draws: Rc<Cell<usize>>) -> RawScrollableContainer<AnyElement> {
        raw_scrollable(
            DrawingChild {
                draws,
                size: ResolvedSize {
                    width: 100.0,
                    height: 400.0,
                },
            }
            .boxed(),
        )
    }

    fn raw_scrollable(child: AnyElement) -> RawScrollableContainer<AnyElement> {
        let mut state = ScrollState::for_test_at(Vec2d::default());
        state.axis = crate::ScrollAxis::Vertical;

        RawScrollableContainer {
            child,
            ctrl: Rc::new(state),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    #[test]
    fn a_scrollable_that_moves_for_a_scroll_says_so_and_one_that_ignores_it_does_not() {
        use aimer_events::pointer::take_scroll_used;

        fn scroll(scrollable: &impl aimer_widget::EventElement, dx: f32, dy: f32) -> bool {
            let _ = take_scroll_used();
            let _ = scrollable.on_event(&ElementEvent::Scroll {
                delta: Vec2d { x: dx, y: dy },
                phase: TouchPhase::Moved,
                kind: ScrollDeltaKind::Pixel,
                is_direct_manipulation: true,
            });
            take_scroll_used()
        }
        let horizontal = |room: f32| {
            let scrollable = sized_scrollable(
                crate::ScrollAxis::Horizontal,
                ResolvedSize { width: 200.0, height: 100.0 },
            );
            scrollable.ctrl.cached_max_scroll.set(Vec2d { x: room, y: 0.0 });
            scrollable.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
            scrollable.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));
            scrollable
        };

        // Room to move sideways, and a sideways scroll: it moved, and says so.
        assert!(scroll(&horizontal(100.0), 40.0, 0.0));
        // The wrong direction for it: a vertical scroll is not a horizontal one's.
        assert!(!scroll(&horizontal(100.0), 0.0, 40.0));
    }

    #[test]
    fn nested_scrollables_give_a_horizontal_dominant_frame_to_the_inner_axis() {
        let inner = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
        );
        inner.ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        inner.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        inner.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));
        let inner_ctrl = inner.ctrl.clone();

        let outer = raw_scrollable(inner.boxed());
        outer.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });
        outer.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        outer.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let result = outer.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: 40.0, y: -20.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        });

        assert!(result.is_consumed());
        assert_eq!(inner_ctrl.scroll_offset.get().x, 40.0);
        assert_eq!(outer.ctrl.scroll_offset.get().y, 0.0);
    }

    #[test]
    fn scrollable_is_an_indexed_target_while_keeping_its_child_dispatch_private() {
        let child = FixedSizeChild {
            size: ResolvedSize {
                width: 100.0,
                height: 200.0,
            },
        }
        .boxed();
        let scrollable = raw_scrollable(child);

        assert_eq!(
            scrollable.event_tree_role(),
            aimer_widget::EventTreeRole::IndexedTarget
        );
        assert_eq!(scrollable.event_tree_bounds(), None);

        let mut event_children = 0;
        scrollable.event_children(&mut |_| event_children += 1);
        assert_eq!(event_children, 0);
    }

    #[test]
    fn nested_scrollables_ignore_cross_axis_drift_for_a_vertical_dominant_frame() {
        let inner = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
        );
        inner.ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        inner.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        inner.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));
        let inner_ctrl = inner.ctrl.clone();

        let outer = raw_scrollable(inner.boxed());
        outer.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });
        outer.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        outer.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let result = outer.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: 3.0, y: -40.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        });

        assert!(result.is_consumed());
        assert_eq!(inner_ctrl.scroll_offset.get().x, 0.0);
        assert_eq!(outer.ctrl.scroll_offset.get().y, -40.0);
    }

    #[test]
    fn nested_scrollable_hands_a_frame_to_the_parent_at_a_hard_edge() {
        let mut inner = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
        );
        Rc::get_mut(&mut inner.ctrl)
            .expect("the test state is not shared yet")
            .scroll_behavior
            .bouncy = false;
        inner.ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        inner.ctrl.set_scroll_offset(Vec2d { x: -100.0, y: 0.0 });
        inner.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        inner.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let outer = raw_scrollable(inner.boxed());
        outer.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });
        outer.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        outer.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let result = outer.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: -20.0, y: -40.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        });

        assert!(result.is_consumed());
        assert_eq!(outer.ctrl.scroll_offset.get().y, -40.0);
    }

    #[test]
    fn an_opaque_wrapper_does_not_hide_parent_scroll_when_nested_axis_cannot_move() {
        let inner = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
        );
        inner.ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        inner.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        inner.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let wrapper = ScrollBlockingWrapper {
            child: inner.boxed(),
        };
        let outer = raw_scrollable(wrapper.boxed());
        outer.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });
        outer.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        outer.ctrl.cursor_pos.set(Some(Vec2d { x: 50.0, y: 50.0 }));

        let result = outer.on_event(&ElementEvent::Scroll {
            delta: Vec2d { x: 0.0, y: -40.0 },
            phase: TouchPhase::Moved,
            kind: ScrollDeltaKind::Pixel,
            is_direct_manipulation: false,
        });

        assert!(result.is_consumed());
        assert_eq!(outer.ctrl.scroll_offset.get().y, -40.0);
    }

    #[test]
    fn nested_pointer_scroll_hands_off_to_the_parent_after_the_child_hits_an_edge() {
        let mut inner = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 200.0,
                height: 100.0,
            },
        );
        Rc::get_mut(&mut inner.ctrl)
            .expect("the test state is not shared yet")
            .scroll_behavior
            .bouncy = false;
        inner.ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        inner.ctrl.set_scroll_offset(Vec2d { x: -80.0, y: 0.0 });
        inner.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let inner_ctrl = inner.ctrl.clone();

        let outer = raw_scrollable(inner.boxed());
        outer.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });
        outer.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        let pointer = |x: f32, y: f32| PointerInfo::touch(Vec2d { x, y }, 3);
        let _ = outer.on_event(&ElementEvent::PointerDown(pointer(50.0, 50.0)));
        let _ = outer.on_event(&ElementEvent::PointerMove(pointer(30.0, 60.0)));
        let _ = outer.on_event(&ElementEvent::PointerMove(pointer(10.0, 70.0)));
        let _ = outer.on_event(&ElementEvent::PointerMove(pointer(10.0, 120.0)));
        let _ = outer.on_event(&ElementEvent::PointerMove(pointer(10.0, 140.0)));

        assert_eq!(inner_ctrl.scroll_offset.get().x, -100.0);
        assert_eq!(outer.ctrl.scroll_offset.get().y, 20.0);
        aimer_widget::release_pointer(PointerKey::new(PointerSource::Touch, 3));
    }

    /// An ancestor that arbitrates touches (a navigator that pops on a sideways
    /// swipe) has to be able to tell that a scroller is scrolling under the
    /// finger. The scroll view says so by claiming the pointer.
    #[test]
    fn a_content_drag_that_moves_the_content_claims_the_pointer() {
        let scrollable = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize { width: 300.0, height: 100.0 },
        );
        scrollable.ctrl.cached_max_scroll.set(Vec2d { x: 200.0, y: 0.0 });
        scrollable.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let finger = PointerKey::new(PointerSource::Touch, 3);
        let pointer = |x: f32| PointerInfo::touch(Vec2d { x, y: 50.0 }, 3);

        let _ = scrollable.on_event(&ElementEvent::PointerDown(pointer(80.0)));
        assert!(!aimer_widget::is_pointer_claimed(finger), "a press is not a scroll");
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(60.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(40.0)));

        assert_ne!(scrollable.ctrl.scroll_offset.get().x, 0.0, "the content moved");
        assert!(aimer_widget::is_pointer_claimed(finger), "so the scroller owns this touch");
        aimer_widget::release_pointer(finger);
    }

    #[test]
    fn a_scroller_that_cannot_move_the_way_the_finger_goes_does_not_claim_it() {
        // At its start, dragging right: nothing to reveal, and no rubber band.
        let mut scrollable = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize { width: 300.0, height: 100.0 },
        );
        Rc::get_mut(&mut scrollable.ctrl)
            .expect("the test state is not shared yet")
            .scroll_behavior
            .bouncy = false;
        scrollable.ctrl.cached_max_scroll.set(Vec2d { x: 200.0, y: 0.0 });
        scrollable.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let finger = PointerKey::new(PointerSource::Touch, 3);
        let pointer = |x: f32| PointerInfo::touch(Vec2d { x, y: 50.0 }, 3);

        let _ = scrollable.on_event(&ElementEvent::PointerDown(pointer(20.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(45.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(70.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(90.0)));

        assert_eq!(scrollable.ctrl.scroll_offset.get().x, 0.0);
        assert!(!aimer_widget::is_pointer_claimed(finger),
            "an ancestor may take a gesture this scroller cannot use");
        aimer_widget::release_pointer(finger);
    }

    #[test]
    fn content_drag_scrolls_with_touch_but_not_with_mouse() {
        let mouse = sized_scrollable(
            crate::ScrollAxis::Vertical,
            ResolvedSize {
                width: 100.0,
                height: 300.0,
            },
        );
        mouse.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 200.0 });
        mouse.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let start = Vec2d { x: 50.0, y: 50.0 };
        let _ = mouse.on_event(&ElementEvent::PointerDown(PointerInfo::mouse(
            start,
            PointerButton::Primary,
        )));
        let _ = mouse.on_event(&ElementEvent::PointerMove(PointerInfo::mouse(
            Vec2d { x: 50.0, y: 25.0 },
            PointerButton::Primary,
        )));
        let _ = mouse.on_event(&ElementEvent::PointerUp(PointerInfo::mouse(
            Vec2d { x: 50.0, y: 25.0 },
            PointerButton::Primary,
        )));

        assert_eq!(mouse.ctrl.scroll_offset.get().y, 0.0);
        assert_eq!(mouse.ctrl.drag_mode.get(), DragMode::None);

        let touch = sized_scrollable(
            crate::ScrollAxis::Vertical,
            ResolvedSize {
                width: 100.0,
                height: 300.0,
            },
        );
        touch.ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 200.0 });
        touch.bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let pointer = |y| PointerInfo::touch(Vec2d { x: 50.0, y }, 3);
        let _ = touch.on_event(&ElementEvent::PointerDown(pointer(50.0)));
        let _ = touch.on_event(&ElementEvent::PointerMove(pointer(25.0)));
        let _ = touch.on_event(&ElementEvent::PointerMove(pointer(15.0)));

        assert_ne!(touch.ctrl.scroll_offset.get().y, 0.0);
        let _ = touch.on_event(&ElementEvent::PointerUp(pointer(25.0)));
        aimer_widget::release_pointer(PointerKey::new(PointerSource::Touch, 3));
    }

    fn drawing_context(visible_rect: Option<(f32, f32, f32, f32)>) -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: 100.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 100.0,
        };
        context.visible_rect = visible_rect;
        context
    }

    #[tokio::test]
    async fn offscreen_scrollable_skips_child_content_before_dispatch() {
        let draws = Rc::new(Cell::new(0));
        let scrollable = drawing_scrollable(draws.clone());
        let mut ctx = drawing_context(Some((0.0, 101.0, 100.0, 20.0)));

        scrollable.update(&ctx);

        assert_eq!(draws.get(), 0);

        ctx.visible_rect = Some((0.0, 0.0, 100.0, 100.0));
        scrollable.update(&ctx);
        assert_eq!(draws.get(), 1);
    }

    /// A scrollable along `axis` holding a child of a fixed intrinsic size,
    /// used to observe how the container resolves its own extents.
    fn sized_scrollable(
        axis: crate::ScrollAxis,
        child: ResolvedSize,
    ) -> RawScrollableContainer<AnyElement> {
        let mut state = ScrollState::for_test_at(Vec2d::default());
        state.axis = axis;
        // These extent tests explicitly model the legacy reserved-space mode.
        state.scroll_bar_placement = crate::ScrollBarPlacement::Inline;

        RawScrollableContainer {
            child: FixedSizeChild { size: child }.boxed(),
            ctrl: Rc::new(state),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    fn counting_scrollable(measures: Rc<Cell<usize>>) -> RawScrollableContainer<AnyElement> {
        let mut state = ScrollState::for_test_at(Vec2d::default());
        state.axis = crate::ScrollAxis::Vertical;

        RawScrollableContainer {
            child: CountingChild {
                size: ResolvedSize {
                    width: 100.0,
                    height: 400.0,
                },
                measures,
            }
            .boxed(),
            ctrl: Rc::new(state),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    fn refining_scrollable(
        ctrl: Rc<ScrollState>,
        size: Rc<Cell<ResolvedSize>>,
        refined_size: ResolvedSize,
    ) -> RawScrollableContainer<AnyElement> {
        RawScrollableContainer {
            child: RefiningChild { size, refined_size }.boxed(),
            ctrl,
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    #[tokio::test]
    async fn changing_only_scroll_offset_reuses_content_layout() {
        let measures = Rc::new(Cell::new(0));
        let scrollable = counting_scrollable(measures.clone());
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 100.0,
                height: 100.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 100.0,
            max_height: 100.0,
        };

        assert_eq!(scrollable.content_size(&ctx).height, 400.0);
        assert_eq!(measures.get(), 1);

        scrollable
            .ctrl
            .scroll_offset
            .set(Vec2d { x: 0.0, y: -80.0 });

        assert_eq!(scrollable.content_size(&ctx).height, 400.0);
        assert_eq!(scrollable.computed_size(&ctx).height, 100.0);
        assert_eq!(
            measures.get(),
            1,
            "changing only the scroll offset must not remeasure content"
        );

        let mut changed_ctx = ctx.clone();
        changed_ctx.box_constraint.max_width = 80.0;
        assert_eq!(scrollable.content_size(&changed_ctx).width, 100.0);
        assert_eq!(
            measures.get(),
            2,
            "changing a layout constraint must retire the scroll snapshot"
        );
    }

    #[tokio::test]
    async fn drawing_refreshes_scroll_range_after_child_refines_content_extent() {
        let size = Rc::new(Cell::new(ResolvedSize {
            width: 100.0,
            height: 350.0,
        }));
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d { x: 0.0, y: -250.0 }));
        let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));

        refining_scrollable(
            ctrl.clone(),
            size.clone(),
            ResolvedSize {
                width: 100.0,
                height: 350.0,
            },
        )
        .update(&ctx);

        size.set(ResolvedSize {
            width: 100.0,
            height: 300.0,
        });
        refining_scrollable(
            ctrl.clone(),
            size.clone(),
            ResolvedSize {
                width: 100.0,
                height: 350.0,
            },
        )
        .update(&ctx);

        assert_eq!(size.get().height, 350.0);
        assert_eq!(ctrl.scroll_offset.get().y, -250.0);
        assert_eq!(ctrl.cached_content_size.get().height, 350.0);
        assert_eq!(ctrl.cached_max_scroll.get().y, 250.0);
    }

    #[tokio::test]
    async fn drawing_preserves_bottom_anchor_when_child_extent_grows() {
        let size = Rc::new(Cell::new(ResolvedSize {
            width: 100.0,
            height: 350.0,
        }));
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d { x: 0.0, y: -250.0 }));
        let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));

        refining_scrollable(
            ctrl.clone(),
            size.clone(),
            ResolvedSize {
                width: 100.0,
                height: 350.0,
            },
        )
        .update(&ctx);

        size.set(ResolvedSize {
            width: 100.0,
            height: 300.0,
        });
        refining_scrollable(
            ctrl.clone(),
            size,
            ResolvedSize {
                width: 100.0,
                height: 450.0,
            },
        )
        .update(&ctx);

        assert_eq!(ctrl.cached_content_size.get().height, 450.0);
        assert_eq!(ctrl.cached_max_scroll.get().y, 350.0);
        assert_eq!(
            ctrl.scroll_offset.get().y,
            -350.0,
            "a rebuild that grows content must keep a viewport parked at the content end"
        );
    }

    #[tokio::test]
    async fn drawing_uses_the_new_bottom_anchor_for_the_first_refined_paint() {
        let size = Rc::new(Cell::new(ResolvedSize {
            width: 100.0,
            height: 350.0,
        }));
        let painted_y = Rc::new(Cell::new(None));
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d { x: 0.0, y: -250.0 }));
        let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));

        RawScrollableContainer {
            child: RefiningPaintChild {
                size: size.clone(),
                refined_size: ResolvedSize {
                    width: 100.0,
                    height: 350.0,
                },
                painted_y: painted_y.clone(),
            }
            .boxed(),
            ctrl: ctrl.clone(),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
        .update(&ctx);

        size.set(ResolvedSize {
            width: 100.0,
            height: 300.0,
        });
        let rebuilt = RawScrollableContainer {
            child: RefiningPaintChild {
                size,
                refined_size: ResolvedSize {
                    width: 100.0,
                    height: 450.0,
                },
                painted_y: painted_y.clone(),
            }
            .boxed(),
            ctrl,
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        };

        rebuilt.update(&ctx);

        assert_eq!(painted_y.get(), Some(-350.0));
    }

    #[tokio::test]
    async fn prepares_a_viewport_dependent_child_before_measuring_its_extent() {
        let prepared = Rc::new(Cell::new(false));
        let measured_before_prepare = Rc::new(Cell::new(0));
        let mut state = ScrollState::for_test_at(Vec2d { x: 0.0, y: -250.0 });
        state.axis = crate::ScrollAxis::Vertical;
        state.cached_content_size.set(ResolvedSize {
            width: 100.0,
            height: 350.0,
        });
        state.cached_content_size_valid.set(true);
        state.cached_max_scroll.set(Vec2d { x: 0.0, y: 250.0 });
        state.last_scale.set(1.0);
        let ctrl = Rc::new(state);
        let scrollable = RawScrollableContainer {
            child: PrepareBeforeMeasureChild {
                prepared: prepared.clone(),
                measured_before_prepare: measured_before_prepare.clone(),
                unprepared_size: ResolvedSize {
                    width: 100.0,
                    height: 300.0,
                },
                prepared_size: ResolvedSize {
                    width: 100.0,
                    height: 450.0,
                },
            },
            ctrl: ctrl.clone(),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        };

        scrollable.update(&drawing_context(None));

        assert!(prepared.get());
        assert_eq!(
            measured_before_prepare.get(),
            0,
            "the scroll range must not be measured before viewport-dependent layout is prepared"
        );
        assert_eq!(ctrl.cached_content_size.get().height, 450.0);
        assert_eq!(ctrl.scroll_offset.get().y, -350.0);
    }

    #[tokio::test]
    async fn retained_child_uses_the_final_bottom_anchor_before_paint() {
        let size = Rc::new(Cell::new(ResolvedSize {
            width: 100.0,
            height: 350.0,
        }));
        let painted_y = Rc::new(Cell::new(None));
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d { x: 0.0, y: -250.0 }));
        let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));

        RawScrollableContainer {
            child: aimer_widget::ChildBuilder::from_widget(RefiningPaintChild {
                size: size.clone(),
                refined_size: ResolvedSize {
                    width: 100.0,
                    height: 350.0,
                },
                painted_y: painted_y.clone(),
            })
            .build(&ctx),
            ctrl: ctrl.clone(),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
        .update(&ctx);

        size.set(ResolvedSize {
            width: 100.0,
            height: 300.0,
        });
        let rebuilt = RawScrollableContainer {
            child: aimer_widget::ChildBuilder::from_widget(RefiningPaintChild {
                size,
                refined_size: ResolvedSize {
                    width: 100.0,
                    height: 450.0,
                },
                painted_y: painted_y.clone(),
            })
            .build(&ctx),
            ctrl,
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds: InteractionBounds::new(),
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        };

        rebuilt.update(&ctx);

        assert_eq!(painted_y.get(), Some(-350.0));
    }

    #[tokio::test]
    async fn refined_extent_clamps_before_paint_on_both_axes() {
        for axis in [crate::ScrollAxis::Vertical, crate::ScrollAxis::Horizontal] {
            for (start, final_extent, expected) in [
                (-250.0, 450.0, -350.0),
                (-250.0, 200.0, -100.0),
                (-180.0, 200.0, -100.0),
                (-80.0, 450.0, -80.0),
                (-250.0, 50.0, 0.0),
            ] {
                let vertical = axis == crate::ScrollAxis::Vertical;
                let extent = |main| ResolvedSize {
                    width: if vertical { 100.0 } else { main },
                    height: if vertical { main } else { 100.0 },
                };
                let mut state = ScrollState::for_test_at(Vec2d {
                    x: if vertical { 0.0 } else { start },
                    y: if vertical { start } else { 0.0 },
                });
                state.axis = axis;
                state.cached_content_size.set(extent(350.0));
                state.cached_content_size_valid.set(true);
                state.cached_max_scroll.set(Vec2d {
                    x: if vertical { 0.0 } else { 250.0 },
                    y: if vertical { 250.0 } else { 0.0 },
                });
                state.last_scale.set(1.0);
                let ctrl = Rc::new(state);
                let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));
                let painted_y = Rc::new(Cell::new(None));
                let scrollable = RawScrollableContainer {
                    child: RefiningPaintChild {
                        size: Rc::new(Cell::new(extent(300.0))),
                        refined_size: extent(final_extent),
                        painted_y: painted_y.clone(),
                    },
                    ctrl: ctrl.clone(),
                    vertical_scroll_bar: None,
                    horizontal_scroll_bar: None,
                    viewport_w: 100.0,
                    viewport_h: 100.0,
                    vertical_bar_width: 0.0,
                    horizontal_bar_height: 0.0,
                    bounds: InteractionBounds::new(),
                    event_dispatcher: RefCell::new(EventDispatcher::new()),
                    bar_dispatcher: Default::default(),
                    layout_cache: Default::default(),
                };
                scrollable.update(&ctx);
                let offset = ctrl.scroll_offset.get();
                assert_eq!(if vertical { offset.y } else { offset.x }, expected);
                if vertical {
                    assert_eq!(painted_y.get(), Some(expected));
                }
                assert_eq!(ctrl.cached_content_size.get(), extent(final_extent));
            }
        }
    }

    /// Regression test: a horizontal scrollable measured under an unbounded
    /// height — a `Column` inside a vertical scroll viewport does exactly that
    /// — must wrap its child's height instead of stretching to the parent's
    /// resolved size, which used to blow a code block up to the full height of
    /// the outer viewport.
    #[tokio::test]
    async fn a_horizontal_scrollable_wraps_its_height_when_the_cross_axis_is_unbounded() {
        let mut scrollable = sized_scrollable(
            crate::ScrollAxis::Horizontal,
            ResolvedSize {
                width: 300.0,
                height: 120.0,
            },
        );
        scrollable.horizontal_bar_height = 10.0;

        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 500.0,
                height: 600.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 500.0,
            max_height: f32::MAX,
        };

        assert_eq!(scrollable.viewport_size(&ctx), (500.0, 120.0));
        assert_eq!(
            scrollable.computed_size(&ctx),
            ResolvedSize {
                width: 500.0,
                height: 130.0,
            }
        );
    }

    #[tokio::test]
    async fn a_vertical_scrollable_wraps_its_width_when_the_cross_axis_is_unbounded() {
        let mut scrollable = sized_scrollable(
            crate::ScrollAxis::Vertical,
            ResolvedSize {
                width: 300.0,
                height: 120.0,
            },
        );
        scrollable.vertical_bar_width = 12.0;

        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 800.0,
                height: 400.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: f32::MAX,
            max_height: 400.0,
        };

        assert_eq!(scrollable.viewport_size(&ctx), (300.0, 400.0));
        assert_eq!(
            scrollable.computed_size(&ctx),
            ResolvedSize {
                width: 312.0,
                height: 400.0,
            }
        );
    }

    /// A container laid out over the top-left 100x100 corner, wrapping a child
    /// that captures the pointer it is pressed with. Both elements of a rebuild
    /// share `ctrl`, exactly as the live scroll engine is shared across one.
    fn capturing_scrollable(
        events: Rc<Cell<usize>>,
        ctrl: Rc<ScrollState>,
    ) -> RawScrollableContainer<AnyElement> {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);

        RawScrollableContainer {
            child: CapturingChild { events }.boxed(),
            ctrl,
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds,
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    #[tokio::test]
    async fn computed_size_fills_the_parent_constraint_over_the_stored_viewport() {
        let scrollable = capturing_scrollable(
            Rc::new(Cell::new(0)),
            Rc::new(ScrollState::for_test_at(Vec2d::default())),
        );
        let mut scrollable = scrollable;
        scrollable.vertical_bar_width = 12.0;
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 200.0,
                height: 160.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 40.0,
            min_height: 30.0,
            max_width: 200.0,
            max_height: 160.0,
        };

        assert_eq!(
            scrollable.computed_size(&ctx),
            ResolvedSize {
                width: 200.0,
                height: 160.0,
            }
        );
    }

    #[tokio::test]
    async fn a_flex_assigned_constraint_shrinks_the_scrollable_viewport() {
        let mut ctrl = ScrollState::for_test_at(Vec2d::default());
        ctrl.scroll_bar_placement = crate::ScrollBarPlacement::Inline;
        let mut scrollable = capturing_scrollable(Rc::new(Cell::new(0)), Rc::new(ctrl));
        scrollable.viewport_w = 800.0;
        scrollable.viewport_h = 600.0;
        scrollable.vertical_bar_width = 12.0;

        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 800.0,
                height: 600.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 320.0,
            max_height: 180.0,
        };

        assert_eq!(scrollable.viewport_size(&ctx), (308.0, 180.0));
        assert_eq!(
            scrollable.computed_size(&ctx),
            ResolvedSize {
                width: 320.0,
                height: 180.0,
            }
        );
    }

    #[tokio::test]
    async fn a_retained_scrollable_expands_when_the_parent_constraint_grows() {
        let mut ctrl = ScrollState::for_test_at(Vec2d::default());
        ctrl.scroll_bar_placement = crate::ScrollBarPlacement::Inline;
        let mut scrollable = capturing_scrollable(Rc::new(Cell::new(0)), Rc::new(ctrl));
        scrollable.viewport_w = 320.0;
        scrollable.viewport_h = 180.0;
        scrollable.vertical_bar_width = 12.0;

        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 320.0,
                height: 180.0,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 320.0,
            max_height: 180.0,
        };

        assert_eq!(scrollable.viewport_size(&ctx), (308.0, 180.0));

        ctx.parent_size = ResolvedSize {
            width: 640.0,
            height: 360.0,
        };
        ctx.box_constraint.max_width = 640.0;
        ctx.box_constraint.max_height = 360.0;

        assert_eq!(scrollable.viewport_size(&ctx), (628.0, 360.0));
        assert_eq!(
            scrollable.computed_size(&ctx),
            ResolvedSize {
                width: 640.0,
                height: 360.0,
            }
        );
    }

    // A rebuild triggered by the press itself — a `Button` inside the list
    // darkening under the finger — replaces this container. The capture the child
    // took lives in the dispatcher beside the children rather than in them, so
    // the positional walk cannot reach it: without the hand-over,
    // `child_route_allowed` sees no capture, the replacement rejects the release
    // as being outside its viewport, and the child never hears the pointer lift.
    #[test]
    fn a_rebuild_during_a_press_keeps_the_capture_so_a_release_outside_still_lands() {
        let events = Rc::new(Cell::new(0));
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::default()));
        let pressed = capturing_scrollable(events.clone(), ctrl.clone());
        let pointer = PointerKey::new(PointerSource::Touch, 2);

        let down = pressed.on_event(&ElementEvent::PointerDown(PointerInfo::touch(
            Vec2d { x: 10.0, y: 10.0 },
            pointer.id,
        )));
        assert_eq!(down.capture_request(), CaptureRequest::Capture(pointer));

        let rebuilt = capturing_scrollable(events.clone(), ctrl);
        // Standing in for the identity transfer reconciliation performs around
        // the hand-over, which is what makes the carried capture resolve against
        // the new subtree.
        rebuilt.child.set_element_id(pressed.child.id());
        rebuilt.adopt_runtime_state_from(&pressed as &dyn Element);

        let up = rebuilt.on_event(&ElementEvent::PointerUp(PointerInfo::touch(
            Vec2d { x: 200.0, y: 200.0 },
            pointer.id,
        )));

        assert_eq!(
            events.get(),
            2,
            "the child must hear the release it is owed"
        );
        assert_eq!(up.capture_request(), CaptureRequest::Release(pointer));
    }

    /// A child that is lit while the pointer is over it.
    struct HoverChild {
        hovered: Rc<Cell<bool>>,
    }

    impl VisitorElement for HoverChild {
        fn debug_name(&self) -> &'static str {
            "HoverChild"
        }
    }

    impl EventElement for HoverChild {
        fn on_event(&self, event: &ElementEvent) -> EventResult {
            match event {
                ElementEvent::PointerMove(_) => self.hovered.set(true),
                ElementEvent::PointerExited(..) => self.hovered.set(false),
                _ => {}
            }
            EventResult::ignored()
        }
    }

    impl LayoutElement for HoverChild {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((Vec2d::default(), Vec2d { x: 100.0, y: 100.0 }))
        }
    }

    impl Drawable for HoverChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for HoverChild {}

    /// A panel can draw a grab band over the scrollable's edge, so the pointer
    /// keeps being reported to the scrollable after it has left the viewport.
    /// The scrollable stops offering those moves to its content, and has to say
    /// that the pointer is gone, or the content it was hovering stays lit.
    #[test]
    fn a_move_outside_the_viewport_releases_the_hover_inside_it() {
        let hovered = Rc::new(Cell::new(false));
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        let scrollable = RawScrollableContainer {
            child: HoverChild {
                hovered: hovered.clone(),
            }
            .boxed(),
            ctrl: Rc::new(ScrollState::for_test_at(Vec2d::default())),
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds,
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        };
        let pointer = |x: f32, y: f32| PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary);

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(50.0, 50.0)));
        assert!(hovered.get(), "the pointer over the content must hover it");

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(104.0, 50.0)));
        assert!(
            !hovered.get(),
            "the content stayed hovered after the pointer left the viewport"
        );

        // Further moves outside have nothing left to release.
        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(110.0, 50.0)));
        assert!(!hovered.get());
    }

    /// A container whose vertical bar is the real, `MouseRegion`-wrapped one,
    /// laid out the way `update` lays it out: a 10 px strip at the right edge.
    fn scrollable_with_bar(ctrl: Rc<ScrollState>) -> RawScrollableContainer<AnyElement> {
        scrollable_with_bar_at(ctrl, Vec2d::ZERO)
    }

    /// The same, with the viewport's top-left at `origin` in the window.
    fn scrollable_with_bar_at(
        ctrl: Rc<ScrollState>,
        origin: Vec2d,
    ) -> RawScrollableContainer<AnyElement> {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, origin.x, origin.y, 100.0, 100.0);
        use crate::ScrollAxis;
        use aimer_widget::Widget as _;
        let ctx = drawing_context(None);
        let bar = crate::scroll_bar::ScrollBar::default()
            .for_scrollable(ctrl.clone(), ScrollAxis::Vertical)
            .to_element(&ctx);

        let mut bar_ctx = drawing_context(None);
        bar_ctx.box_constraint.max_width = 10.0;
        bar_ctx.parent_size = ResolvedSize {
            width: 10.0,
            height: 100.0,
        };
        bar_ctx.canvas.save();
        bar_ctx.canvas.translate(Vec2d {
            x: origin.x + 90.0,
            y: origin.y,
        });
        bar.update(&bar_ctx);
        bar_ctx.canvas.restore();

        RawScrollableContainer {
            child: HoverChild {
                hovered: Rc::new(Cell::new(false)),
            }
            .boxed(),
            ctrl,
            vertical_scroll_bar: Some(bar),
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 10.0,
            horizontal_bar_height: 0.0,
            bounds,
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    /// Regression: the bar appeared whenever the mouse entered the viewport.
    /// Only the bar's own strip may show it, as its `MouseRegion` decides; a
    /// mouse moving over the content must leave it alone.
    #[tokio::test]
    async fn a_mouse_over_the_content_does_not_show_the_scroll_bar() {
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::default()));
        let scrollable = scrollable_with_bar(ctrl.clone());
        let pointer = |x: f32, y: f32| PointerInfo::mouse(Vec2d { x, y }, PointerButton::Primary);

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(40.0, 80.0)));
        assert!(
            !ctrl.scroll_bar_hovered(),
            "the cursor is over the content, not the bar"
        );

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(95.0, 30.0)));
        assert!(ctrl.scroll_bar_hovered(), "the cursor is on the bar strip");

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(40.0, 80.0)));
        assert!(
            !ctrl.scroll_bar_hovered(),
            "moving back onto the content releases the bar"
        );

        let _ = scrollable.on_event(&ElementEvent::PointerMove(pointer(95.0, 30.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerExited(PointerSource::Mouse, 0));
        assert!(
            !ctrl.scroll_bar_hovered(),
            "a mouse that has left is no longer holding the bar"
        );
    }

    /// A container without bars, around content that ignores events.
    fn scrollable_with_thumb(ctrl: Rc<ScrollState>) -> RawScrollableContainer<AnyElement> {
        let bounds = InteractionBounds::new();
        bounds.save(1.0, 0.0, 0.0, 100.0, 100.0);
        RawScrollableContainer {
            child: HoverChild {
                hovered: Rc::new(Cell::new(false)),
            }
            .boxed(),
            ctrl,
            vertical_scroll_bar: None,
            horizontal_scroll_bar: None,
            viewport_w: 100.0,
            viewport_h: 100.0,
            vertical_bar_width: 0.0,
            horizontal_bar_height: 0.0,
            bounds,
            event_dispatcher: RefCell::new(EventDispatcher::new()),
            bar_dispatcher: Default::default(),
            layout_cache: Default::default(),
        }
    }

    /// A scrollable that is not at the window's corner: a 100 x 100 viewport
    /// whose top-left sits at (50, 30), with a thumb at the right edge of the
    /// viewport (published relative to the viewport, as the bar paints it).
    fn offset_scrollable_with_thumb() -> (RawScrollableContainer<AnyElement>, Rc<ScrollState>) {
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::default()));
        ctrl.cached_viewport.set((100.0, 100.0));
        ctrl.cached_v_track_width.set(10.0);
        ctrl.v_thumb_rect.set(Some((90.0, 10.0, 10.0, 40.0)));
        let scrollable = scrollable_with_thumb(ctrl.clone());
        scrollable.bounds.save(1.0, 50.0, 30.0, 100.0, 100.0);
        (scrollable, ctrl)
    }

    /// The same press, but on the content beside the thumb in window terms: the
    /// window position that would have matched the viewport-relative rectangle
    /// is not the thumb.
    #[test]
    fn a_press_beside_the_thumb_does_not_grab_it() {
        let (scrollable, ctrl) = offset_scrollable_with_thumb();
        let press = PointerInfo::mouse(Vec2d { x: 95.0, y: 20.0 }, PointerButton::Primary);

        let _ = scrollable.on_event(&ElementEvent::PointerDown(press));

        assert_ne!(ctrl.drag_mode.get(), DragMode::VerticalScrollbar);
    }

    const BAR_ORIGIN: Vec2d = Vec2d { x: 50.0, y: 30.0 };

    /// A scrollable offset from the window corner with a real bar over content
    /// four times its height (so a 4x thumb multiplier), and the thumb rectangle
    /// the bar published when it was updated.
    fn scrollable_with_published_thumb() -> (
        RawScrollableContainer<AnyElement>,
        Rc<ScrollState>,
        (f32, f32, f32, f32),
    ) {
        let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::default()));
        ctrl.cached_viewport.set((100.0, 100.0));
        ctrl.cached_content_size.set(ResolvedSize {
            width: 100.0,
            height: 400.0,
        });
        ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 300.0 });
        let scrollable = scrollable_with_bar_at(ctrl.clone(), BAR_ORIGIN);
        let thumb = ctrl.v_thumb_rect.get().expect("the bar publishes its thumb");
        (scrollable, ctrl, thumb)
    }

    fn mouse_at(x: f32, y: f32) -> PointerInfo {
        PointerInfo::mouse(
            Vec2d {
                x: BAR_ORIGIN.x + x,
                y: BAR_ORIGIN.y + y,
            },
            PointerButton::Primary,
        )
    }

    /// Dragging the thumb moves the content in proportion, whatever the
    /// viewport's place in the window, and letting go ends the drag.
    #[tokio::test]
    async fn dragging_the_thumb_scrolls_the_content() {
        let (scrollable, ctrl, (x, y, w, h)) = scrollable_with_published_thumb();
        let (grab_x, grab_y) = (x + w / 2.0, y + h / 2.0);
        let multiplier = ctrl.v_scroll_multiplier.get();
        assert!(multiplier > 0.0);

        let _ = scrollable.on_event(&ElementEvent::PointerDown(mouse_at(grab_x, grab_y)));
        for step in 1..=4 {
            let _ = scrollable.on_event(&ElementEvent::PointerMove(mouse_at(
                grab_x,
                grab_y + 5.0 * step as f32,
            )));
        }
        let dragged = ctrl.scroll_offset.get().y;
        assert!(
            (dragged + 20.0 * multiplier).abs() < 0.01,
            "20 px of thumb is 20 x {multiplier} px of content, got {dragged}"
        );
        assert_eq!(ctrl.drag_mode.get(), DragMode::VerticalScrollbar);

        let _ = scrollable.on_event(&ElementEvent::PointerUp(mouse_at(grab_x, grab_y + 20.0)));
        assert_eq!(ctrl.drag_mode.get(), DragMode::None);
        assert!((ctrl.scroll_offset.get().y - dragged).abs() < 0.01, "release does not move it");
    }

    /// A drag that leaves the viewport keeps following the pointer.
    #[tokio::test]
    async fn a_thumb_drag_keeps_following_the_pointer_outside_the_viewport() {
        let (scrollable, ctrl, (x, y, w, h)) = scrollable_with_published_thumb();
        let (grab_x, grab_y) = (x + w / 2.0, y + h / 2.0);
        let multiplier = ctrl.v_scroll_multiplier.get();

        let _ = scrollable.on_event(&ElementEvent::PointerDown(mouse_at(grab_x, grab_y)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(mouse_at(grab_x, grab_y + 10.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(mouse_at(grab_x + 80.0, grab_y + 30.0)));

        let dragged = ctrl.scroll_offset.get().y;
        assert!(
            (dragged + 30.0 * multiplier).abs() < 0.01,
            "only the distance along the bar counts, got {dragged}"
        );
    }

    /// Clicking the track scrolls to that spot: the thumb centres on the click.
    /// The content is pushed there with momentum rather than set down in a frame.
    #[tokio::test]
    async fn clicking_the_track_glides_to_the_click() {
        let (scrollable, ctrl, (x, y, w, h)) = scrollable_with_published_thumb();
        let multiplier = ctrl.v_scroll_multiplier.get();
        let click_y = y + h + 30.0;

        let _ = scrollable.on_event(&ElementEvent::PointerDown(mouse_at(x + w / 2.0, click_y)));
        let _ = scrollable.on_event(&ElementEvent::PointerUp(mouse_at(x + w / 2.0, click_y)));

        let distance = -(click_y - h / 2.0 - y) * multiplier;
        assert_eq!(ctrl.scroll_offset.get().y, 0.0, "it has not jumped");
        assert!(ctrl.momentum_friction.get().is_some());
        let launch = ctrl.pointer_velocity.get().y;
        let expected = distance * (1.0 - crate::scrollable::constants::KEY_GLIDE_FRICTION);
        assert!((launch - expected).abs() < 1e-3, "launch {launch}, expected {expected}");
        assert_eq!(ctrl.drag_mode.get(), DragMode::None);
    }

    /// Pressing the track and dragging carries the thumb from the click.
    #[tokio::test]
    async fn dragging_from_the_track_carries_the_thumb_from_the_click() {
        let (scrollable, ctrl, (x, y, w, h)) = scrollable_with_published_thumb();
        let multiplier = ctrl.v_scroll_multiplier.get();
        let press_y = y + h + 30.0;

        let _ = scrollable.on_event(&ElementEvent::PointerDown(mouse_at(x + w / 2.0, press_y)));
        // The first move past the slop starts the drag, the next one drives it.
        let _ = scrollable.on_event(&ElementEvent::PointerMove(mouse_at(x + w / 2.0, press_y + 6.0)));
        let _ = scrollable.on_event(&ElementEvent::PointerMove(mouse_at(x + w / 2.0, press_y + 10.0)));

        let expected = -(press_y - h / 2.0 - y + 10.0) * multiplier;
        let got = ctrl.scroll_offset.get().y;
        assert!((got - expected).abs() < 0.01, "expected {expected}, got {got}");
    }


    fn key_press(key: aimer_events::element::NamedKey) -> ElementEvent {
        ElementEvent::KeyInput {
            key,
            action: aimer_events::element::KeyAction::Pressed,
            modifiers: Default::default(),
        }
    }

    /// A key press pushes the content: it does not move on the press itself, it
    /// leaves with momentum and glides on, like a fling.
    #[tokio::test]
    async fn a_key_press_launches_a_glide() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));

        let _ = scrollable.on_event(&key_press(aimer_events::element::NamedKey::ArrowDown));

        assert_eq!(ctrl.scroll_offset.get().y, 0.0, "it has not jumped");
        assert!(ctrl.fling_start_time.get().is_none(), "it is momentum, not a timed slide");
        assert!(ctrl.momentum_friction.get().is_some());
        let launch = ctrl.pointer_velocity.get().y;
        let expected = -crate::scrollable::constants::KEYBOARD_SCROLL_STEP
            * (1.0 - crate::scrollable::constants::KEY_GLIDE_FRICTION);
        assert!((launch - expected).abs() < 1e-4, "launch {launch}, expected {expected}");
    }

    /// Holding a key repeats it; each repeat adds to the speed already built up
    /// instead of restarting.
    #[tokio::test]
    async fn repeated_key_presses_build_on_the_glide() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));
        let down = key_press(aimer_events::element::NamedKey::ArrowDown);

        let _ = scrollable.on_event(&down);
        let one = ctrl.pointer_velocity.get().y;
        let _ = scrollable.on_event(&down);
        let _ = scrollable.on_event(&down);

        assert!((ctrl.pointer_velocity.get().y - 3.0 * one).abs() < 1e-4);
    }

    /// Holding a key makes the platform repeat it. Each repeat is a press as far
    /// as scrolling goes; ignoring them left only the first step of a held key.
    #[tokio::test]
    async fn a_held_key_keeps_scrolling_through_its_repeats() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));
        let key = aimer_events::element::NamedKey::ArrowDown;
        let repeat = ElementEvent::KeyInput {
            key: key.clone(),
            action: aimer_events::element::KeyAction::Repeat,
            modifiers: Default::default(),
        };

        let _ = scrollable.on_event(&key_press(key));
        let one = ctrl.pointer_velocity.get().y;
        let result = scrollable.on_event(&repeat);
        let _ = scrollable.on_event(&repeat);

        assert!(result.is_consumed(), "a repeat is the scrollable's, like the press");
        assert!(
            (ctrl.pointer_velocity.get().y - 3.0 * one).abs() < 1e-4,
            "press and two repeats add up"
        );
    }

    /// Letting go is not a step.
    #[tokio::test]
    async fn releasing_a_key_does_not_scroll() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));
        let release = ElementEvent::KeyInput {
            key: aimer_events::element::NamedKey::ArrowDown,
            action: aimer_events::element::KeyAction::Released,
            modifiers: Default::default(),
        };

        let _ = scrollable.on_event(&release);

        assert_eq!(ctrl.pointer_velocity.get().y, 0.0);
    }

    /// The push is sized to carry the content one step, then it keeps
    /// decelerating to a stop on its own. Driven by the real momentum update.
    #[tokio::test]
    async fn the_glide_carries_the_content_one_step_and_settles() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));
        let _ = scrollable.on_event(&key_press(aimer_events::element::NamedKey::ArrowDown));

        let mut offset = ctrl.scroll_offset.get();
        for _ in 0..400 {
            tokio::time::sleep(std::time::Duration::from_millis(8)).await;
            let (next, redraw) = ctrl.update_momentum(offset);
            offset = next;
            if !redraw {
                break;
            }
        }

        let step = crate::scrollable::constants::KEYBOARD_SCROLL_STEP;
        assert!(
            (offset.y + step).abs() < step * 0.12,
            "one step of {step}, got {}",
            offset.y
        );
        assert!(ctrl.momentum_friction.get().is_none(), "normal momentum is restored");
        assert_eq!(ctrl.pointer_velocity.get().y, 0.0);
    }

    /// Any other input takes over from the glide, including its friction.
    #[tokio::test]
    async fn a_new_input_ends_the_glide_friction() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));
        let _ = scrollable.on_event(&key_press(aimer_events::element::NamedKey::ArrowDown));

        ctrl.cancel_fling();

        assert!(ctrl.momentum_friction.get().is_none());
    }

    /// The destination is the edge, not past it.
    #[tokio::test]
    async fn a_key_scroll_stops_at_the_end_of_the_content() {
        let (scrollable, ctrl, _) = scrollable_with_published_thumb();
        ctrl.cursor_pos.set(Some(Vec2d { x: 60.0, y: 40.0 }));

        let _ = scrollable.on_event(&key_press(aimer_events::element::NamedKey::End));

        assert_eq!(ctrl.fling_target_offset.get().y, -300.0);
    }
}
