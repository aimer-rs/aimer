use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Mat3;
use aimer_events::element::ElementEvent;
use aimer_events::window::request_animation_frame;
use aimer_widget::base::*;
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, LayoutElement, Rebuildable,
    VisitorElement, Widget, request_isolated_animation_frame,
};

use super::container_transform::ElementBounds;
use crate::control::controller::AnimationController;
use crate::primitives::time::AnimInstant;

/// How [`AnimatedPaint`] presents its child for one controller sample.
///
/// The child keeps its recorded paint; the transform, opacity and corner
/// radius are applied to the child's render node, so a frame costs no
/// re-recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintPresentation {
    /// Maps the child's logical coordinates (relative to its own origin) to
    /// where it is shown. It must be finite.
    pub transform: Mat3,
    /// Multiplies the child's opacity. Values are clamped to `0.0..=1.0`; a
    /// non-finite value paints nothing for that frame.
    pub opacity: f32,
    /// Rounds the corners of the child's own bounds, measured as shown: after
    /// `transform`, in logical pixels. `0.0` adds no clip at all, so a child
    /// that clips itself keeps doing so. A negative or non-finite value paints
    /// nothing for that frame.
    pub corner_radius: f32,
    /// The part of the child that shows, in the child's own coordinates before
    /// `transform`: everything outside it is clipped away, and `corner_radius`
    /// rounds this rectangle instead of the child's bounds. `None` shows the
    /// whole child.
    ///
    /// A child that grows out of a smaller item by revealing more of itself,
    /// rather than by scaling, animates this rectangle from the item's bounds
    /// to the child's own.
    pub reveal: Option<ElementBounds>,
}

impl PaintPresentation {
    /// The child shown unchanged.
    pub const IDENTITY_OPAQUE: Self = Self {
        transform: Mat3::identity(),
        opacity: 1.0,
        corner_radius: 0.0,
        reveal: None,
    };

    /// Whether every component can be shown. A presentation that is not is
    /// declined for the frame instead of painting the child wrongly.
    pub fn is_finite(&self) -> bool {
        self.opacity.is_finite()
            && self.corner_radius.is_finite()
            && self.corner_radius >= 0.0
            && self.transform.cols.iter().flatten().all(|value| value.is_finite())
            && self.reveal.is_none_or(|reveal| reveal.is_valid())
    }

    /// Applies this presentation to the retained child at `child_index` of the
    /// element being synchronized, returning whether it could be shown.
    ///
    /// `child_size` is the child's size in logical pixels; a corner radius
    /// clips exactly that rectangle, unless a [`reveal`](Self::reveal) names
    /// another. `clip` remembers whether this host put a
    /// clip on the child, so that a presentation without a radius removes only
    /// what the host added.
    ///
    /// Call it from `Drawable::sync_local_v2_state`.
    pub fn apply_to_child(
        &self,
        ctx: &BuildContext,
        child_index: usize,
        child_size: ResolvedSize,
        clip: &PresentationClip,
    ) -> bool {
        if !self.is_finite() {
            return false;
        }
        if !ctx.set_local_v2_child_presentation_at(
            child_index,
            self.transform,
            self.opacity.clamp(0.0, 1.0),
        ) {
            return false;
        }
        // The clip is local to the child, so the renderer scales its radius by
        // the transform; divide it out to make `corner_radius` the shown size.
        let scale = (self.transform.cols[0][0].abs() + self.transform.cols[1][1].abs()) * 0.5;
        let fits = child_size.width.is_finite()
            && child_size.height.is_finite()
            && child_size.width > 0.0
            && child_size.height > 0.0;
        if (self.corner_radius > 0.0 || self.reveal.is_some()) && scale > 0.0 && fits {
            clip.applied.set(true);
            let (x, y, width, height) = match self.reveal {
                Some(reveal) => (reveal.x, reveal.y, reveal.width, reveal.height),
                None => (0.0, 0.0, child_size.width, child_size.height),
            };
            ctx.set_local_v2_child_clip_at(
                child_index,
                Some((x, y, width, height)),
                [self.corner_radius / scale; 4],
            )
        } else if clip.applied.replace(false) {
            ctx.set_local_v2_child_clip_at(child_index, None, [0.0; 4])
        } else {
            true
        }
    }
}

