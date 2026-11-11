use aimer_attribute::size::ResolvedSize;

use crate::base::BuildContext;
use crate::components::element::Element;

/// A transform that can be applied without changing retained paint commands.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositorTransform {
    /// Leaves the current transform unchanged.
    Identity,
    /// Translates the retained content by `(x, y)`.
    Translate { x: f32, y: f32 },
    /// Scales around `(origin_x, origin_y)`.
    Scale {
        sx: f32,
        sy: f32,
        origin_x: f32,
        origin_y: f32,
    },
    /// Rotates around `(origin_x, origin_y)` in radians.
    Rotate {
        radians: f32,
        origin_x: f32,
        origin_y: f32,
    },
}

/// One sampled frame of a compositor-safe animation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompositorAnimationFrame {
    /// The sampled controller value used for damage tracking.
    pub progress: f32,
    /// The transform to apply to the retained paint.
    pub transform: CompositorTransform,
    /// An opacity override, when the animation changes opacity.
    pub opacity: Option<f32>,
    /// A local rectangular clip, when the animation requires one.
    pub clip: Option<ResolvedSize>,
    /// Whether another frame must be requested after this one.
    pub active: bool,
    /// Whether all compositor properties are finite and safe to apply.
    pub valid: bool,
}

impl CompositorAnimationFrame {
    /// Creates a sampled compositor frame.
    #[inline]
    #[doc(hidden)]
    pub const fn new(
        progress: f32,
        transform: CompositorTransform,
        opacity: Option<f32>,
        clip: Option<ResolvedSize>,
        active: bool,
        valid: bool,
    ) -> Self {
        Self {
            progress,
            transform,
            opacity,
            clip,
            active,
            valid,
        }
    }
}

/// Describes whether a drawable can move its current animation to the
/// compositor while retaining its static paint commands.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompositorAnimationDecision {
    /// This drawable has no compositor animation provider.
    None,
    /// The sampled frame must be drawn live, but the provider already sampled
    /// it and can avoid ticking its controller a second time.
    Live(CompositorAnimationFrame),
    /// The sampled frame may use the retained paint path.
    Compositor(CompositorAnimationFrame),
}

/// The per-frame half of an element: advancing it and producing its pixels.
///
/// Drawing is split in two so that most frames do almost no work:
///
/// - [`update`](Drawable::update) is the **frame walk**. It lays children out,
///   advances animations, publishes interaction geometry and passes the walk to
///   the children that take part in the frame.
/// - The **retained paint** hooks ([`paint`](Drawable::paint) and the
///   geometry hooks around it) record what the element looks like once into
///   a render node. The
///   renderer replays that record on later frames and re-records it only when
///   the element reports that its paint changed, so an idle element costs
///   nothing per frame.
///
/// Only [`update`](Drawable::update) is commonly overridden by hand. An element
/// that has pixels of its own overrides [`can_paint`](Drawable::can_paint) and
/// [`paint`](Drawable::paint) together; the rest of the hooks describe geometry
/// around that record (clips, bounds, outsets, interaction rectangles) and are
/// hidden from the documentation because they are framework plumbing. Every
/// method has a default, so a layout-only element implements this trait with
/// an empty `impl`.
///
/// Methods here run on the UI thread during the frame, so they are on the
/// hottest path in the framework: they must not block or allocate per call.
pub trait Drawable {
    /// Walks this element for one frame.
    ///
    /// This is the per-frame traversal: lay out and position children, advance
    /// animations, rebuild dirty state, publish interaction geometry, and call
    /// `update` on every child that should take part in the frame. Visual
    /// output belongs to [`Self::paint`]: anything drawn on the canvas from
    /// `update` is not presented.
    ///
    /// The default does nothing.
    #[inline]
    fn update(&self, _ctx: &BuildContext) {}

