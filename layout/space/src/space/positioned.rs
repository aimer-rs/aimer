use aimer_attribute::BoxConstraint;
use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_container::ZeroSizedBox;
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, ResolvedSize};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, Rebuildable,
    VisitorElement, Widget,
};

use super::hug::hugged_size;

#[allow(dead_code)]
/// Positions and transforms one child relative to its parent, typically a
/// [`crate::Stack`].
///
/// Attach a child with [`Positioned::child`] to retain its concrete type, or
/// with [`Positioned::box_child`] when branches need a shared erased type.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_space::space::Positioned")]
pub struct Positioned<W: Widget + 'static = ZeroSizedBox> {
    #[portable_child]
    pub child: W,
    #[portable_skip]
    pub position: Position,
    #[portable_optional]
    pub left: Dimension,
    #[portable_optional]
    pub top: Dimension,
    #[portable_optional]
    pub right: Dimension,
    #[portable_optional]
    pub bottom: Dimension,
    #[portable_skip]
    pub transform: Transform,
    pub layer: u32,
}

impl Default for Positioned {
    fn default() -> Self {
        Self::new()
    }
}

impl Positioned {
    /// Creates a relative, untransformed positioned widget on layer zero.
    ///
    /// The placeholder is already a valid widget; use [`Positioned::child`] or
    /// [`Positioned::box_child`] to attach content.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: ZeroSizedBox,
            position: Position::default(),
            left: Dimension::default(),
            top: Dimension::default(),
            right: Dimension::default(),
            bottom: Dimension::default(),
            transform: Transform::default(),
            layer: 0,
        }
    }
}

impl<W: Widget + 'static> Positioned<W> {
    // #[inline] pub fn box() -> Box<Self> {
    //     Box::new(Self::new())
    // }

    /// Records the positioning mode associated with this widget.
    ///
    /// The default is [`Position::Relative`]. The current renderer stores this
    /// value for inspection but resolves edge offsets identically for every
    /// [`Position`] variant.
    #[inline]
    pub fn position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    /// Sets the logical left inset or offset.
    ///
    /// The default is [`Dimension::Auto`]. Pixel values are logical pixels and
    /// percentage values resolve against the parent's width. If both left and
    /// right are specified, left takes precedence for painting.
    #[inline]
    pub fn left(mut self, left: impl Into<Dimension>) -> Self {
        self.left = left.into();
        self
    }

    /// Sets the logical top inset or offset.
    ///
    /// The default is [`Dimension::Auto`]. Pixel values are logical pixels and
    /// percentage values resolve against the parent's height. If both top and
    /// bottom are specified, top takes precedence for painting.
    #[inline]
    pub fn top(mut self, top: impl Into<Dimension>) -> Self {
        self.top = top.into();
        self
    }

    /// Sets the logical right inset.
    ///
    /// The default is [`Dimension::Auto`]. Pixel values are logical pixels and
    /// percentage values resolve against the parent's width. A right-only inset
    /// positions the child from the parent's right edge.
    #[inline]
    pub fn right(mut self, right: impl Into<Dimension>) -> Self {
        self.right = right.into();
        self
    }

    /// Sets the logical bottom inset.
    ///
    /// The default is [`Dimension::Auto`]. Pixel values are logical pixels and
    /// percentage values resolve against the parent's height. A bottom-only
    /// inset positions the child from the parent's bottom edge.
    #[inline]
    pub fn bottom(mut self, bottom: impl Into<Dimension>) -> Self {
        self.bottom = bottom.into();
        self
    }

    /// Replaces the additional paint transform.
    ///
    /// The default is [`Transform::None`]. Translation values use logical
    /// pixels, rotation uses radians, and scale values are dimensionless.
    /// The transform affects painting rather than the child's measured
    /// layout size.
    #[inline]
    pub fn transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// Sets the z-order layer used by layered parents such as [`crate::Stack`].
    ///
    /// The default is `0`; higher layers paint later in a normal-direction
    /// stack.
    #[inline]
    pub fn layer(mut self, layer: u32) -> Self {
        self.layer = layer;
        self
    }

    /// Attaches or replaces the child while preserving positioning settings.
    ///
    /// `Positioned::new()` is already valid with a zero-sized placeholder. This
    /// operation preserves the new child's concrete type; use
    /// [`Positioned::box_child`] for branch type erasure.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> Positioned<C> {
        Positioned {
            child,
            position: self.position,
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
            transform: self.transform,
            layer: self.layer,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Positioned::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget> Widget for Positioned<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        RawPositionedElement {
            child,
            position: self.position,
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom,
            transform: self.transform,
            layer: self.layer,
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Copy)]
pub enum Transform {
    Translate(f32, f32),
    TranslateX(f32),
    TranslateY(f32),
    Scale(f32, f32),
    ScaleX(f32),
    ScaleY(f32),
    // Matrix(Vec<f32>),
    Rotate(f32), // radians
    #[default]
    None,
}