/// Remembers whether a host has put a rounded clip on its child.
///
/// A presentation with no corner radius must not touch the child's clip, which
/// the child may own. Hosts keep one of these per presented child so that, when
/// the radius returns to zero, only the clip they added is removed.
#[derive(Debug, Default)]
pub struct PresentationClip {
    applied: std::cell::Cell<bool>,
}

type PresentationFn = dyn Fn(f32) -> PaintPresentation + 'static;

/// Receives the controller's curved value, the child's size in logical pixels,
/// and the wrapper's top-left corner in the window in logical pixels.
type GeometryFn = dyn Fn(f32, ResolvedSize, (f32, f32)) -> PaintPresentation + 'static;

enum PaintEffect {
    Noop,
    Opacity,
    Presentation(Box<PresentationFn>),
    Geometry(Box<GeometryFn>),
}

/// Applies a controller-driven presentation to a retained child.
///
/// `AnimatedPaint` converts `child` to an element exactly once. Each frame it
/// samples the controller, presents the child's render node with the sampled
/// transform and opacity, and schedules another frame while the controller is
/// active. The child is never rebuilt or re-recorded by the animation.
///
/// # Example
/// ```rust
/// use std::time::Duration;
///
/// use aimer_animation::{AnimatedPaint, AnimationController, curves};
/// use aimer_widget::ErrorWidget;
///
/// let controller = AnimationController::new(Duration::from_millis(250), curves::LINEAR);
/// let animated = AnimatedPaint::new(controller, ErrorWidget::new("Loading")).opacity();
/// ```
pub struct AnimatedPaint<T> {
    controller: AnimationController,
    child: T,
    effect: PaintEffect,
    inert: bool,
}

impl<T: Widget> AnimatedPaint<T> {
    /// Creates a retained paint wrapper without starting or resetting the
    /// controller. The default presents the child unchanged.
    #[inline]
    pub fn new(controller: AnimationController, child: T) -> Self {
        Self {
            controller,
            child,
            effect: PaintEffect::Noop,
            inert: false,
        }
    }

    /// Makes the child take no pointer input and no focus, while it is still
    /// painted and presented as usual.
    ///
    /// For a copy of something drawn above the real thing, such as the item a
    /// container transform scales up, which must not answer a tap that the
    /// item underneath is also waiting for.
    #[inline]
    pub fn inert(mut self) -> Self {
        self.inert = true;
        self
    }

    /// Sets the presentation applied to the retained child.
    ///
    /// The closure is retained by the element and must therefore be `'static`.
    /// It runs on the UI/rendering thread with the controller's curved value,
    /// once per frame.
    #[inline]
    pub fn effect<F>(mut self, effect: F) -> Self
    where
        F: Fn(f32) -> PaintPresentation + 'static,
    {
        self.effect = PaintEffect::Presentation(Box::new(effect));
        self
    }

    /// Sets a presentation that depends on where the child is.
    ///
    /// Like [`effect`](Self::effect), but the closure also receives the child's
    /// size and the wrapper's top-left corner in the window, both in logical
    /// pixels, read live each frame. An effect that grows the child out of
    /// something measured in the window, such as a
    /// [`SharedElement`](crate::SharedElement), needs both.
    #[inline]
    pub fn geometry_effect<F>(mut self, effect: F) -> Self
    where
        F: Fn(f32, ResolvedSize, (f32, f32)) -> PaintPresentation + 'static,
    {
        self.effect = PaintEffect::Geometry(Box::new(effect));
        self
    }

    /// Fades the retained child with compositor opacity, keeping its draw list
    /// unchanged while the controller advances.
    #[inline]
    pub fn opacity(mut self) -> Self {
        self.effect = PaintEffect::Opacity;
        self
    }
}

impl<T: Widget + 'static> Widget for AnimatedPaint<T> {
    fn debug_name(&self) -> &'static str {
        "AnimatedPaint"
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        AnimatedPaintElement {
            child: self.child.to_element(ctx),
            controller: self.controller,
            effect: self.effect,
            inert: self.inert,
            clip: PresentationClip::default(),
        }
        .boxed()
    }
}

// The portable interface cannot represent an arbitrary native closure. The
// default PortableWidget implementation therefore reports the existing
// unsupported-widget diagnostic when this wrapper is lowered in a guest.
impl<T: Widget + 'static> aimer_widget::PortableWidget for AnimatedPaint<T> {}

