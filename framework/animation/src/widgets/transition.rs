use std::cell::Cell;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, CompositorAnimationDecision, CompositorAnimationFrame, CompositorTransform,
    Drawable, Element, EventElement, EventResult, LayoutElement, Rebuildable, VisitorElement,
    Widget,
};

use crate::control::controller::AnimationController;
use crate::primitives::time::AnimInstant;

fn request_next_frame() {
    aimer_events::window::request_animation_frame();
}

/// Updates the child for one sampled frame and keeps the animation running.
///
/// The retained render tree applies the sampled transform and opacity to the
/// child's node, so nothing is drawn or transformed here.
#[inline]
fn update_transition_frame(
    ctx: &BuildContext,
    child: &dyn Element,
    frame: CompositorAnimationFrame,
) {
    child.update(ctx);
    if frame.active {
        request_next_frame();
    }
}

// ---------------------------------------------------------------------------
// FadeTransition
// ---------------------------------------------------------------------------

/// Animates the opacity of its child based on the controller's value.
///
/// At value `0.0` the child is fully transparent; at `1.0` it is fully opaque.
/// Values are clamped to that range for drawing. Layout and event behavior are
/// delegated to the child.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_animation::FadeTransition", schema_only, manual_lowering)]
pub struct FadeTransition<T: Widget + 'static> {
    #[portable_skip]
    pub opacity: AnimationController,
    #[portable_child]
    pub child: T,
}

impl<T: Widget> FadeTransition<T> {
    /// Creates an opacity transition without starting or resetting `opacity`.
    pub fn new(opacity: AnimationController, child: T) -> Self {
        Self { opacity, child }
    }
}

impl<T: Widget + 'static> Widget for FadeTransition<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        let controller = self.opacity.clone();
        let animating = Cell::new(self.opacity.is_animating());
        FadeTransitionElement {
            child,
            controller,
            animating,
        }
        .boxed()
    }
}

macro_rules! impl_transition_element {
    ($name:ident, $debug:expr, $sample:expr) => {
        struct $name {
            child: AnyElement,
            controller: AnimationController,
            animating: Cell<bool>,
        }

        unsafe impl Send for $name {}
        unsafe impl Sync for $name {}

        impl Drawable for $name {
            fn update(&self, ctx: &BuildContext) {
                update_transition_frame(ctx, self.child.as_ref(), self.sample_frame(ctx));
            }

            #[inline]
            fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
                true
            }

            #[inline]
            fn paint_local_v2(&self, _ctx: &BuildContext) {}

            #[inline]
            fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
                self.child.update(ctx);
            }

            #[inline]
            fn sync_paint_geometry(&self, ctx: &BuildContext) {
                self.child.sync_paint_geometry(ctx);
            }

            #[inline]
            fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
                let frame = self.sample_frame(ctx);
                if frame.valid {
                    CompositorAnimationDecision::Compositor(frame)
                } else {
                    CompositorAnimationDecision::Live(frame)
                }
            }
        }

        impl $name {
            #[inline]
            fn sample_frame(&self, ctx: &BuildContext) -> CompositorAnimationFrame {
                let progress = self.controller.tick(AnimInstant::now());
                let active = self.controller.is_animating();
                self.animating.set(active);
                ($sample)(ctx, progress, active)
            }
        }

        impl VisitorElement for $name {
            fn debug_name(&self) -> &'static str {
                $debug
            }
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }
        }

        impl EventElement for $name {
            fn on_event(&self, event: &ElementEvent) -> EventResult {
                self.child.on_event(event)
            }
            fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }
        }

        impl Rebuildable for $name {
            fn rebuild_if_dirty(&self, ctx: &BuildContext) {
                self.child.rebuild_if_dirty(ctx);
            }
        }

        impl LayoutElement for $name {
            fn pos(&self) -> Option<Vec2d> {
                self.child.pos()
            }
            fn size(&self) -> Option<Size> {
                self.child.size()
            }
            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                self.child.computed_size(ctx)
            }
            fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
                self.child.content_size(ctx)
            }
            fn get_size_from_child(&self) -> Option<Size> {
                self.child.get_size_from_child()
            }
            /// Where the child is. A transition changes how its child is painted,
            /// not where it is laid out, and whoever measures the page it is the
            /// root of (a navigator's swipe zone, say) must find the child's bounds
            /// and not nothing.
            fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
                self.child.pos_start_end()
            }
            fn invalidate_layout(&self) {
                self.child.invalidate_layout();
            }
            #[inline]
            fn is_layout_stable(&self) -> bool {
                self.child.is_layout_stable()
            }
        }
    };
}