    /// Returns whether this element can record its own paint for the current
    /// context.
    ///
    /// Returning `false` paints nothing for this element this frame (a list it
    /// recorded earlier is dropped, so stale pixels never stay on screen) and
    /// asks again next frame, for an element whose content is not ready yet,
    /// such as an image still loading. Its children are still walked. This
    /// check must be side-effect free. The default is `false`: an element
    /// without pixels of its own, such as a layout container.
    #[inline]
    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        false
    }

    /// Records this element's own paint into its render node.
    ///
    /// The record is replayed on later frames until the element reports a
    /// change, so this runs only when the paint is first needed or out of
    /// date. It must not draw children, which keep their own render nodes, and
    /// must stay paint-only: no layout, event, animation or external side
    /// effects. Open the list with `Canvas::of(ctx)`; it commits when finished
    /// or dropped. Called only while [`Self::can_paint`] returns `true`.
    #[inline]
    fn paint(&self, _ctx: &BuildContext) {}

    /// Brings the state and interaction geometry that [`Self::paint`] reads up
    /// to date, before the paint is replayed or recorded. It must not draw.
    ///
    /// Return `false` when the element cannot present a valid picture this
    /// frame; it then paints nothing, as for [`Self::can_paint`] returning
    /// `false`, and is asked again next frame. The default syncs the paint
    /// geometry and returns `true`.
    #[doc(hidden)]
    #[inline]
    fn sync_paint_state(&self, ctx: &BuildContext) -> bool {
        self.sync_paint_geometry(ctx);
        true
    }

    /// Walks this element for one frame after the render-tree walker has
    /// already done part of [`Self::update`]'s work.
    ///
    /// While a render tree presents the frame, the framework calls this
    /// instead of `update`, and only after it has run
    /// [`Self::sync_paint_state`] for the element, or sampled its
    /// [`Self::compositor_animation`]. The default runs `update`, which is
    /// right for any element whose `update` does not repeat that work.
    ///
    /// Override it only to leave out what the walker already did, so it is not
    /// done twice in one frame: an animation's controller tick and sample, or
    /// geometry and text state that `sync_paint_state` already brought up to
    /// date. Everything else `update` does (walking children, publishing
    /// interaction bounds, requesting the next frame) must still happen.
    #[doc(hidden)]
    #[inline]
    fn update_retained(&self, ctx: &BuildContext) {
        self.update(ctx);
    }

    /// Declares that this element is a transparent layer: it records no
    /// visible commands of its own, never has a compositor animation, and
    /// [`Self::paint_needs_recording`] is always `false`.
    ///
    /// The traversal then skips, for this element, the questions that exist
    /// only for elements that can paint or animate (the compositor sample and
    /// its clearing, and the re-record check). Geometry and state still sync and
    /// the children are still walked. The element keeps its own render node, so
    /// [`Self::can_paint`] **must** return `true` in every context: a wrapper
    /// forwarding geometry to this element relies on its own update to sync it
    /// once the element has a node, without asking about its paint source. The
    /// default is `false`; returning `true` from an element that paints, or that
    /// overrides [`Self::compositor_animation`], leaves its animation unapplied.
    #[doc(hidden)]
    #[inline]
    fn paints_nothing(&self) -> bool {
        false
    }

    /// Reports whether an already recorded paint must be recorded again.
    ///
    /// Called before a recorded paint is replayed. It is for paint values that
    /// change without a rebuild or a routed event, such as a clock-driven fade.
    /// The default keeps the recorded paint.
    #[doc(hidden)]
    #[inline]
    fn paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        false
    }

    /// Returns a rectangular clip owned by this retained render node.
    ///
    /// The clip applies to this node and its descendants. A wrapper that clips
    /// what it contains exposes the clip here instead of recording its whole
    /// subtree into its own paint.
    #[doc(hidden)]
    #[inline]
    fn retained_clip(&self, _ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd::Rect> {
        None
    }

    /// Overrides this node's render bounds while keeping its layout size.
    ///
    /// Containers whose paint is limited to a viewport can use this to keep
    /// their retained damage bounds independent of the larger content extent.
    #[doc(hidden)]
    #[inline]
    fn paint_bounds(&self, _ctx: &BuildContext) -> Option<ResolvedSize> {
        None
    }

    /// Adds local paint space around this node without changing its layout or
    /// hit-test bounds. Values are logical pixels in `[left, top, right,
    /// bottom]` order. Text shadows and glyph overhang use this to keep damage
    /// tracking around every painted pixel.
    #[doc(hidden)]
    #[inline]
    fn paint_outsets(&self, _ctx: &BuildContext) -> Option<[f32; 4]> {
        None
    }

    /// Opts this element into render-tree interaction bounds and reports the
    /// size of the rectangle it hit-tests against, in device pixels.
    ///
    /// An element that returns `Some` is handed a live reference to its render
    /// node through [`Self::adopt_interaction_source`] after every
    /// retained tree synchronization, so it no longer has to read the canvas
    /// transform stack while updating, and keeps following the node when a
    /// scroll moves it without a new synchronization. The size is separate from the node's own size because a
    /// box model widget lays its node out as the content box while pointers hit
    /// the border box (`computed_size`).
    #[doc(hidden)]
    #[inline]
    fn interaction_size(
        &self,
        _ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        None
    }

    /// Where the interaction rectangle starts relative to this element's render
    /// node, in device pixels (a margin, for instance). Zero by default.
    #[doc(hidden)]
    #[inline]
    fn interaction_offset(&self, _ctx: &BuildContext) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Receives the live render-tree source of this element's interaction
    /// rectangle, after it reported an interaction size.
    #[doc(hidden)]
    #[inline]
    fn adopt_interaction_source(
        &self,
        _source: crate::components::interaction_bounds::InteractionSource,
    ) {
    }

    /// Reports how the render-tree interaction rectangle differs from the one
    /// measured through the canvas transform, when they differ by more than a
    /// pixel. Diagnostic: tests use it before the canvas measurement retires.
    #[doc(hidden)]
    #[inline]
    fn interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        None
    }

    /// Supplies a specialized context for one direct child during retained
    /// tree synchronization. Scroll viewports use it to preserve their
    /// unbounded scroll-axis constraint and visible window for virtualization.
    #[doc(hidden)]
    #[inline]
    fn child_context<'a>(
        &self,
        _ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        None
    }

    /// Supplies a child context when its visitation ordinal affects layout.
    ///
    /// The default preserves [`Self::child_context`]. Indexed
    /// containers such as flex layouts can use `child_index` to recover a
    /// materialized child's exact slot without searching the full data source.
    #[doc(hidden)]
    #[inline]
    fn child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.child_context(ctx, child)
    }

    /// Overrides one direct child's render bounds and clip during tree sync.
    ///
    /// The returned rectangle is local to `self`. This lets a viewport keep a
    /// moving child clipped to its fixed bounds in the same geometry update,
    /// so synchronization damages the old and new visible footprints only.
    #[doc(hidden)]
    #[inline]
    fn child_geometry(
        &self,
        _ctx: &BuildContext,
        _child: &dyn Element,
    ) -> Option<(aimer_cupid::draw_cmd::Rect, Option<aimer_cupid::draw_cmd::Rect>)> {
        None
    }

    /// Overrides one direct child's geometry when its visitation ordinal
    /// affects layout. The default preserves [`Self::child_geometry`].
    #[doc(hidden)]
    #[inline]
    fn child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<(aimer_cupid::draw_cmd::Rect, Option<aimer_cupid::draw_cmd::Rect>)> {
        self.child_geometry(ctx, child)
    }

    /// Returns the corner radii of the clip this element applies to one direct
    /// child, in logical pixels: top-left, top-right, bottom-right,
    /// bottom-left.
    ///
    /// The radii round the clip rectangle returned by
    /// [`Self::child_geometry`] (or [`Self::retained_clip`]). The
    /// renderer keeps one clip at a time and takes its radii from the
    /// innermost clip, so this must match what the live `draw` path pushes for
    /// the same child. The default is square corners.
    #[doc(hidden)]
    #[inline]
    fn child_clip_radius(&self, _ctx: &BuildContext, _child: &dyn Element) -> [f32; 4] {
        [0.0; 4]
    }

    /// Synchronizes live geometry needed by interaction and hit testing before
    /// a retained paint replay.
    ///
    /// This hook is deliberately separate from [`Self::paint`]. A cached
    /// visual subtree must still publish current bounds and layout-derived
    /// interaction state even when no descendant paint commands are emitted.
    /// The default is a no-op for leaves whose geometry is already available.
    #[doc(hidden)]
    #[inline]
    fn sync_paint_geometry(&self, _ctx: &BuildContext) {}

    /// Forwards [`Self::sync_paint_geometry`] to this element, a child of the
    /// wrapper calling it, unless the child is about to do the same work itself.
    ///
    /// A wrapper whose `update` always updates its child should forward through
    /// this instead of calling `sync_paint_geometry` on the child. The default
    /// forwards unconditionally; the retained element node skips the forward
    /// when the child's update is promised and the child will sync its own
    /// geometry there.
    #[doc(hidden)]
    #[inline]
    fn forward_paint_geometry(&self, ctx: &BuildContext) {
        self.sync_paint_geometry(ctx);
    }

    /// Gives layout-aware containers a chance to reconcile viewport-dependent
    /// geometry before the visible paint pass. The return value is `true` when
    /// the element has made its content extent authoritative for this context;
    /// callers may then update an end-anchored scroll position before painting.
    /// The default is intentionally a no-op: ordinary leaves have no layout
    /// work that can be prepared without painting.
    #[doc(hidden)]
    #[inline]
    fn prepare_layout(&self, _ctx: &BuildContext) -> bool {
        false
    }

    /// Samples the compositor animation for the current frame.
    #[doc(hidden)]
    #[inline]
    fn compositor_animation(&self, _ctx: &BuildContext) -> CompositorAnimationDecision {
        CompositorAnimationDecision::None
    }

}

impl Drawable for Box<dyn Drawable> {
    #[inline]
    fn update(&self, ctx: &BuildContext) {
        self.as_ref().update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.as_ref().sync_paint_geometry(ctx);
    }

    #[inline]
    fn forward_paint_geometry(&self, ctx: &BuildContext) {
        self.as_ref().forward_paint_geometry(ctx);
    }

    #[inline]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.as_ref().prepare_layout(ctx)
    }

    #[inline]
    fn paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.as_ref().paint_outsets(ctx)
    }

    #[inline]
    fn interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        self.as_ref().interaction_size(ctx)
    }

    #[inline]
    fn interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.as_ref().interaction_offset(ctx)
    }

    #[inline]
    fn adopt_interaction_source(
        &self,
        source: crate::components::interaction_bounds::InteractionSource,
    ) {
        self.as_ref().adopt_interaction_source(source)
    }

    #[inline]
    fn interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        self.as_ref().interaction_disagreement()
    }

    #[inline]
    fn child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        self.as_ref().child_clip_radius(ctx, child)
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        self.as_ref().compositor_animation(ctx)
    }

}