/// Retained implementation behind [`AnimatedPaint`].
struct AnimatedPaintElement {
    child: AnyElement,
    controller: AnimationController,
    effect: PaintEffect,
    inert: bool,
    clip: PresentationClip,
}

// Safety: widget rendering and the retained element tree are single-threaded.
unsafe impl Send for AnimatedPaintElement {}
unsafe impl Sync for AnimatedPaintElement {}

impl AnimatedPaintElement {
    /// The presentation for one sampled controller value. `size` is the
    /// child's logical size and `origin` the wrapper's window position.
    fn presentation(&self, progress: f32, size: ResolvedSize, origin: impl FnOnce() -> (f32, f32)) -> PaintPresentation {
        match &self.effect {
            PaintEffect::Geometry(effect) => effect(progress, size, origin()),
            PaintEffect::Noop => PaintPresentation::IDENTITY_OPAQUE,
            PaintEffect::Opacity => PaintPresentation {
                opacity: progress.clamp(0.0, 1.0),
                ..PaintPresentation::IDENTITY_OPAQUE
            },
            PaintEffect::Presentation(effect) => effect(progress),
        }
    }
}

impl Drawable for AnimatedPaintElement {
    fn update(&self, ctx: &BuildContext) {
        self.controller.tick(AnimInstant::now());
        let animating = self.controller.is_animating();
        self.child.update(ctx);
        if animating {
            request_animation_frame();
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        let progress = self.controller.tick(AnimInstant::now());
        if self.controller.is_animating() {
            // Everything a frame of this animation changes is the child's
            // presentation set below, so the next frame may revisit this
            // element alone. The child is retained and never re-recorded.
            request_isolated_animation_frame(ctx);
        }
        if !progress.is_finite() {
            return false;
        }
        self.child.sync_paint_geometry(ctx);
        let size = self.child.computed_size(ctx);
        let size = ResolvedSize {
            width: size.width / ctx.scale,
            height: size.height / ctx.scale,
        };
        let presentation = self.presentation(progress, size, || {
            ctx.local_v2_node_handle()
                .and_then(|node| node.world_bounds())
                .map_or((0.0, 0.0), |(x, y, _, _)| (x, y))
        });
        presentation.apply_to_child(ctx, 0, size, &self.clip)
    }

    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        // `sync_local_v2_state` has already asked for the next frame.
        self.child.update(ctx);
    }
}

impl VisitorElement for AnimatedPaintElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedPaintElement"
    }
}

impl EventElement for AnimatedPaintElement {
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if self.inert {
            return EventResult::ignored();
        }
        self.child.on_event(event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if !self.inert {
            visitor(self.child.as_ref());
        }
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // Reconciliation still needs the child even when it takes no input.
        visitor(self.child.as_ref());
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if !self.inert {
            visitor(self.child.as_ref());
        }
    }
}

impl Rebuildable for AnimatedPaintElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for AnimatedPaintElement {
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

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::OnceLock;

    use aimer_attribute::BoxConstraint;
    use aimer_widget::base::{BuildContext, ResolvedSize, Vec2d, WindowHandle};
    use aimer_widget::{
        AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
        Widget,
    };

    use super::super::test_frame_requester;
    use super::{AnimatedPaint, PaintPresentation};
    use crate::{AnimationController, curves};

    const FRAME_WIDTH: u32 = 101;
    const FRAME_HEIGHT: u32 = 101;