impl_transition_element!(
    FadeTransitionElement,
    "FadeTransitionElement",
    |_ctx: &BuildContext, v: f32, active: bool| {
        CompositorAnimationFrame::new(
            v,
            CompositorTransform::Identity,
            Some(v.clamp(0.0, 1.0)),
            None,
            active,
            v.is_finite(),
        )
    }
);

// ---------------------------------------------------------------------------
// SlideTransition
// ---------------------------------------------------------------------------

/// Animates a slide offset for its child.
///
/// The child is translated by `offset * (1.0 - controller_value)` pixels.
/// At value `0.0` the child is at the offset position; at `1.0` it is at its
/// natural position.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_animation::SlideTransition", schema_only, manual_lowering)]
pub struct SlideTransition<T: Widget + 'static> {
    #[portable_skip]
    pub position: AnimationController,
    /// The offset direction in pixels at value 0.0. At value 1.0 the child is
    /// at (0,0).
    #[portable_skip]
    pub offset: (f32, f32),
    #[portable_child]
    pub child: T,
}

impl<T: Widget> SlideTransition<T> {
    /// Creates a slide transition from the pixel `offset` to the child's
    /// natural position, without starting or resetting `position`.
    pub fn new(position: AnimationController, offset: (f32, f32), child: T) -> Self {
        Self {
            position,
            offset,
            child,
        }
    }
}

impl<T: Widget + 'static> Widget for SlideTransition<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        let controller = self.position.clone();
        let animating = Cell::new(self.position.is_animating());
        let offset = self.offset;
        SlideTransitionElement {
            child,
            controller,
            animating,
            offset,
        }
        .boxed()
    }
}

struct SlideTransitionElement {
    child: AnyElement,
    controller: AnimationController,
    animating: Cell<bool>,
    offset: (f32, f32),
}

unsafe impl Send for SlideTransitionElement {}
unsafe impl Sync for SlideTransitionElement {}

impl Drawable for SlideTransitionElement {
    fn update(&self, ctx: &BuildContext) {
        update_transition_frame(ctx, self.child.as_ref(), self.sample_frame(ctx));
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = self.sample_frame(ctx);
        if frame.valid {
            CompositorAnimationDecision::Compositor(frame)
        } else {
            CompositorAnimationDecision::Live(frame)
        }
    }
}

impl SlideTransitionElement {
    #[inline]
    fn sample_frame(&self, _ctx: &BuildContext) -> CompositorAnimationFrame {
        let progress = self.controller.tick(AnimInstant::now());
        let active = self.controller.is_animating();
        self.animating.set(active);

        let remaining = 1.0 - progress;
        let x = self.offset.0 * remaining;
        let y = self.offset.1 * remaining;
        let valid = progress.is_finite()
            && self.offset.0.is_finite()
            && self.offset.1.is_finite()
            && x.is_finite()
            && y.is_finite();
        CompositorAnimationFrame::new(
            progress,
            CompositorTransform::Translate { x, y },
            None,
            None,
            active,
            valid,
        )
    }
}

impl VisitorElement for SlideTransitionElement {
    fn debug_name(&self) -> &'static str {
        "SlideTransitionElement"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl EventElement for SlideTransitionElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.child.on_event(event)
    }
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for SlideTransitionElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for SlideTransitionElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }
    fn size(&self) -> Option<Size> {
        self.child.size()
    }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }
    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
    /// Where the child is laid out; the transition only changes how it is painted.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }
}

// ---------------------------------------------------------------------------
// ScaleTransition
// ---------------------------------------------------------------------------

/// Animates uniform scale for its child based on the controller's value.
///
/// A value of `1.0` is the child's natural size. The drawing transform is
/// centered in the current box constraints; layout itself is unchanged.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_animation::ScaleTransition", schema_only, manual_lowering)]
pub struct ScaleTransition<T: Widget + 'static> {
    #[portable_skip]
    pub scale: AnimationController,
    #[portable_child]
    pub child: T,
}

impl<T: Widget> ScaleTransition<T> {
    /// Creates a centered scale transition without starting or resetting
    /// `scale`.
    pub fn new(scale: AnimationController, child: T) -> Self {
        Self { scale, child }
    }
}

