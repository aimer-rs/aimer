//! [`ForegroundDecorationBox`], which paints a decoration over its child.

use std::cell::Cell;

use crate::decoration_box::{PaintGeometry, PaintKey, sanitized_outsets};
use crate::{BoxDecoration, Decoration};
use aimer_attribute::{ResolvedSize, Size, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    PortableWidget, Rebuildable, RequiredChild, VisitorElement, Widget,
};

/// Paints one decoration over its child while preserving its layout and interaction.
///
/// This is the mirror of [`DecorationBox`](crate::DecorationBox): the child
/// paints first and the decoration paints on top of it, so an opaque child can
/// no longer cover a highlight such as [`LightEdge`](crate::LightEdge). The
/// decoration covers exactly the child's box. Borders do not inset the child,
/// corner radii do not clip it, and the wrapper never consumes pointer or
/// scroll events: they reach the child as if the wrapper were not there.
///
/// Decorations that pick a color from the surface beneath them, such as an
/// automatic [`LightEdge`](crate::LightEdge), see the background the wrapper
/// inherited, not whatever the child painted.
///
/// The decoration is painted by its own retained render node, positioned after
/// the child's subtree, so changing it re-records only that node. Layered
/// decorations compose through the same collections [`DecorationBox`](crate::DecorationBox)
/// accepts. Custom decoration widgets do not support portable lowering.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{ForegroundDecorationBox, LightEdge};
/// use aimer_widget::base::Color;
///
/// let widget = ForegroundDecorationBox::new()
///     .decoration(LightEdge::new().width(6.0))
///     .child(SizedBox::new().width(120).height(40).color(Color::BLUE));
/// ```
pub struct ForegroundDecorationBox<D = BoxDecoration, W = RequiredChild> {
    decoration: D,
    child: W,
}

impl Default for ForegroundDecorationBox {
    fn default() -> Self {
        Self::new()
    }
}

impl ForegroundDecorationBox {
    /// Creates an empty foreground builder that requires a child.
    ///
    /// Finish with [`ForegroundDecorationBox::child`] or
    /// [`ForegroundDecorationBox::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            decoration: BoxDecoration::new(),
            child: RequiredChild,
        }
    }
}

impl<D: Decoration> ForegroundDecorationBox<D> {
    /// Replaces the decoration, preserving its concrete type without boxing.
    #[inline]
    pub fn decoration<E: Decoration>(self, decoration: E) -> ForegroundDecorationBox<E> {
        ForegroundDecorationBox {
            decoration,
            child: self.child,
        }
    }

    /// Attaches the required child and completes the builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> ForegroundDecorationBox<D, W> {
        ForegroundDecorationBox {
            decoration: self.decoration,
            child,
        }
    }

    /// Attaches a child and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget
    where
        D: 'static,
    {
        self.child(child).boxed()
    }
}

impl<D: Decoration + 'static, W: Widget> PortableWidget for ForegroundDecorationBox<D, W> {}

impl<D: Decoration + 'static, W: Widget> Widget for ForegroundDecorationBox<D, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        foreground_element(
            self.decoration,
            self.child.to_element(ctx),
            "ForegroundDecorationBox",
        )
    }

    fn debug_name(&self) -> &'static str {
        "ForegroundDecorationBox"
    }
}

/// Wraps `child` and a layer that paints `decoration` after it, over the same box.
pub(crate) fn foreground_element<D: Decoration + 'static>(
    decoration: D,
    child: AnyElement,
    name: &'static str,
) -> AnyElement {
    overlay_element(
        child,
        ForegroundLayer {
            decoration,
            recorded: Cell::new(None),
        }
        .boxed(),
        name,
    )
}

/// Wraps `child` and a leaf `layer` that paints after it over the same box.
///
/// The layer is a second retained child, visited after the first, because the
/// retained tree paints a node before its children and has no way for a node to
/// paint after them.
pub(crate) fn overlay_element(
    child: AnyElement,
    layer: AnyElement,
    name: &'static str,
) -> AnyElement {
    RawOverlayBox { child, layer, name }.boxed()
}

struct RawOverlayBox {
    child: AnyElement,
    layer: AnyElement,
    name: &'static str,
}