#[derive(Debug, Copy, Clone, PartialEq, Default)]
pub enum Position {
    Absolute,
    #[default]
    Relative,
    Static,
    Sticky,
    Fixed,
}

#[allow(dead_code)]
pub struct RawPositionedElement<E: Element> {
    pub(crate) child: E,
    pub(crate) position: Position,
    pub(crate) left: Dimension,
    pub(crate) top: Dimension,
    pub(crate) right: Dimension,
    pub(crate) bottom: Dimension,
    pub(crate) transform: Transform,
    pub(crate) layer: u32,
    pub(crate) bounds: InteractionBounds,
}

impl<E: Element> RawPositionedElement<E> {
    /// Creates a low-level positioned element with default positioning values.
    #[doc(hidden)]
    #[inline]
    pub fn new(child: E) -> Self {
        Self {
            child,
            position: Default::default(),
            left: Dimension::Auto,
            top: Dimension::Auto,
            right: Dimension::Auto,
            bottom: Dimension::Auto,
            transform: Default::default(),
            layer: 0,
            bounds: InteractionBounds::new(),
        }
    }

    /// Sets the left inset for this low-level element.
    #[doc(hidden)]
    #[inline]
    pub fn left(mut self, left: impl Into<Dimension>) -> Self {
        self.left = left.into();
        self
    }

    /// Sets the top inset for this low-level element.
    #[doc(hidden)]
    #[inline]
    pub fn top(mut self, top: impl Into<Dimension>) -> Self {
        self.top = top.into();
        self
    }

    /// Whether no edge offset is set, in which case the child is laid out in the
    /// parent's own space rather than placed within it.
    #[inline]
    fn has_no_edges(&self) -> bool {
        self.top == Dimension::Auto
            && self.left == Dimension::Auto
            && self.right == Dimension::Auto
            && self.bottom == Dimension::Auto
    }

    /// The size the child occupies once placed.
    ///
    /// A child placed by edge offsets hugs its content, so `bottom` and `right`
    /// are measured from the edge of the child that is really there; a child that
    /// is not placed fills the constraint as it always did.
    #[inline]
    fn placed_child_size(&self, ctx: &BuildContext) -> ResolvedSize {
        if self.has_no_edges() {
            self.child.computed_size(ctx)
        } else {
            hugged_size(&self.child, ctx)
        }
    }