impl<T: Widget + 'static> Widget for ScaleTransition<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        let controller = self.scale.clone();
        let animating = Cell::new(self.scale.is_animating());
        ScaleTransitionElement {
            child,
            controller,
            animating,
        }
        .boxed()
    }
}

struct ScaleTransitionElement {
    child: AnyElement,
    controller: AnimationController,
    animating: Cell<bool>,
}

unsafe impl Send for ScaleTransitionElement {}
unsafe impl Sync for ScaleTransitionElement {}

impl Drawable for ScaleTransitionElement {
    fn update(&self, ctx: &BuildContext) {
        update_transition_frame(ctx, self.child.as_ref(), self.sample_frame(ctx));
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = self.sample_frame(ctx);
        if frame.valid {
            CompositorAnimationDecision::Compositor(frame)
        } else {
            CompositorAnimationDecision::Live(frame)
        }
    }
}

impl ScaleTransitionElement {
    #[inline]
    fn sample_frame(&self, ctx: &BuildContext) -> CompositorAnimationFrame {
        let progress = self.controller.tick(AnimInstant::now());
        let active = self.controller.is_animating();
        self.animating.set(active);

        let cx = ctx.box_constraint.max_width / 2.0;
        let cy = ctx.box_constraint.max_height / 2.0;
        let valid = progress.is_finite() && cx.is_finite() && cy.is_finite();
        CompositorAnimationFrame::new(
            progress,
            CompositorTransform::Scale {
                sx: progress,
                sy: progress,
                origin_x: cx,
                origin_y: cy,
            },
            None,
            None,
            active,
            valid,
        )
    }
}

impl VisitorElement for ScaleTransitionElement {
    fn debug_name(&self) -> &'static str {
        "ScaleTransitionElement"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl EventElement for ScaleTransitionElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.child.on_event(event)
    }
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for ScaleTransitionElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for ScaleTransitionElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }
    fn size(&self) -> Option<Size> {
        self.child.size()
    }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }
    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
    /// Where the child is laid out; the transition only changes how it is painted.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }
}

// ---------------------------------------------------------------------------
// RotationTransition
// ---------------------------------------------------------------------------

/// Animates rotation for its child based on the controller's value.
///
/// By default, a controller value of 0.0 means 0 turns and a value of 1.0
/// means one full turn (2π radians). Use [`Self::turn_range`] when the
/// transition should cover a different range, such as a disclosure chevron's
/// 0.0 to -0.25 turns.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_animation::RotationTransition",
    schema_only,
    manual_lowering
)]
pub struct RotationTransition<T: Widget + 'static> {
    #[portable_skip]
    pub turns: AnimationController,
    #[portable_skip]
    begin_turns: f32,
    #[portable_skip]
    end_turns: f32,
    #[portable_child]
    pub child: T,
}

impl<T: Widget> RotationTransition<T> {
    /// Creates a centered rotation transition without starting or resetting
    /// `turns`.
    pub fn new(turns: AnimationController, child: T) -> Self {
        Self {
            turns,
            begin_turns: 0.0,
            end_turns: 1.0,
            child,
        }
    }

    /// Sets the rotation range, in full turns, represented by controller
    /// values 0.0 and 1.0.
    ///
    /// Negative values rotate counterclockwise. For example,
    /// `turn_range(0.0, -0.25)` rotates a child 90° counterclockwise as the
    /// controller advances.
    #[inline]
    pub fn turn_range(mut self, begin_turns: f32, end_turns: f32) -> Self {
        self.begin_turns = begin_turns;
        self.end_turns = end_turns;
        self
    }
}

impl<T: Widget + 'static> Widget for RotationTransition<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        let controller = self.turns.clone();
        let animating = Cell::new(self.turns.is_animating());
        RotationTransitionElement {
            child,
            controller,
            begin_turns: self.begin_turns,
            end_turns: self.end_turns,
            animating,
        }
        .boxed()
    }
}

struct RotationTransitionElement {
    child: AnyElement,
    controller: AnimationController,
    begin_turns: f32,
    end_turns: f32,
    animating: Cell<bool>,
}

unsafe impl Send for RotationTransitionElement {}
unsafe impl Sync for RotationTransitionElement {}

impl Drawable for RotationTransitionElement {
    fn update(&self, ctx: &BuildContext) {
        update_transition_frame(ctx, self.child.as_ref(), self.sample_frame(ctx));
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = self.sample_frame(ctx);
        if frame.valid {
            CompositorAnimationDecision::Compositor(frame)
        } else {
            CompositorAnimationDecision::Live(frame)
        }
    }
}

