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

pub trait Drawable {
    /// Walks this element for one frame.
    ///
    /// This is the per-frame traversal: lay out and position children, advance
    /// animations, rebuild dirty state, publish interaction geometry, and call
    /// `update` on every child that should take part in the frame. Visual
    /// output belongs to [`Self::paint_local_v2`]. While the legacy canvas path
    /// still exists an element's `update` may also issue legacy paint commands,
    /// which the retained presentation suppresses.
    ///
    /// The default does nothing.
    #[inline]
    fn update(&self, _ctx: &BuildContext) {}

    /// Returns whether this element can record its own visual commands into a
    /// retained v2 list for the current context. Returning `false` keeps the
    /// complete subtree on the legacy paint path during migration. This check
    /// should be side-effect-free. The default opts existing elements out.
    #[doc(hidden)]
    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        false
    }

    /// Records this element's own visual commands into the retained v2 list.
    ///
    /// This hook must not draw or record children; child elements keep their
    /// own render nodes. It must remain paint-only and avoid layout, event,
    /// animation, or external side effects. Direct retained-presentation
    /// frames use this list for the node's pixels; legacy islands keep using
    /// the frame-wide command stream. Open the list with `Canvas::of(ctx)`; it
    /// commits when explicitly finished or dropped.
    #[doc(hidden)]
    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Updates state and interaction geometry needed by the current local v2
    /// paint before that list is recorded. This hook must not emit visual
    /// commands. Return `false` when the element needs one legacy frame to
    /// settle state that depends on its live draw path. State updates performed
    /// before returning `false` must be safe for the fallback draw to observe.
    #[doc(hidden)]
    #[inline]
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.sync_paint_geometry(ctx);
        true
    }

    /// Runs the compatibility traversal after local v2 paint is recorded.
    ///
    /// The default preserves existing draw-time behavior and visits children.
    /// A leaf may override this when `sync_local_v2_state` has already done all
    /// of its required work and it has no children to traverse.
    #[doc(hidden)]
    #[inline]
    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.update(ctx);
    }

    /// Reports whether an already recorded local v2 list must be refreshed.
    ///
    /// The retained walker calls this for local-v2 nodes before replay. It is
    /// for paint values that can change without a rebuild or routed event,
    /// such as a clock-driven fade. The default keeps an unchanged list cached.
    #[doc(hidden)]
    #[inline]
    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        false
    }

    /// Returns a rectangular clip owned by this retained render node.
    ///
    /// The clip applies to this node and its descendants. Paint-affecting
    /// wrappers can expose clips here so v2 child lists preserve their legacy
    /// clipping behavior without recording the wrapper's entire subtree.
    #[doc(hidden)]
    #[inline]
    fn retained_clip(&self, _ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        None
    }

    /// Overrides this node's v2 render bounds while keeping its layout size.
    ///
    /// Containers whose paint is limited to a viewport can use this to keep
    /// their retained damage bounds independent of the larger content extent.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_bounds(&self, _ctx: &BuildContext) -> Option<ResolvedSize> {
        None
    }

    /// Adds local paint space around this node without changing its layout or
    /// hit-test bounds. Values are logical pixels in `[left, top, right,
    /// bottom]` order. Text shadows and glyph overhang use this to keep damage
    /// tracking around every painted pixel.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_paint_outsets(&self, _ctx: &BuildContext) -> Option<[f32; 4]> {
        None
    }

    /// Opts this element into render-tree interaction bounds and reports the
    /// size of the rectangle it hit-tests against, in device pixels.
    ///
    /// An element that returns `Some` is handed a live reference to its render
    /// node through [`Self::adopt_retained_v2_interaction_source`] after every
    /// retained tree synchronization, so it no longer has to read the canvas
    /// transform stack while updating, and keeps following the node when a
    /// scroll moves it without a new synchronization. The size is separate from the node's own size because a
    /// box model widget lays its node out as the content box while pointers hit
    /// the border box (`computed_size`).
    #[doc(hidden)]
    #[inline]
    fn retained_v2_interaction_size(
        &self,
        _ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        None
    }

    /// Where the interaction rectangle starts relative to this element's render
    /// node, in device pixels (a margin, for instance). Zero by default.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_interaction_offset(&self, _ctx: &BuildContext) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// Receives the live render-tree source of this element's interaction
    /// rectangle, after it reported an interaction size.
    #[doc(hidden)]
    #[inline]
    fn adopt_retained_v2_interaction_source(
        &self,
        _source: crate::components::interaction_bounds::InteractionSource,
    ) {
    }

    /// Reports how the render-tree rectangle currently differs from the last one
    /// measured through the legacy canvas transform, if it does by more than a
    /// pixel. Diagnostic: it gates retiring the canvas measurement.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        None
    }

    /// Supplies a specialized context for one direct child during retained
    /// tree synchronization. Scroll viewports use it to preserve their
    /// unbounded scroll-axis constraint and visible window for virtualization.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_child_context<'a>(
        &self,
        _ctx: &BuildContext<'a>,
        _child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        None
    }

    /// Supplies a child context when its visitation ordinal affects layout.
    ///
    /// The default preserves [`Self::retained_v2_child_context`]. Indexed
    /// containers such as flex layouts can use `child_index` to recover a
    /// materialized child's exact slot without searching the full data source.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.retained_v2_child_context(ctx, child)
    }

    /// Overrides one direct child's v2 bounds and clip during tree sync.
    ///
    /// The returned rectangle is local to `self`. This lets a viewport keep a
    /// moving child clipped to its fixed bounds in the same geometry update,
    /// so synchronization damages the old and new visible footprints only.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_child_geometry(
        &self,
        _ctx: &BuildContext,
        _child: &dyn Element,
    ) -> Option<(aimer_cupid::draw_cmd_v2::Rect, Option<aimer_cupid::draw_cmd_v2::Rect>)> {
        None
    }

    /// Overrides one direct child's geometry when its visitation ordinal
    /// affects layout. The default preserves [`Self::retained_v2_child_geometry`].
    #[doc(hidden)]
    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<(aimer_cupid::draw_cmd_v2::Rect, Option<aimer_cupid::draw_cmd_v2::Rect>)> {
        self.retained_v2_child_geometry(ctx, child)
    }

    /// Returns the corner radii of the clip this element applies to one direct
    /// child, in logical pixels: top-left, top-right, bottom-right,
    /// bottom-left.
    ///
    /// The radii round the clip rectangle returned by
    /// [`Self::retained_v2_child_geometry`] (or [`Self::retained_clip`]). The
    /// renderer keeps one clip at a time and takes its radii from the
    /// innermost clip, so this must match what the live `draw` path pushes for
    /// the same child. The default is square corners.
    #[doc(hidden)]
    #[inline]
    fn retained_v2_child_clip_radius(&self, _ctx: &BuildContext, _child: &dyn Element) -> [f32; 4] {
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
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.as_ref().prepare_layout(ctx)
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.as_ref().retained_v2_paint_outsets(ctx)
    }

    #[inline]
    fn retained_v2_interaction_size(
        &self,
        ctx: &BuildContext,
    ) -> Option<aimer_attribute::size::ResolvedSize> {
        self.as_ref().retained_v2_interaction_size(ctx)
    }

    #[inline]
    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.as_ref().retained_v2_interaction_offset(ctx)
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(
        &self,
        source: crate::components::interaction_bounds::InteractionSource,
    ) {
        self.as_ref().adopt_retained_v2_interaction_source(source)
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<crate::components::interaction_bounds::InteractionDisagreement> {
        self.as_ref().retained_v2_interaction_disagreement()
    }

    #[inline]
    fn retained_v2_child_clip_radius(&self, ctx: &BuildContext, child: &dyn Element) -> [f32; 4] {
        self.as_ref().retained_v2_child_clip_radius(ctx, child)
    }

    #[inline]
    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        self.as_ref().compositor_animation(ctx)
    }

}
