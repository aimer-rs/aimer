//! Track mattes: a layer shown only where another layer has coverage.
//!
//! The SVG pipeline can only clip to a rectangle, so a matte is not applied
//! inside a scene. It is applied by the renderer's coverage mask instead, which
//! keeps an element's own drawing only where its descendants have alpha. A
//! scene with mattes therefore becomes a small tree:
//!
//! ```text
//! RawMattes                      sizes the box, owns the clock
//!   Stack                        one child per block, bottom first
//!     LottiePlane                a plain plane
//!     MaskedDecorationBox        the cut layer, drawn by `Cut`, ...
//!       LottiePlane                ... shown where this matte plane has alpha
//!     LottiePlane
//! ```
//!
//! Every piece reads the same [`Clock`], so the cut layer and its matte always
//! show the same frame.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use aimer_attribute::BoxConstraint;
use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd::Rect;
use aimer_decoration::{Decoration, MaskedDecorationBox};
use aimer_space::Stack;
use aimer_utils::AnimInstant;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    Rebuildable, VisitorElement, Widget, request_isolated_subtree_animation_frame,
};

use crate::playback::Playback;
use crate::scene::{Block, LottieScene};
use crate::widget::{
    LottieFit, PaintKey, draw_plane, lottie_layout_is_stable, lottie_size,
};

/// The playhead shared by every piece of one animation.
pub(crate) struct Clock {
    pub(crate) playback: Cell<Playback>,
    pub(crate) last_tick: Cell<Option<AnimInstant>>,
}

impl Clock {
    pub(crate) fn new(playback: Playback) -> Self {
        Self { playback: Cell::new(playback), last_tick: Cell::new(None) }
    }

    /// The frame to show.
    #[inline]
    pub(crate) fn frame(&self) -> f32 {
        self.playback.get().frame()
    }

    /// Moves the playhead to now and says whether it wants another frame.
    fn advance(&self) -> bool {
        let now = AnimInstant::now();
        let dt = self.last_tick.get().map_or(Duration::ZERO, |previous| now.duration_since(previous));
        let mut playback = self.playback.get();
        let tick = playback.advance(dt);
        self.playback.set(playback);
        // Idle time must not be counted when playback resumes.
        self.last_tick.set(tick.wants_frame.then_some(now));
        tick.wants_frame
    }
}

/// Builds the element of an animation that has track mattes.
pub(crate) fn matted_element(
    scene: Arc<LottieScene>,
    width: Dimension,
    height: Dimension,
    fit: LottieFit,
    fixed: bool,
    clock: Rc<Clock>,
    ctx: &BuildContext,
) -> AnyElement {
    let layers: Vec<AnyWidget> = scene
        .blocks()
        .iter()
        .map(|block| match *block {
            Block::Plain(plane) => PlaneWidget::new(scene.clone(), plane, fit, clock.clone()).boxed(),
            Block::Matted { content, matte } => MaskedDecorationBox::new()
                .decoration(Cut::new(scene.clone(), content, fit, clock.clone()))
                .child(PlaneWidget::new(scene.clone(), matte, fit, clock.clone()))
                .boxed(),
        })
        .collect();
    RawMattes {
        scene,
        width,
        height,
        fixed,
        clock,
        child: Stack::new().children(layers).to_element(ctx),
    }
    .boxed()
}

/// The box of an animation with mattes. It decides the size, advances the clock
/// and asks for frames; everything it draws is below it.
pub(crate) struct RawMattes {
    pub(crate) scene: Arc<LottieScene>,
    width: Dimension,
    height: Dimension,
    /// A fixed `progress` was requested, so the clock never moves.
    fixed: bool,
    pub(crate) clock: Rc<Clock>,
    child: AnyElement,
}

impl RawMattes {
    /// The context the stack below sees: exactly this box.
    fn child_ctx<'a>(&self, ctx: &BuildContext<'a>) -> BuildContext<'a> {
        let size = self.computed_size(ctx);
        let mut child = ctx.clone();
        child.parent_size = size;
        child.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        child
    }

    fn is_paintable(&self, ctx: &BuildContext) -> bool {
        let size = self.computed_size(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
    }
}

impl Drawable for RawMattes {
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.is_paintable(ctx)
    }

    fn paint(&self, _ctx: &BuildContext) {}

    fn sync_paint_state(&self, ctx: &BuildContext) -> bool {
        if self.clock.advance() {
            // The pieces that change are the ones below this element, so the
            // next animation-only frame walks the subtree and nothing else.
            request_isolated_subtree_animation_frame(ctx);
        }
        true
    }

    fn child_context<'a>(&self, ctx: &BuildContext<'a>, _child: &dyn Element) -> Option<BuildContext<'a>> {
        self.is_paintable(ctx).then(|| self.child_ctx(ctx))
    }

    fn child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(Rect, Option<Rect>)> {
        if !self.is_paintable(ctx) {
            return None;
        }
        let child_ctx = self.child_ctx(ctx);
        let position = child.pos().unwrap_or_default();
        let size = child
            .paint_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        let bounds = Rect::new(
            position.x / ctx.scale,
            position.y / ctx.scale,
            size.width / ctx.scale,
            size.height / ctx.scale,
        );
        let finite = [bounds.x, bounds.y, bounds.width, bounds.height].iter().all(|v| v.is_finite());
        (finite && bounds.width >= 0.0 && bounds.height >= 0.0)
            .then(|| (bounds, child.retained_clip(&child_ctx)))
    }

    fn update(&self, ctx: &BuildContext) {
        if self.is_paintable(ctx) {
            ctx.canvas.save();
            self.child.update(&self.child_ctx(ctx));
            ctx.canvas.restore();
        }
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        if self.is_paintable(ctx) {
            self.child.sync_paint_geometry(&self.child_ctx(ctx));
        }
    }
}