    /// The child node's own scale or rotation, about the child's origin. `None`
    /// for translation-only transforms, which move the child instead.
    fn child_presentation(&self) -> Option<aimer_cupid::utilities::Mat3> {
        match self.transform {
            Transform::Scale(sx, sy) => Some(aimer_cupid::utilities::Mat3::scale(sx, sy)),
            Transform::ScaleX(sx) => Some(aimer_cupid::utilities::Mat3::scale(sx, 1.0)),
            Transform::ScaleY(sy) => Some(aimer_cupid::utilities::Mat3::scale(1.0, sy)),
            Transform::Rotate(radians) => Some(aimer_cupid::utilities::Mat3::rotate(radians)),
            Transform::Translate(..)
            | Transform::TranslateX(..)
            | Transform::TranslateY(..)
            | Transform::None => None,
        }
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> Option<(Vec2d, BuildContext<'a>)> {
        if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
            return None;
        }
        // Scale and rotation do not move the child's origin; they are the child
        // node's presentation (see `sync_local_v2_state`).
        let (transform_x, transform_y) = match self.transform {
            Transform::Translate(x, y) => (x, y),
            Transform::TranslateX(x) => (x, 0.0),
            Transform::TranslateY(y) => (0.0, y),
            Transform::None
            | Transform::Scale(..)
            | Transform::ScaleX(..)
            | Transform::ScaleY(..)
            | Transform::Rotate(..) => (0.0, 0.0),
        };
        let is_auto = self.top == Dimension::Auto
            && self.left == Dimension::Auto
            && self.right == Dimension::Auto
            && self.bottom == Dimension::Auto;
        // Edge offsets apply to the complete child box, including its padding,
        // border, and margin, rather than only the child's inner content area.
        let child_size = self.placed_child_size(ctx);
        if !child_size.width.is_finite()
            || !child_size.height.is_finite()
            || child_size.width < 0.0
            || child_size.height < 0.0
            || child_size.width > 1_000_000.0
            || child_size.height > 1_000_000.0
            || !transform_x.is_finite()
            || !transform_y.is_finite()
        {
            return None;
        }

        let mut offset_x = 0.0;
        let mut offset_y = 0.0;
        if !is_auto {
            if !ctx.parent_size.width.is_finite() || !ctx.parent_size.height.is_finite() {
                return None;
            }
            if self.left != Dimension::Auto {
                offset_x = self.left.resolve(ctx.parent_size.width, ctx.scale);
            } else if self.right != Dimension::Auto {
                offset_x = ctx.parent_size.width
                    - self.right.resolve(ctx.parent_size.width, ctx.scale)
                    - child_size.width;
            }
            if self.top != Dimension::Auto {
                offset_y = self.top.resolve(ctx.parent_size.height, ctx.scale);
            } else if self.bottom != Dimension::Auto {
                offset_y = ctx.parent_size.height
                    - self.bottom.resolve(ctx.parent_size.height, ctx.scale)
                    - child_size.height;
            }
        }
        let offset = Vec2d {
            x: offset_x + transform_x,
            y: offset_y + transform_y,
        };
        if !offset.x.is_finite()
            || !offset.y.is_finite()
            || offset.x.abs() > 1_000_000.0
            || offset.y.abs() > 1_000_000.0
        {
            return None;
        }

        let mut child_ctx = ctx.clone();
        child_ctx.visible_rect =
            shift_visible_rect(ctx.visible_rect, offset_x, offset_y, &self.transform);
        if !is_auto {
            child_ctx.parent_size = child_size;
            child_ctx.box_constraint = BoxConstraint {
                min_width: 0.0,
                min_height: 0.0,
                max_width: child_size.width,
                max_height: child_size.height,
            };
        }
        Some((offset, child_ctx))
    }
}