impl RotationTransitionElement {
    #[inline]
    fn sample_frame(&self, ctx: &BuildContext) -> CompositorAnimationFrame {
        let progress = self.controller.tick(AnimInstant::now());
        let active = self.controller.is_animating();
        self.animating.set(active);

        self.frame_at(ctx, progress, active)
    }

    #[inline]
    fn frame_at(
        &self,
        ctx: &BuildContext,
        progress: f32,
        active: bool,
    ) -> CompositorAnimationFrame {
        let turns = interpolate_turns(self.begin_turns, self.end_turns, progress);
        let angle = turns * std::f32::consts::TAU;
        let cx = ctx.box_constraint.max_width / 2.0;
        let cy = ctx.box_constraint.max_height / 2.0;
        let valid = progress.is_finite()
            && self.begin_turns.is_finite()
            && self.end_turns.is_finite()
            && angle.is_finite()
            && cx.is_finite()
            && cy.is_finite();
        CompositorAnimationFrame::new(
            progress,
            CompositorTransform::Rotate {
                radians: angle,
                origin_x: cx,
                origin_y: cy,
            },
            None,
            None,
            active,
            valid,
        )
    }
}

#[inline]
fn interpolate_turns(begin_turns: f32, end_turns: f32, progress: f32) -> f32 {
    begin_turns + (end_turns - begin_turns) * progress
}

impl VisitorElement for RotationTransitionElement {
    fn debug_name(&self) -> &'static str {
        "RotationTransitionElement"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl EventElement for RotationTransitionElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.child.on_event(event)
    }
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for RotationTransitionElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for RotationTransitionElement {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }
    fn size(&self) -> Option<Size> {
        self.child.size()
    }
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }
    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
    /// Where the child is laid out; the transition only changes how it is painted.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }
}

macro_rules! impl_portable_transition_lowering {
    ($transition:ident) => {
        impl<T: Widget + 'static> aimer_widget::PortableWidget for $transition<T> {
            #[cfg(feature = "portable-guest")]
            fn to_portable_node(
                self,
                ctx: &mut aimer_widget::portable::PortableBuildContext,
                source: aimer_widget::portable::SourceFingerprint,
            ) -> Result<
                aimer_widget::portable::PortableNodeId,
                aimer_widget::portable::PortableBuildError,
            > {
                aimer_widget::PortableWidget::to_portable_node(self.child, ctx, source.child(0))
            }
        }
    };
}

