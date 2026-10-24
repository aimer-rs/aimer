use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_container::{Container, ZeroSizedBox};
use aimer_events::element::{ElementEvent, KeyAction, NamedKey};
use aimer_macro::Rebuildable;
use aimer_widget::SafeAreaInsets;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, LayoutElement,
    RequiredChild, VisitorElement, Widget,
};

use crate::anchor::AnchorHandle;
use crate::animated_content::wrap_animated_content;
use crate::host::{self, ModalHandle, ModalId, ModalTimeline};
use crate::paint::{contains, overlay_child_context};
use crate::placement::{FloatingAlign, FloatingSide, OverflowPolicy, PlacementSpec};
use crate::{ModalAnimation, resolve_placement};

/// Displays content pinned to an [`crate::Anchor`] above the entire
/// application render tree.
///
/// `Floating` is the anchored placement primitive of the framework: it reuses
/// the same overlay entry stack as [`crate::Modal`] — and therefore the same
/// stacking order, pointer routing, `Escape` handling and enter/exit
/// animation — but resolves its position from the anchor rectangle instead of
/// the viewport. Dropdown menus, tooltips, selects and context menus are meant
/// to be composed from it rather than to reimplement it.
///
/// The panel is re-positioned on every frame, so it keeps following its anchor
/// while the anchor moves, resizes or scrolls. When the resolved position would
/// leave the viewport, the configured [`OverflowPolicy`] flips the panel to the
/// opposite side or slides it back inside.
///
/// Unlike a modal, the barrier defaults to [`Color::Transparent`]: the content
/// below stays visible while pointer presses outside the panel still dismiss
/// it.
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_modal::{Anchor, AnchorHandle, Floating, FloatingAlign, FloatingSide};
///
/// let anchor = AnchorHandle::new();
/// let _trigger = Anchor::new().handle(anchor.clone())
///                             .child(SizedBox::new().width(120).height(32));
///
/// let handle = Floating::new().anchor(anchor)
///                             .side(FloatingSide::Bottom)
///                             .align(FloatingAlign::Start)
///                             .gap(4.0)
///                             .child(SizedBox::new().width(180).height(240))
///                             .show();
///
/// handle.dismiss();
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_modal::Floating", schema_only)]
pub struct Floating<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_skip]
    anchor: AnchorHandle,
    #[portable_skip]
    placement: PlacementSpec,
    #[portable_skip]
    barrier_color: Color,
    #[portable_skip]
    animation: Option<ModalAnimation>,
    #[portable_skip]
    barrier_dismissible: bool,
    #[portable_skip]
    escape_dismissible: bool,
    #[portable_skip]
    viewport_margin: f32,
    #[portable_skip]
    respect_safe_area: bool,
}

impl Default for Floating {
    fn default() -> Self {
        Self::new()
    }
}