impl RawOverlayBox {
    #[inline]
    fn is_layer(&self, child: &dyn Element) -> bool {
        std::ptr::eq(child, self.layer.as_ref())
    }

    /// The context the layer is laid out and painted in: tight to the child's
    /// box, so the layer is exactly as large as the child.
    fn layer_context<'a>(&self, ctx: &BuildContext<'a>) -> BuildContext<'a> {
        let size = self.child.computed_size(ctx);
        let mut layer_ctx = ctx.clone();
        layer_ctx.parent_size = size;
        layer_ctx.box_constraint = aimer_attribute::BoxConstraint {
            min_width: size.width,
            min_height: size.height,
            max_width: size.width,
            max_height: size.height,
        };
        layer_ctx
    }
}

impl Rebuildable for RawOverlayBox {}

impl Drawable for RawOverlayBox {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
        // The layer is a retained node of its own; the frame walk reaches it
        // only through here.
        self.layer.update(&self.layer_context(ctx));
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    /// The wrapper paints nothing itself; its layer node does.
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<BuildContext<'a>> {
        // Both children are given a context so the walker never has to guess
        // whether this wrapper is transparent. With two retained children it
        // would decide it is not, and constrain the child to its own size.
        Some(if self.is_layer(child) {
            self.layer_context(ctx)
        } else {
            ctx.clone()
        })
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        _child_index: usize,
    ) -> Option<(Rect, Option<Rect>)> {
        if !self.is_layer(child) {
            return None;
        }
        let size = self.child.computed_size(ctx);
        let scale = ctx.scale;
        let valid = scale.is_finite() && scale > 0.0;
        Some((
            if valid {
                Rect::new(0.0, 0.0, size.width / scale, size.height / scale)
            } else {
                Rect::new(0.0, 0.0, 0.0, 0.0)
            },
            None,
        ))
    }
}

impl EventElement for RawOverlayBox {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl VisitorElement for RawOverlayBox {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
        visitor(self.layer.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        self.name
    }
}

impl LayoutElement for RawOverlayBox {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

/// The leaf that records a decoration over the box its wrapper hands it.
///
/// Its size is the context's `parent_size`, which the wrapper sets to the
/// child's computed size, so it is exactly as large as the child.
struct ForegroundLayer<D> {
    decoration: D,
    recorded: Cell<Option<PaintKey>>,
}

impl<D: Decoration> ForegroundLayer<D> {
    #[inline]
    fn paint_key(&self, ctx: &BuildContext) -> PaintKey {
        PaintKey {
            revision: self.decoration.revision(),
            background: ctx.paint_background,
            geometry: PaintGeometry::new(ctx.parent_size, ctx.scale),
        }
    }
}

impl<D: Decoration> Rebuildable for ForegroundLayer<D> {}

impl<D: Decoration> Drawable for ForegroundLayer<D> {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let key = self.paint_key(ctx);
        let canvas = Canvas::of(ctx);
        if let Some(geometry) = key.geometry {
            let [left, top, _, _] = sanitized_outsets(&self.decoration, geometry);
            // The node starts at the outer paint corner; see `DecorationBox`.
            self.decoration.paint_layer(
                &canvas,
                Vec2d { x: left, y: top },
                geometry.logical_size(),
                geometry.scale,
            );
        }
        canvas.finish();
        self.recorded.set(Some(key));
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.recorded.get() != Some(self.paint_key(ctx))
    }

    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(
            PaintGeometry::new(ctx.parent_size, ctx.scale)
                .map_or(ResolvedSize::default(), |geometry| geometry.size),
        )
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        Some(
            PaintGeometry::new(ctx.parent_size, ctx.scale)
                .map_or([0.0; 4], |geometry| sanitized_outsets(&self.decoration, geometry)),
        )
    }
}

impl<D: Decoration> EventElement for ForegroundLayer<D> {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl<D: Decoration> VisitorElement for ForegroundLayer<D> {
    fn debug_name(&self) -> &'static str {
        "ForegroundDecorationLayer"
    }
}

impl<D: Decoration> LayoutElement for ForegroundLayer<D> {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        true
    }
}