impl_portable_transition_lowering!(FadeTransition);
impl_portable_transition_lowering!(SlideTransition);
impl_portable_transition_lowering!(ScaleTransition);
impl_portable_transition_lowering!(RotationTransition);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::curve::curves;
    use crate::widgets::test_frame_requester;
    use aimer_attribute::BoxConstraint;

    struct TestWidget;

    struct TestElement;

    impl Drawable for TestElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for TestElement {}

    impl LayoutElement for TestElement {}

    impl Rebuildable for TestElement {}

    impl VisitorElement for TestElement {
        fn debug_name(&self) -> &'static str {
            "TestElement"
        }
    }

    impl Widget for TestWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            TestElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for TestWidget {}

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn dummy_build_context() -> BuildContext<'static> {
        let canvas = {
            let leaked: &'static aimer_canvas::InnerCanvas =
                Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(leaked)
        };
        let mut context = BuildContext::new(
            canvas,
            ResolvedSize {
                width: 64.0,
                height: 64.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 64.0,
            max_height: 64.0,
        };
        context
    }

    fn controller() -> AnimationController {
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.forward_from_first_tick();
        controller
    }

    /// A child that has been laid out: it knows where it is.
    const BOUNDS: (Vec2d, Vec2d) = (Vec2d { x: 10.0, y: 20.0 }, Vec2d { x: 110.0, y: 220.0 });

    struct PlacedWidget;

    struct PlacedElement;

    impl Drawable for PlacedElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl EventElement for PlacedElement {}

    impl LayoutElement for PlacedElement {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some(BOUNDS)
        }
    }

    impl Rebuildable for PlacedElement {}

    impl VisitorElement for PlacedElement {
        fn debug_name(&self) -> &'static str {
            "PlacedElement"
        }
    }

    impl Widget for PlacedWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            PlacedElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for PlacedWidget {}

    #[test]
    fn every_transition_reports_where_its_child_is() {
        // A host that measures the page it presents, such as the navigator's swipe
        // zone, asks the page's root for its bounds. A transition wrapping the
        // root is transparent to layout, so it has to answer with its child's.
        let ctx = dummy_build_context();
        let bounds = Some(BOUNDS);

        assert_eq!(FadeTransition::new(controller(), PlacedWidget).to_element(&ctx).pos_start_end(), bounds);
        assert_eq!(
            SlideTransition::new(controller(), (10.0, 10.0), PlacedWidget).to_element(&ctx).pos_start_end(),
            bounds
        );
        assert_eq!(ScaleTransition::new(controller(), PlacedWidget).to_element(&ctx).pos_start_end(), bounds);
        assert_eq!(RotationTransition::new(controller(), PlacedWidget).to_element(&ctx).pos_start_end(), bounds);
    }

    #[test]
    fn rotation_transition_interpolates_a_custom_turn_range() {
        assert_eq!(interpolate_turns(0.0, -0.25, 0.0), 0.0);
        assert_eq!(interpolate_turns(0.0, -0.25, 0.5), -0.125);
        assert_eq!(interpolate_turns(0.0, -0.25, 1.0), -0.25);
    }

    #[test]
    fn a_valid_sample_goes_to_the_compositor_and_an_invalid_one_stays_live() {
        let ctx = dummy_build_context();

        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.set_value(0.5);
        let element = FadeTransition::new(controller, TestWidget).to_element(&ctx);
        assert!(matches!(
            element.compositor_animation(&ctx),
            CompositorAnimationDecision::Compositor(frame) if frame.valid
        ));

        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.set_value(f32::NAN);
        let element = FadeTransition::new(controller, TestWidget).to_element(&ctx);
        assert!(matches!(
            element.compositor_animation(&ctx),
            CompositorAnimationDecision::Live(frame) if !frame.valid
        ));
    }

    fn assert_defers_next_frame(widget: impl Widget + 'static) {
        test_frame_requester::reset();
        let ctx = dummy_build_context();
        let element = widget.to_element(&ctx);

        element.update(&ctx);

        assert_eq!(test_frame_requester::count(), 1);
        assert!(!ctx.window.take_redraw_request());
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn active_explicit_transitions_defer_their_next_frame_request() {
        test_frame_requester::install();

        assert_defers_next_frame(FadeTransition::new(controller(), TestWidget));
        assert_defers_next_frame(SlideTransition::new(controller(), (10.0, 10.0), TestWidget));
        assert_defers_next_frame(ScaleTransition::new(controller(), TestWidget));
        assert_defers_next_frame(RotationTransition::new(controller(), TestWidget));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_transitions_lower_to_their_current_child() {
        use aimer_widget::portable::__anteros::{Version, WIDGET_TEXT, WidgetDocumentView};
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };
        use aimer_widget::{AnyElement, PortableWidget};

        struct PortableLeaf;

        impl Widget for PortableLeaf {
            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                panic!("portable transition test must not enter native element construction")
            }
        }

        impl PortableWidget for PortableLeaf {
            fn to_portable_node(
                self,
                ctx: &mut PortableBuildContext,
                source: SourceFingerprint,
            ) -> Result<
                aimer_widget::portable::PortableNodeId,
                aimer_widget::portable::PortableBuildError,
            > {
                ctx.push_node(WIDGET_TEXT, Version::new(1, 0), None, source, &[], &[])
            }
        }

        fn assert_transparent<W: Widget + 'static>(widget: W) {
            let mut context = PortableBuildContext::new(
                1,
                1,
                PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
                PortableLimits::new(8, 16, 64, 128, 1_024),
            )
            .unwrap();
            let root = widget
                .to_portable_node(
                    &mut context,
                    SourceFingerprint::new(StableId128::from_bytes([9; 16])),
                )
                .unwrap();
            let document = context.finish_document(root).unwrap();
            let bytes = document.encode().unwrap();
            let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();

            assert_eq!(view.node_count(), 1);
            assert_eq!(
                view.node(view.root_node()).unwrap().widget_type(),
                WIDGET_TEXT
            );
        }

        assert_transparent(FadeTransition::new(controller(), PortableLeaf));
        assert_transparent(SlideTransition::new(
            controller(),
            (10.0, 10.0),
            PortableLeaf,
        ));
        assert_transparent(ScaleTransition::new(controller(), PortableLeaf));
        assert_transparent(RotationTransition::new(controller(), PortableLeaf));
    }
}