impl<E: Element> Drawable for RawPositionedElement<E> {
    /// Hit-tested over the positioned child, which starts `offset` in from the
    /// node. A transform the layout cannot express has no hit area.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        if self.child_presentation().is_some() {
            return None;
        }
        let (_, child_ctx) = self.retained_child_layout(ctx)?;
        Some(self.placed_child_size(&child_ctx))
    }

    fn retained_v2_interaction_offset(&self, ctx: &BuildContext) -> (f32, f32) {
        self.retained_child_layout(ctx)
            .map_or((0.0, 0.0), |(offset, _)| (offset.x, offset.y))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("Positioned")
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let Some((_, child_ctx)) = self.retained_child_layout(ctx) else {
            return false;
        };
        let size = self
            .child
            .retained_v2_bounds(&child_ctx)
            .unwrap_or_else(|| self.child.content_size(&child_ctx));
        let position = self.child.pos().unwrap_or_default();
        size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
            && size.width <= 1_000_000.0
            && size.height <= 1_000_000.0
            && position.x.is_finite()
            && position.y.is_finite()
            && position.x.abs() <= 1_000_000.0
            && position.y.abs() <= 1_000_000.0
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.sync_paint_geometry(ctx);
        match self.child_presentation() {
            Some(transform) => ctx.set_local_v2_child_presentation_at(0, transform, 1.0),
            None => true,
        }
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        _child: &dyn Element,
        _child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.retained_child_layout(ctx)
            .map(|(_, child_ctx)| child_ctx)
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        let (offset, child_ctx) = self.retained_child_layout(ctx)?;
        let position = child.pos().unwrap_or_default();
        let size = child
            .retained_v2_bounds(&child_ctx)
            .unwrap_or_else(|| child.content_size(&child_ctx));
        let bounds = aimer_cupid::draw_cmd_v2::Rect::new(
            (offset.x + position.x) / ctx.scale,
            (offset.y + position.y) / ctx.scale,
            size.width / ctx.scale,
            size.height / ctx.scale,
        );
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
            || bounds.width < 0.0
            || bounds.height < 0.0
        {
            return None;
        }
        Some((bounds, child.retained_clip(&child_ctx)))
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let Some((offset, child_ctx)) = self.retained_child_layout(ctx) else {
            self.bounds.clear_canvas();
            return;
        };
        let size = self.placed_child_size(&child_ctx);
        let (start_x, start_y) = ctx.canvas.get_transform_translation();
        self.bounds.save(
            ctx.scale,
            start_x + offset.x,
            start_y + offset.y,
            size.width,
            size.height,
        );
        ctx.canvas.save();
        ctx.canvas.translate(offset);
        self.child.sync_paint_geometry(&child_ctx);
        ctx.canvas.restore();
    }

    fn update(&self, ctx: &BuildContext) {
        // A position or non-identity transform can change the wrapper's
        // screen rectangle. Unknown transforms stay uncached so bounded
        // pointer descent never under-culls a child.
        self.bounds.clear_canvas();
        // debug!("Positioned::draw");
        let is_auto = self.top == Dimension::Auto
            && self.left == Dimension::Auto
            && self.right == Dimension::Auto
            && self.bottom == Dimension::Auto;

        if is_auto && self.transform == Transform::None {
            self.child.update(ctx);
            return;
        }

        ctx.canvas.save();

        let mut offset_x = 0.0;
        let mut offset_y = 0.0;
        let child_size = self.placed_child_size(ctx);

        if !is_auto {
            if self.left != Dimension::Auto {
                offset_x = self.left.resolve(ctx.parent_size.width, ctx.scale);
            } else if self.right != Dimension::Auto {
                offset_x = ctx.parent_size.width
                    - self.right.resolve(ctx.parent_size.width, ctx.scale)
                    - child_size.width;
            }

            if self.top != Dimension::Auto {
                offset_y = self.top.resolve(ctx.parent_size.height, ctx.scale);
            } else if self.bottom != Dimension::Auto {
                offset_y = ctx.parent_size.height
                    - self.bottom.resolve(ctx.parent_size.height, ctx.scale)
                    - child_size.height;
            }
        }

        if let Some((transform_x, transform_y)) = match self.transform {
            Transform::Translate(tx, ty) => Some((tx, ty)),
            Transform::TranslateX(tx) => Some((tx, 0.0)),
            Transform::TranslateY(ty) => Some((0.0, ty)),
            Transform::None => Some((0.0, 0.0)),
            Transform::Scale(..)
            | Transform::ScaleX(..)
            | Transform::ScaleY(..)
            | Transform::Rotate(..) => None,
        } {
            let (start_x, start_y) = ctx.canvas.get_transform_translation();
            self.bounds.save(
                ctx.scale,
                start_x + offset_x + transform_x,
                start_y + offset_y + transform_y,
                child_size.width,
                child_size.height,
            );
        }

        ctx.canvas.translate(Vec2d {
            x: offset_x,
            y: offset_y,
        });

        match &self.transform {
            Transform::Translate(tx, ty) => {
                ctx.canvas.translate(Vec2d { x: *tx, y: *ty });
            }
            Transform::TranslateX(tx) => {
                ctx.canvas.translate(Vec2d { x: *tx, y: 0.0 });
            }
            Transform::TranslateY(ty) => {
                ctx.canvas.translate(Vec2d { x: 0.0, y: *ty });
            }
            Transform::Scale(sx, sy) => {
                ctx.canvas.scale(*sx, *sy);
            }
            Transform::ScaleX(sx) => {
                ctx.canvas.scale(*sx, 1.0);
            }
            Transform::ScaleY(sy) => {
                ctx.canvas.scale(1.0, *sy);
            }
            Transform::Rotate(rad) => {
                ctx.canvas.rotate(*rad);
            }
            Transform::None => {}
        }

        if is_auto {
            self.child.update(ctx);
        } else {
            let parent_pos = ctx.parent_pos;

            let parent_size = child_size;

            let child_constraint = BoxConstraint {
                min_width: 0.0,
                min_height: 0.0,
                max_width: parent_size.width,
                max_height: parent_size.height,
            };

            // The child is drawn after translating the canvas by the position
            // offset (and any translation transform), so the visibility rect (used
            // for scroll culling) must be shifted by the same amount. Otherwise, a
            // positioned block's top children (e.g. a title above a body) are
            // culled too early and disappear while the taller body survives.
            let child_visible_rect =
                shift_visible_rect(ctx.visible_rect, offset_x, offset_y, &self.transform);

            let child_ctx = BuildContext {
                parent_size,
                canvas: ctx.canvas.clone(),
                scale: ctx.scale,
                parent_pos,
                cursor_pos: ctx.cursor_pos,
                box_constraint: child_constraint,
                visible_rect: child_visible_rect,
                paint_background: ctx.paint_background,
                window: ctx.window.clone(),
                #[cfg(not(target_arch = "wasm32"))]
                async_handle: ctx.async_handle.clone(),
                inherited_states: ctx.inherited_states.clone(),
            };

            self.child.update(&child_ctx);
        }
        ctx.canvas.restore();
    }
}