impl Floating {
    /// Creates a panel placed below an untracked anchor, with a transparent
    /// barrier that dismisses on an outside press or `Escape`.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            anchor: AnchorHandle::new(),
            placement: PlacementSpec::new(),
            barrier_color: Color::Transparent,
            animation: None,
            barrier_dismissible: true,
            escape_dismissible: true,
            viewport_margin: 0.0,
            respect_safe_area: true,
        }
    }

    /// Pins the panel to the rectangle reported by `anchor`.
    #[inline]
    pub fn anchor(mut self, anchor: AnchorHandle) -> Self {
        self.anchor = anchor;
        self
    }

    /// Replaces the whole placement request at once.
    #[inline]
    pub fn placement(mut self, placement: PlacementSpec) -> Self {
        self.placement = placement;
        self
    }

    /// Sets the preferred side of the anchor.
    #[inline]
    pub fn side(mut self, side: FloatingSide) -> Self {
        self.placement = self.placement.side(side);
        self
    }

    /// Sets the cross-axis alignment against the anchor.
    #[inline]
    pub fn align(mut self, align: FloatingAlign) -> Self {
        self.placement = self.placement.align(align);
        self
    }

    /// Sets the distance kept between the anchor and the panel.
    #[inline]
    pub fn gap(mut self, gap: f32) -> Self {
        self.placement = self.placement.gap(gap);
        self
    }

    /// Sets an additional translation applied to the resolved position.
    #[inline]
    pub fn offset(mut self, offset: Vec2d) -> Self {
        self.placement = self.placement.offset(offset);
        self
    }

    /// Sets what happens when the panel does not fit inside the viewport.
    #[inline]
    pub fn overflow(mut self, overflow: OverflowPolicy) -> Self {
        self.placement = self.placement.overflow(overflow);
        self
    }

    /// Sets the viewport-wide barrier color.
    #[inline]
    pub fn barrier_color(mut self, barrier_color: Color) -> Self {
        self.barrier_color = barrier_color;
        self
    }

    /// Enables a paint-only enter and exit transition.
    #[inline]
    pub fn animation(mut self, animation: ModalAnimation) -> Self {
        self.animation = Some(animation);
        self
    }

    /// Controls whether pressing outside the panel dismisses it.
    #[inline]
    pub fn barrier_dismissible(mut self, dismissible: bool) -> Self {
        self.barrier_dismissible = dismissible;
        self
    }

    /// Controls whether a pressed Escape key dismisses the panel.
    #[inline]
    pub fn escape_dismissible(mut self, dismissible: bool) -> Self {
        self.escape_dismissible = dismissible;
        self
    }

    /// Sets a margin the panel keeps from every edge of the viewport.
    ///
    /// It is folded into the region the system already reserves, so it only
    /// applies where the system asks for less: a panel is held off the bare
    /// window edge without being pushed twice away from a status bar.
    ///
    /// In logical pixels, like [`Floating::gap`].
    #[inline]
    pub fn viewport_margin(mut self, margin: f32) -> Self {
        self.viewport_margin = if margin.is_finite() && margin > 0.0 {
            margin
        } else {
            0.0
        };
        self
    }

    /// Controls whether the panel stays out of the region the system draws
    /// over.
    ///
    /// On by default, and it is what keeps a panel out of the status bar, the
    /// notch and the home indicator — where a press never reaches the
    /// application at all. Turn it off only for content that is *meant* to run
    /// under them, such as a full-bleed backdrop.
    #[inline]
    pub fn respect_safe_area(mut self, respect: bool) -> Self {
        self.respect_safe_area = respect;
        self
    }

    /// Attaches the required panel content and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Floating<W> {
        Floating {
            child,
            anchor: self.anchor,
            placement: self.placement,
            barrier_color: self.barrier_color,
            animation: self.animation,
            barrier_dismissible: self.barrier_dismissible,
            escape_dismissible: self.escape_dismissible,
            viewport_margin: self.viewport_margin,
            respect_safe_area: self.respect_safe_area,
        }
    }

    /// Attaches and erases the required panel content.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Floating<W> {
    /// Presents this panel through the application-wide host immediately.
    ///
    /// Calls made before the first application frame are queued and presented
    /// as soon as the root host is built.
    pub fn show(self) -> ModalHandle {
        let animation = self.animation.clone();
        host::show(
            animation,
            Box::new(move |ctx, id, timeline| self.to_raw_element(ctx, Some(id), timeline)),
        )
    }

    fn to_raw_element(
        self,
        ctx: &BuildContext,
        id: Option<ModalId>,
        timeline: Rc<RefCell<ModalTimeline>>,
    ) -> AnyElement {
        let barrier_animation = (self.barrier_color.to_rgba().3 != 0)
            .then(|| self.animation.clone())
            .flatten();
        let barrier = Container::new()
            .color(self.barrier_color)
            .child(ZeroSizedBox)
            .to_element(ctx);
        let animate_barrier = barrier_animation.is_some();
        let barrier = wrap_animated_content(barrier, barrier_animation, timeline.clone(), false);
        let child = wrap_animated_content(
            self.child.to_element(ctx),
            self.animation.clone(),
            timeline.clone(),
            true,
        );
        RawFloating {
            barrier,
            animate_barrier,
            timeline,
            last_barrier_progress: Cell::new(f32::NAN),
            child,
            anchor: self.anchor.clone(),
            placement: self.placement,
            id,
            barrier_dismissible: self.barrier_dismissible,
            escape_dismissible: self.escape_dismissible,
            viewport_margin: self.viewport_margin,
            respect_safe_area: self.respect_safe_area,
            child_bounds: RefCell::new(None),
        }
        .boxed()
    }

    /// Returns the handle this panel is pinned to.
    #[inline]
    pub fn anchor_value(&self) -> AnchorHandle {
        self.anchor.clone()
    }

    /// Returns the placement request resolved on every frame.
    #[inline]
    pub fn placement_value(&self) -> PlacementSpec {
        self.placement
    }

    /// Returns the barrier color painted below the panel.
    #[inline]
    pub fn barrier_color_value(&self) -> Color {
        self.barrier_color
    }

    #[cfg(test)]
    pub(crate) fn animation_config(&self) -> Option<ModalAnimation> {
        self.animation.clone()
    }
}