    fn runtime_handle() -> tokio::runtime::Handle {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("animated paint test runtime should build")
            })
            .handle()
            .clone()
    }

    fn context() -> BuildContext<'static> {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        let mut context = BuildContext::new(
            aimer_canvas::FrameCanvas::new(inner),
            ResolvedSize {
                width: FRAME_WIDTH as f32,
                height: FRAME_HEIGHT as f32,
            },
            1.0,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0),
            runtime_handle(),
        );
        context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: FRAME_WIDTH as f32,
            max_height: FRAME_HEIGHT as f32,
        };
        context
    }

    /// Runs the element's state synchronization the way the render-tree sync
    /// does: against a root node that has the element's child as its only child.
    fn sync_state(element: &dyn Element, context: &BuildContext<'_>) -> bool {
        use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};

        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 101.0, 101.0)).unwrap();
        tree.add_child(root, Rect::new(0.0, 0.0, 10.0, 10.0))
            .unwrap();
        let retained = tree.context(root).unwrap();
        context
            .with_local_v2_paint_context(retained, |context| element.sync_local_v2_state(context))
    }

    struct StableLeaf;

    impl Widget for StableLeaf {
        fn to_element(self, _context: &BuildContext) -> AnyElement {
            StableElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for StableLeaf {}

    struct StableElement;

    impl Drawable for StableElement {
        fn update(&self, _context: &BuildContext) {}
    }

    impl EventElement for StableElement {}

    impl LayoutElement for StableElement {
        fn computed_size(&self, _context: &BuildContext) -> ResolvedSize {
            ResolvedSize {
                width: 10.0,
                height: 10.0,
            }
        }
    }

    impl Rebuildable for StableElement {}

    impl VisitorElement for StableElement {
        fn debug_name(&self) -> &'static str {
            "StableElement"
        }
    }

    struct CountingWidget {
        builds: Rc<Cell<usize>>,
    }

    impl Widget for CountingWidget {
        fn to_element(self, _context: &BuildContext) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            StableElement.boxed()
        }
    }

    impl aimer_widget::PortableWidget for CountingWidget {}

    fn controller() -> AnimationController {
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.forward_from_first_tick();
        controller
    }

    #[test]
    fn retained_child_is_built_once() {
        let builds = Rc::new(Cell::new(0));
        let context = context();
        let element = AnimatedPaint::new(
            controller(),
            CountingWidget {
                builds: builds.clone(),
            },
        )
        .opacity()
        .to_element(&context);

        assert_eq!(builds.get(), 1);
        assert!(sync_state(element.as_ref(), &context));
        assert!(sync_state(element.as_ref(), &context));
        assert_eq!(builds.get(), 1);
    }

    #[test]
    fn effect_receives_the_controller_value() {
        let value = Rc::new(Cell::new(-1.0));
        let observed = value.clone();
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.set_value(0.25);
        let context = context();
        let element = AnimatedPaint::new(controller, StableLeaf)
            .effect(move |current| {
                observed.set(current);
                PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);

        assert!(sync_state(element.as_ref(), &context));

        assert!((value.get() - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn effect_presentation_reaches_the_child_node() {
        let controller = AnimationController::with_millis(100, curves::LINEAR);
        controller.set_value(0.5);
        let context = context();
        let shifted = AnimatedPaint::new(controller.clone(), StableLeaf)
            .effect(|value| PaintPresentation {
                transform: aimer_canvas::Mat3::translate(value * 10.0, 0.0),
                opacity: value,
                ..PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);
        assert!(sync_state(shifted.as_ref(), &context));

        // A presentation the render tree cannot show declines instead of
        // painting the child wrongly.
        let invalid = AnimatedPaint::new(controller.clone(), StableLeaf)
            .effect(|_| PaintPresentation {
                transform: aimer_canvas::Mat3::translate(f32::NAN, 0.0),
                ..PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);
        assert!(!sync_state(invalid.as_ref(), &context));

        let non_finite_opacity = AnimatedPaint::new(controller, StableLeaf)
            .effect(|_| PaintPresentation {
                opacity: f32::NAN,
                ..PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);
        assert!(!sync_state(non_finite_opacity.as_ref(), &context));
    }

    #[test]
    #[cfg(not(target_os = "ios"))]
    fn active_animation_requests_the_next_frame() {
        test_frame_requester::install();
        test_frame_requester::reset();
        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf).to_element(&context);

        element.update(&context);

        assert_eq!(test_frame_requester::count(), 1);
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn portable_lowering_reports_the_native_only_widget() {
        use aimer_widget::PortableWidget;
        use aimer_widget::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let result = AnimatedPaint::new(
            AnimationController::with_millis(100, curves::LINEAR),
            StableLeaf,
        )
        .to_portable_node(
            &mut context,
            SourceFingerprint::new(StableId128::from_bytes([9; 16])),
        );

        assert!(result.is_err());
    }

    /// A retained tree whose root holds the element's child, kept alive so a
    /// test can read back what synchronization did to that child.
    struct Retained {
        tree: aimer_cupid::draw_cmd_v2::RenderTree,
        root: aimer_cupid::draw_cmd_v2::RenderNodeId,
        child: aimer_cupid::draw_cmd_v2::RenderNodeId,
    }

    impl Retained {
        fn new() -> Self {
            Self::at(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 101.0, 101.0))
        }

        fn at(bounds: aimer_cupid::draw_cmd_v2::Rect) -> Self {
            use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};

            let tree = RenderTree::new();
            let root = tree.add_root(bounds).unwrap();
            let child = tree
                .add_child(root, Rect::new(0.0, 0.0, 10.0, 10.0))
                .unwrap();
            Self { tree, root, child }
        }

        fn sync(&self, element: &dyn Element, context: &BuildContext<'_>) -> bool {
            let retained = self.tree.context(self.root).unwrap();
            context.with_local_v2_paint_context(retained, |context| {
                element.sync_local_v2_state(context)
            })
        }

        fn drawn_child(&self) -> aimer_cupid::draw_cmd_v2::RenderItem {
            use aimer_cupid::draw_cmd_v2::RenderOp;

            self.tree
                .render_all()
                .into_iter()
                .find_map(|op| match op {
                    RenderOp::Draw(item) if item.element == self.child => Some(item),
                    _ => None,
                })
                .expect("the child must draw")
        }
    }

    fn rounded(radius: Rc<Cell<f32>>) -> impl Fn(f32) -> PaintPresentation {
        move |_| PaintPresentation {
            transform: aimer_canvas::Mat3::translate(20.0, 30.0)
                .mul(&aimer_canvas::Mat3::scale(0.5, 0.5)),
            corner_radius: radius.get(),
            ..PaintPresentation::IDENTITY_OPAQUE
        }
    }

    #[test]
    fn a_corner_radius_clips_the_child_and_follows_its_transform() {
        use aimer_cupid::draw_cmd_v2::Rect;

        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .effect(rounded(Rc::new(Cell::new(2.0))))
            .to_element(&context);
        let retained = Retained::new();

        assert!(retained.sync(element.as_ref(), &context));

        // The child is 10x10, drawn at half size, so the visual radius of 2
        // is a local radius of 4.
        let item = retained.drawn_child();
        assert_eq!(item.clip, Some(Rect::new(20.0, 30.0, 5.0, 5.0)));
        assert_eq!(item.clip_radius, [2.0; 4]);
    }

    #[test]
    fn without_a_corner_radius_the_childs_own_clip_is_left_alone() {
        use aimer_cupid::draw_cmd_v2::Rect;

        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .effect(rounded(Rc::new(Cell::new(0.0))))
            .to_element(&context);
        let retained = Retained::new();
        retained
            .tree
            .set_clip(retained.child, Some(Rect::new(0.0, 0.0, 6.0, 6.0)))
            .unwrap();

        assert!(retained.sync(element.as_ref(), &context));

        assert_eq!(
            retained.drawn_child().clip,
            Some(Rect::new(20.0, 30.0, 3.0, 3.0))
        );
    }

    #[test]
    fn dropping_the_corner_radius_removes_the_clip_it_added() {
        let radius = Rc::new(Cell::new(2.0));
        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .effect(rounded(radius.clone()))
            .to_element(&context);
        let retained = Retained::new();
        assert!(retained.sync(element.as_ref(), &context));
        assert!(retained.drawn_child().clip.is_some());

        radius.set(0.0);
        assert!(retained.sync(element.as_ref(), &context));

        assert_eq!(retained.drawn_child().clip, None);
        assert_eq!(retained.drawn_child().clip_radius, [0.0; 4]);
    }

    #[test]
    fn a_reveal_clips_the_child_to_that_window_and_leaving_it_removes_the_clip() {
        use aimer_cupid::draw_cmd_v2::Rect;

        let window = Rc::new(Cell::new(Some(super::ElementBounds::new(2.0, 4.0, 4.0, 2.0))));
        let shown = window.clone();
        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .effect(move |_| PaintPresentation {
                transform: aimer_canvas::Mat3::translate(20.0, 30.0)
                    .mul(&aimer_canvas::Mat3::scale(0.5, 0.5)),
                corner_radius: 2.0,
                reveal: shown.get(),
                ..PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);
        let retained = Retained::new();

        assert!(retained.sync(element.as_ref(), &context));

        // The window is in the child's own coordinates, so it moves and shrinks
        // with the child; the radius is still the size shown.
        let item = retained.drawn_child();
        assert_eq!(item.clip, Some(Rect::new(21.0, 32.0, 2.0, 1.0)));
        assert_eq!(item.clip_radius, [2.0; 4]);

        // Without a window the rounded corners go back to the child's bounds.
        window.set(None);
        assert!(retained.sync(element.as_ref(), &context));
        assert_eq!(retained.drawn_child().clip, Some(Rect::new(20.0, 30.0, 5.0, 5.0)));
    }

    #[test]
    fn a_reveal_without_a_corner_radius_still_clips() {
        use aimer_cupid::draw_cmd_v2::Rect;

        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .effect(|_| PaintPresentation {
                reveal: Some(super::ElementBounds::new(0.0, 0.0, 6.0, 3.0)),
                ..PaintPresentation::IDENTITY_OPAQUE
            })
            .to_element(&context);
        let retained = Retained::new();

        assert!(retained.sync(element.as_ref(), &context));

        assert_eq!(retained.drawn_child().clip, Some(Rect::new(0.0, 0.0, 6.0, 3.0)));
    }

    #[test]
    fn an_empty_or_non_finite_reveal_declines_the_frame() {
        let context = context();
        for reveal in [
            super::ElementBounds::new(0.0, 0.0, 0.0, 5.0),
            super::ElementBounds::new(0.0, 0.0, 5.0, -1.0),
            super::ElementBounds::new(f32::NAN, 0.0, 5.0, 5.0),
        ] {
            let element = AnimatedPaint::new(controller(), StableLeaf)
                .effect(move |_| PaintPresentation {
                    reveal: Some(reveal),
                    ..PaintPresentation::IDENTITY_OPAQUE
                })
                .to_element(&context);

            assert!(!Retained::new().sync(element.as_ref(), &context), "{reveal:?}");
        }
    }

    #[test]
    fn an_invalid_corner_radius_declines_the_frame() {
        let context = context();
        for radius in [f32::NAN, f32::INFINITY, -1.0] {
            let element = AnimatedPaint::new(controller(), StableLeaf)
                .effect(rounded(Rc::new(Cell::new(radius))))
                .to_element(&context);

            assert!(!Retained::new().sync(element.as_ref(), &context), "{radius}");
        }
    }

    #[test]
    fn a_geometry_effect_is_told_the_childs_size_and_where_the_wrapper_is_in_the_window() {
        let seen = Rc::new(Cell::new(None));
        let recorded = seen.clone();
        let context = context();
        let element = AnimatedPaint::new(controller(), StableLeaf)
            .geometry_effect(move |progress, size, origin| {
                recorded.set(Some((progress, size.width, size.height, origin)));
                PaintPresentation {
                    transform: aimer_canvas::Mat3::translate(origin.0, origin.1),
                    ..PaintPresentation::IDENTITY_OPAQUE
                }
            })
            .to_element(&context);
        let retained = Retained::at(aimer_cupid::draw_cmd_v2::Rect::new(30.0, 40.0, 101.0, 101.0));

        assert!(retained.sync(element.as_ref(), &context));

        // The child is 10x10 at scale 1; progress has not started yet.
        assert_eq!(seen.get(), Some((0.0, 10.0, 10.0, (30.0, 40.0))));
        let (transform, _) = retained.tree.presentation(retained.child).unwrap();
        assert_eq!(transform, aimer_canvas::Mat3::translate(30.0, 40.0));
    }

    fn count(visit: impl Fn(&mut dyn FnMut(&dyn Element))) -> usize {
        let mut count = 0;
        visit(&mut |_| count += 1);
        count
    }

    #[test]
    fn an_inert_wrapper_takes_no_input_and_no_focus_but_still_paints() {
        let context = context();
        let live = AnimatedPaint::new(controller(), StableLeaf).to_element(&context);
        let inert = AnimatedPaint::new(controller(), StableLeaf).inert().to_element(&context);

        assert_eq!(count(|visit| live.event_children(visit)), 1);
        assert_eq!(count(|visit| live.focus_children(visit)), 1);
        assert_eq!(count(|visit| inert.event_children(visit)), 0);
        assert_eq!(count(|visit| inert.focus_children(visit)), 0);
        // It is still part of the tree and still presents its child.
        assert_eq!(count(|visit| inert.structural_children(visit)), 1);
        assert!(Retained::new().sync(inert.as_ref(), &context));
    }
}