/// Shift the scroll-culling `visible_rect` into the positioned child's local
/// coordinate space, i.e. by the same offset the canvas was translated:
/// the position offset plus any translate transform. Scale/Rotate transforms
/// leave the rect unchanged (culling stays conservative — never over-culls).
fn shift_visible_rect(
    visible_rect: Option<(f32, f32, f32, f32)>,
    offset_x: f32,
    offset_y: f32,
    transform: &Transform,
) -> Option<(f32, f32, f32, f32)> {
    let (t_tx, t_ty) = match transform {
        Transform::Translate(tx, ty) => (*tx, *ty),
        Transform::TranslateX(tx) => (*tx, 0.0),
        Transform::TranslateY(ty) => (0.0, *ty),
        _ => (0.0, 0.0),
    };
    let shift_x = offset_x + t_tx;
    let shift_y = offset_y + t_ty;
    visible_rect.map(|(vx, vy, vw, vh)| (vx - shift_x, vy - shift_y, vw, vh))
}

impl<E: Element> VisitorElement for RawPositionedElement<E> {
    fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {
        // Positioned handles its own child rendering in draw() with proper
        // offset, so we don't expose children here to avoid
        // double-rendering at (0,0).
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        visitor(0, &self.child);
    }

    fn debug_name(&self) -> &'static str {
        "RawPositionedElement"
    }
}

impl<E: Element> EventElement for RawPositionedElement<E> {
    /// Positioned hides its child from visual traversal because it renders the
    /// child itself, but the event view is the structural child view.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }
}

impl<E: Element> LayoutElement for RawPositionedElement<E> {
    fn layer(&self) -> u32 {
        self.layer
    }