impl<W: Widget + 'static> Widget for Floating<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.to_raw_element(
            ctx,
            None,
            Rc::new(RefCell::new(ModalTimeline::new_static())),
        )
    }

    fn debug_name(&self) -> &'static str {
        "Floating"
    }
}

#[derive(Rebuildable)]
struct RawFloating {
    barrier: AnyElement,
    animate_barrier: bool,
    timeline: Rc<RefCell<ModalTimeline>>,
    last_barrier_progress: Cell<f32>,
    child: AnyElement,
    anchor: AnchorHandle,
    placement: PlacementSpec,
    id: Option<ModalId>,
    barrier_dismissible: bool,
    escape_dismissible: bool,
    viewport_margin: f32,
    respect_safe_area: bool,
    child_bounds: RefCell<Option<(Vec2d, Vec2d)>>,
}

impl RawFloating {
    /// Returns the anchor rectangle in the painted coordinate space.
    ///
    /// Anchors record logical units, while the paint pass works in scaled
    /// units. An anchor that has never been painted collapses to the viewport
    /// origin, which places the panel in the top-left corner instead of an
    /// arbitrary position.
    fn anchor_bounds(&self, ctx: &BuildContext) -> Bounds {
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        self.anchor
            .bounds()
            .map(|bounds| bounds * scale)
            .unwrap_or_else(|| Bounds::new(ctx.parent_pos.x, ctx.parent_pos.y, 0.0, 0.0))
    }

    /// Returns this frame's placement request, in the painted coordinate space.
    ///
    /// A spec is written in logical pixels — a gap of `8.0` is eight pixels on
    /// every screen — while the anchor, the panel and the viewport handed to
    /// the resolver are all scaled, so the distances are converted here rather
    /// than left to shrink on a dense display.
    ///
    /// The region the system reserves is read per frame rather than captured at
    /// build time, because a rotation moves it without rebuilding anything.
    fn frame_placement(&self, ctx: &BuildContext) -> PlacementSpec {
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        let system = if self.respect_safe_area {
            aimer_widget::safe_area_insets()
        } else {
            SafeAreaInsets::ZERO
        };
        let reserved = reserved_edges(
            system,
            self.viewport_margin,
            self.placement.safe_area_value(),
        )
        .scaled(scale);
        let offset = self.placement.offset_value();
        self.placement
            .gap(self.placement.gap_value() * scale)
            .offset(Vec2d {
                x: offset.x * scale,
                y: offset.y * scale,
            })
            .safe_area(reserved)
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> Option<(ResolvedSize, Vec2d, BuildContext<'a>)> {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return None;
        }

        let child_size = self.child.computed_size(ctx);
        if !child_size.width.is_finite()
            || !child_size.height.is_finite()
            || child_size.width < 0.0
            || child_size.height < 0.0
        {
            return None;
        }
        let placement = resolve_placement(
            self.frame_placement(ctx),
            self.anchor_bounds(ctx),
            child_size,
            ctx.parent_size,
        );
        let offset = Vec2d {
            x: placement.origin.x - ctx.parent_pos.x,
            y: placement.origin.y - ctx.parent_pos.y,
        };
        if !offset.x.is_finite() || !offset.y.is_finite() {
            return None;
        }

        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = child_size;
        child_ctx.parent_pos = Vec2d {
            x: ctx.parent_pos.x + offset.x,
            y: ctx.parent_pos.y + offset.y,
        };
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: child_size.width,
            max_height: child_size.height,
        };
        child_ctx.visible_rect = ctx
            .visible_rect
            .map(|(x, y, width, height)| (x - offset.x, y - offset.y, width, height));
        Some((child_size, offset, child_ctx))
    }
}