impl LayoutElement for RawMattes {
    fn size(&self) -> Option<aimer_attribute::size::Size> {
        match (self.width, self.height) {
            (Dimension::Px(_), Dimension::Px(_)) => {
                Some(aimer_attribute::size::Size { width: self.width, height: self.height })
            }
            _ => None,
        }
    }

    fn is_layout_stable(&self) -> bool {
        lottie_layout_is_stable(self.width, self.height)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        lottie_size(&self.scene, self.width, self.height, ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

impl EventElement for RawMattes {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawMattes {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Lottie"
    }
}

impl Rebuildable for RawMattes {
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    /// Keeps the playhead across a rebuild of an ancestor; see
    /// [`RawLottie`](crate::widget::RawLottie). The pieces below share this
    /// clock, so they need nothing of their own.
    fn adopt_runtime_state_from(&self, old: &dyn Element) {
        let Some(old) = old.option_any().and_then(|any| any.downcast_ref::<Self>()) else {
            return;
        };
        if self.fixed || !Arc::ptr_eq(&self.scene, &old.scene) {
            return;
        }
        let mut playback = self.clock.playback.get();
        playback.adopt_position(&old.clock.playback.get());
        self.clock.playback.set(playback);
        self.clock.last_tick.set(old.clock.last_tick.get());
    }
}

/// One plane of a scene, drawn as it is into whatever space it is given.
pub(crate) struct PlaneWidget {
    scene: Arc<LottieScene>,
    plane: usize,
    fit: LottieFit,
    clock: Rc<Clock>,
}

impl PlaneWidget {
    pub(crate) fn new(scene: Arc<LottieScene>, plane: usize, fit: LottieFit, clock: Rc<Clock>) -> Self {
        Self { scene, plane, fit, clock }
    }
}

impl Widget for PlaneWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawPlane {
            scene: self.scene,
            plane: self.plane,
            fit: self.fit,
            clock: self.clock,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "LottiePlane"
    }
}

struct RawPlane {
    scene: Arc<LottieScene>,
    plane: usize,
    fit: LottieFit,
    clock: Rc<Clock>,
    /// What the last recording was made for: the box and the frame.
    recorded: Cell<Option<(PaintKey, u32)>>,
}

impl RawPlane {
    fn target(&self, ctx: &BuildContext) -> Option<(PaintKey, ResolvedSize)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        if !size.width.is_finite() || !size.height.is_finite() || size.width < 0.0 || size.height < 0.0 {
            return None;
        }
        // Layout sizes are device-scaled; recorded commands use logical pixels.
        Some((
            PaintKey::new(size.width, size.height, scale),
            ResolvedSize { width: size.width / scale, height: size.height / scale },
        ))
    }
}

impl Drawable for RawPlane {
    #[inline]
    fn can_paint(&self, ctx: &BuildContext) -> bool {
        self.target(ctx).is_some()
    }

    fn paint(&self, ctx: &BuildContext) {
        let (key, size) = self.target(ctx).expect("Lottie support must be checked before painting");
        let frame = self.clock.frame();
        self.recorded.set(Some((key, frame.to_bits())));
        let canvas = Canvas::of(ctx);
        draw_plane(&canvas, &self.scene, self.plane, frame, self.fit, Vec2d { x: 0.0, y: 0.0 }, size);
        canvas.finish();
    }

    #[inline]
    fn paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        let now = self.target(ctx).map(|(key, _)| (key, self.clock.frame().to_bits()));
        self.recorded.get() != now
    }
}

impl LayoutElement for RawPlane {
    /// A plane has no size of its own: it fills the box it is placed in.
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ctx.parent_size
    }
}

impl EventElement for RawPlane {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl VisitorElement for RawPlane {
    fn debug_name(&self) -> &'static str {
        "LottiePlane"
    }
}

impl Rebuildable for RawPlane {}

/// The layer a matte cuts, painted as the decoration of a masked box: the box
/// shows it only where its child, the matte plane, has coverage.
pub(crate) struct Cut {
    scene: Arc<LottieScene>,
    plane: usize,
    fit: LottieFit,
    clock: Rc<Clock>,
}

impl Cut {
    pub(crate) fn new(scene: Arc<LottieScene>, plane: usize, fit: LottieFit, clock: Rc<Clock>) -> Self {
        Self { scene, plane, fit, clock }
    }
}

impl Decoration for Cut {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        draw_plane(canvas, &self.scene, self.plane, self.clock.frame(), self.fit, origin, size);
    }

    /// The picture changes with the frame. The clock is advanced by the wrapper
    /// above, which is visited before the box this decoration belongs to.
    fn revision(&self) -> u64 {
        u64::from(self.clock.frame().to_bits()) + 1
    }
}