    #[inline]
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

// `visit_children` is intentionally empty so `Drawable::update` doesn't double-
// render the child. The default `Rebuildable::mark_needs_rebuild` and
// `rebuild_if_dirty` both walk `visit_children`, which means in a
// `Stack → Positioned → Scrollable → Stateful` chain a resize cascade would
// stop at the `Positioned` and never reach the inner `StatefulElement` — the
// `dirty=true` flag set by `adopt_state_from` would sit unused, and the
// rebuilt tree from the adopted `rebuild_fn` (with restored state) would
// never be produced. Visual symptom: stateful child "snaps back" on resize.
impl<E: Element + 'static> Rebuildable for RawPositionedElement<E> {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        // eprintln!("[diag] Positioned.rebuild_if_dirty -> child");
        self.child.rebuild_if_dirty(ctx);
    }

    fn mark_needs_rebuild(&self) {
        // eprintln!("[diag] Positioned.mark_needs_rebuild -> child");
        self.child.mark_needs_rebuild();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Cull test used by the flex layout: a child at local `[y, y+h)` is drawn
    /// only if it overlaps the visible window `[vy, vy+vh)`.
    fn visible(child_y: f32, child_h: f32, rect: (f32, f32, f32, f32)) -> bool {
        let (_, vy, _, vh) = rect;
        !(child_y + child_h < vy || child_y > vy + vh)
    }

    // Reproduces the reported bug: a feature block positioned 110px down inside a
    // 500px stack, with a short title (0..30) above a tall body (40..300). The
    // scrollable's viewport shows y 200..800 of the stack.
    #[test]
    fn positioned_offset_keeps_title_visible() {
        let stack_visible = (0.0, 200.0, 1000.0, 600.0);

        // Before the fix: visible_rect forwarded unchanged. In block-local space
        // the title (0..30) is treated as far above the viewport and culled,
        // while the taller body straddles the line and survives — exactly the
        // "title clipped, body left" symptom.
        let unshifted = stack_visible;
        assert!(
            !visible(0.0, 30.0, unshifted),
            "buggy path should cull the title"
        );
        assert!(visible(40.0, 260.0, unshifted), "buggy path keeps the body");

        // After the fix: shift by the 110px top offset. The title's real position
        // is stack y 110..140, still above the viewport (200), so culling it is
        // now *correct*; both title and body are judged in the same, correct space.
        let shifted =
            shift_visible_rect(Some(stack_visible), 0.0, 110.0, &Transform::None).unwrap();
        assert_eq!(shifted, (0.0, 90.0, 1000.0, 600.0));

        // A block near the top of the stack (offset 10px) whose title IS on screen
        // must keep its title after the fix, where the unshifted path wrongly culls it.
        let top_block =
            shift_visible_rect(Some((0.0, 5.0, 1000.0, 600.0)), 0.0, 10.0, &Transform::None)
                .unwrap();
        assert!(
            visible(0.0, 30.0, top_block),
            "on-screen title must not be culled"
        );
    }

    #[test]
    fn shift_includes_translate_transform() {
        let r = shift_visible_rect(
            Some((0.0, 100.0, 10.0, 10.0)),
            5.0,
            20.0,
            &Transform::Translate(1.0, 2.0),
        )
        .unwrap();
        assert_eq!(r, (-6.0, 78.0, 10.0, 10.0));
        // Scale/Rotate leave the rect untouched (conservative — never over-cull).
        let s = shift_visible_rect(
            Some((0.0, 100.0, 10.0, 10.0)),
            0.0,
            0.0,
            &Transform::Scale(2.0, 2.0),
        )
        .unwrap();
        assert_eq!(s, (0.0, 100.0, 10.0, 10.0));
    }

    /// Regression for the `Positioned → Scrollable → Stateful` resize bug:
    /// `RawPositionedElement::visit_children` is empty (intentional, to avoid
    /// double-render), so the default `Rebuildable::mark_needs_rebuild` /
    /// `rebuild_if_dirty` walk `visit_children` and stop at `Positioned`. The
    /// inner `StatefulElement` then never receives the resize cascade and the
    /// `dirty=true` flag set by `adopt_state_from` is never consumed. This
    /// test pins the structural trap and the divergence between the two child
    /// accessors — the actual rebuild behavior is covered by the framework
    /// reconcile tests in `aimer_widget`.
    #[test]
    fn positioned_propagates_dirty_into_child() {
        use aimer_container::ZeroSizedBox;
        let positioned: RawPositionedElement<ZeroSizedBox> = RawPositionedElement {
            child: ZeroSizedBox,
            position: Default::default(),
            left: Default::default(),
            top: Default::default(),
            right: Default::default(),
            bottom: Default::default(),
            transform: Default::default(),
            layer: 0,
            bounds: InteractionBounds::new(),
        };

        // `visit_children` is intentionally empty (no double-render).
        let mut visit_count = 0;
        positioned.visit_children(&mut |_| visit_count += 1);
        assert_eq!(
            visit_count, 0,
            "visit_children must stay empty so draw() doesn't double-render"
        );

        // `event_children` DOES surface the wrapped child — events still need
        // to reach it. The Rebuildable fix above must NOT change this.
        let mut event_count = 0;
        positioned.event_children(&mut |_| event_count += 1);
        assert_eq!(
            event_count, 1,
            "event_children must keep surfacing the wrapped child"
        );
    }
}