/// Merges the three reservations a panel has to respect — the system's, the
/// panel's own margin and whatever its spec already asked for — by keeping the
/// largest of each edge.
///
/// Adding them would push a panel twice away from a status bar that already
/// leaves room for the margin.
fn reserved_edges(
    system: SafeAreaInsets,
    margin: f32,
    requested: SafeAreaInsets,
) -> SafeAreaInsets {
    system.max(SafeAreaInsets::all(margin)).max(requested)
}

impl Drawable for RawFloating {
    fn update(&self, ctx: &BuildContext) {
        self.barrier.update(ctx);
        let child_size = self.child.computed_size(ctx);
        let placement = resolve_placement(
            self.frame_placement(ctx),
            self.anchor_bounds(ctx),
            child_size,
            ctx.parent_size,
        );
        let offset = Vec2d {
            x: placement.origin.x - ctx.parent_pos.x,
            y: placement.origin.y - ctx.parent_pos.y,
        };
        let child_ctx = overlay_child_context(ctx, child_size, offset, &self.child_bounds);
        ctx.canvas.save();
        ctx.canvas.translate(offset);
        self.child.update(&child_ctx);
        ctx.canvas.restore();
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        // The barrier and hosted content have their own retained nodes. Keep
        // the barrier's full-window background out of compatibility painting
        // when a child such as a menu only changes a local row list.
        let barrier_animation_needs_sync = if self.animate_barrier {
            let (progress, active) = {
                let timeline = self.timeline.borrow();
                (timeline.progress(), timeline.is_active())
            };
            let changed = self.last_barrier_progress.replace(progress) != progress;
            active || changed
        } else {
            false
        };
        if barrier_animation_needs_sync {
            self.barrier.update(ctx);
        }
        if let Some((_, offset, child_ctx)) = self.retained_child_layout(ctx) {
            ctx.canvas.save();
            ctx.canvas.translate(offset);
            self.child.update(&child_ctx);
            ctx.canvas.restore();
        } else {
            self.update(ctx);
        }
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        if child_index != 1 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        self.retained_child_layout(ctx)
            .map(|(_, _, child_context)| child_context)
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
        if child_index != 1 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        let (size, offset, _) = self.retained_child_layout(ctx)?;
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                offset.x / ctx.scale,
                offset.y / ctx.scale,
                size.width / ctx.scale,
                size.height / ctx.scale,
            ),
            None,
        ))
    }
}

impl EventElement for RawFloating {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        let dismiss = match event {
            ElementEvent::PointerDown(pointer)
                if self.barrier_dismissible && !contains(&self.child_bounds, pointer.pos) =>
            {
                true
            }
            ElementEvent::KeyInput {
                key: NamedKey::Escape,
                action: KeyAction::Pressed,
                ..
            } if self.escape_dismissible => true,
            _ => false,
        };
        if dismiss && let Some(id) = self.id {
            host::dismiss(id);
        }
        EventResult::consumed()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.barrier.as_ref());
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawFloating {
    fn size(&self) -> Option<Size> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }
}

impl VisitorElement for RawFloating {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.barrier.as_ref());
        visitor(self.child.as_ref());
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        visitor(0, self.barrier.as_ref());
        visitor(1, self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Floating"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_animation::curves;
    use aimer_container::ZeroSizedBox;

    use super::*;
    use crate::anchor::Anchor;

    #[test]
    fn a_new_panel_hangs_below_its_anchor_behind_a_transparent_barrier() {
        let floating = Floating::new().child(ZeroSizedBox);

        assert_eq!(floating.barrier_color_value(), Color::Transparent);
        assert_eq!(
            floating.placement_value().side_value(),
            FloatingSide::Bottom
        );
        assert_eq!(
            floating.placement_value().align_value(),
            FloatingAlign::Start
        );
        assert_eq!(
            floating.placement_value().overflow_value(),
            OverflowPolicy::Flip
        );
    }

    #[test]
    fn configuration_survives_child_attachment() {
        let anchor = Anchor::new().child(ZeroSizedBox).handle_value();
        let animation = ModalAnimation::new()
            .enter_duration(Duration::from_millis(120))
            .enter_curve(curves::EASE_OUT);
        let floating = Floating::new()
            .anchor(anchor.clone())
            .side(FloatingSide::Right)
            .align(FloatingAlign::Center)
            .gap(6.0)
            .offset(Vec2d { x: 2.0, y: -3.0 })
            .overflow(OverflowPolicy::Shift)
            .barrier_color(Color::BLACK.with_opacity(40))
            .animation(animation.clone())
            .child(ZeroSizedBox);

        assert_eq!(floating.anchor_value(), anchor);
        assert_eq!(floating.placement_value().side_value(), FloatingSide::Right);
        assert_eq!(
            floating.placement_value().align_value(),
            FloatingAlign::Center
        );
        assert_eq!(floating.placement_value().gap_value(), 6.0);
        assert_eq!(
            floating.placement_value().offset_value(),
            Vec2d { x: 2.0, y: -3.0 }
        );
        assert_eq!(
            floating.placement_value().overflow_value(),
            OverflowPolicy::Shift
        );
        assert_eq!(
            floating.barrier_color_value(),
            Color::BLACK.with_opacity(40)
        );
        assert_eq!(floating.animation_config(), Some(animation));
    }

    #[test]
    fn a_margin_only_applies_where_the_system_reserves_less() {
        let reserved = reserved_edges(
            SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0),
            8.0,
            SafeAreaInsets::ZERO,
        );

        assert_eq!(reserved.top, 59.0);
        assert_eq!(reserved.bottom, 34.0);
        assert_eq!(reserved.left, 8.0);
        assert_eq!(reserved.right, 8.0);
    }

    #[test]
    fn a_spec_that_reserves_more_than_the_system_keeps_its_own_edge() {
        let reserved = reserved_edges(
            SafeAreaInsets::new(0.0, 20.0, 0.0, 0.0),
            0.0,
            SafeAreaInsets::new(0.0, 100.0, 0.0, 0.0),
        );

        assert_eq!(reserved.top, 100.0);
    }

    #[test]
    fn a_panel_that_ignores_the_safe_area_still_keeps_its_margin() {
        let panel = Floating::new()
            .respect_safe_area(false)
            .viewport_margin(12.0)
            .child(ZeroSizedBox);

        assert!(!panel.respect_safe_area);
        assert_eq!(
            reserved_edges(
                SafeAreaInsets::ZERO,
                panel.viewport_margin,
                SafeAreaInsets::ZERO
            )
            .top,
            12.0
        );
    }

    #[test]
    fn a_nonsense_margin_reserves_nothing() {
        let panel = Floating::new()
            .viewport_margin(f32::NAN)
            .child(ZeroSizedBox);

        assert_eq!(panel.viewport_margin, 0.0);
    }

    #[test]
    fn a_whole_placement_spec_can_replace_the_individual_setters() {
        let spec = PlacementSpec::new()
            .side(FloatingSide::Top)
            .overflow(OverflowPolicy::Fixed);
        let floating = Floating::new().placement(spec).child(ZeroSizedBox);

        assert_eq!(floating.placement_value(), spec);
    }

    #[test]
    fn show_and_dismiss_enqueue_framework_commands_immediately() {
        host::reset_registry_for_test();

        let handle = Floating::new().child(ZeroSizedBox).show();
        assert_eq!(host::pending_command_count_for_test(), 1);

        assert!(handle.dismiss());
        assert!(!handle.dismiss());
        assert_eq!(host::pending_command_count_for_test(), 2);
    }
}
