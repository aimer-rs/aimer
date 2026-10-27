//! [`DecorationBox`], which paints a decoration behind its child.
//!
//! The widget is layout-transparent: it forwards layout, events and focus to the
//! child. The material effects that sample what is already painted
//! ([`BackdropFilter`], [`LightEdge`], [`Magnification`]) and the ramp decorations
//! ([`Gradient`], [`OpacityGradient`]) live in child modules and are re-exported
//! from here.

use std::cell::Cell;

mod backdrop_filter;
mod gradient;
mod light_edge;
mod magnification;
mod opacity_gradient;
pub use backdrop_filter::BackdropFilter;
pub use gradient::Gradient;
pub use light_edge::LightEdge;
pub use magnification::{Magnification, MagnificationStyle};
pub use opacity_gradient::OpacityGradient;

use crate::{BoxDecoration, Decoration};
use aimer_attribute::{ResolvedSize, Size, Vec2d};
use aimer_canvas::Canvas;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    PortableWidget, Rebuildable, RequiredChild, VisitorElement, Widget,
};

/// Paints one child's background while preserving its layout and interaction.
///
/// Decoration borders do not inset the child, and corner radii do not clip it.
/// Compose with `aimer_container::Padding`, `aimer_container::Margin`, or
/// `aimer_container::SizedBox` for layout. All decoration commands paint before
/// the child; the wrapper never consumes pointer or scroll events, even with
/// an opaque background.
/// Custom decoration widgets do not support portable lowering.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::DecorationBox;
/// use aimer_decoration::BoxDecoration;
/// use aimer_widget::base::Color;
///
/// let widget = DecorationBox::new()
///     .decoration(BoxDecoration::new().background_color(Color::BLUE).border_radius(8.0))
///     .child(SizedBox::new().width(120).height(40));
/// ```
pub struct DecorationBox<D = BoxDecoration, W = RequiredChild> {
    decoration: D,
    child: W,
}

impl Default for DecorationBox {
    fn default() -> Self {
        Self::new()
    }
}

impl DecorationBox {
    /// Creates an empty decoration builder that requires a child.
    ///
    /// Finish with [`DecorationBox::child`] or [`DecorationBox::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            decoration: BoxDecoration::new(),
            child: RequiredChild,
        }
    }
}

impl<D: Decoration> DecorationBox<D> {
    /// Replaces the decoration, preserving its concrete type without boxing.
    #[inline]
    pub fn decoration<E: Decoration>(self, decoration: E) -> DecorationBox<E> {
        DecorationBox {
            decoration,
            child: self.child,
        }
    }

    /// Attaches the required child and completes the builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> DecorationBox<D, W> {
        DecorationBox {
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

impl<D: Decoration + 'static, W: Widget> PortableWidget for DecorationBox<D, W> {}

impl<D: Decoration + 'static, W: Widget> Widget for DecorationBox<D, W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        decoration_element(self.decoration, self.child, ctx, "DecorationBox")
    }

    fn debug_name(&self) -> &'static str {
        "DecorationBox"
    }
}

/// Returns the corner radii that preceding layers declared on `canvas`.
///
/// Each radius is bounded to `0.0..=max`; negative and nonfinite radii yield
/// square corners. Shared by the material decorations that adopt the box shape.
#[inline]
pub(crate) fn declared_corner_radii(canvas: &Canvas, max: f32) -> [f32; 4] {
    canvas
        .corner_radii()
        .map(|radius| if radius.is_finite() { radius.clamp(0.0, max) } else { 0.0 })
}

/// Returns `decoration`'s declared outsets with nonfinite and negative values
/// treated as zero.
pub(crate) fn sanitized_outsets<D: Decoration + ?Sized>(
    decoration: &D,
    geometry: PaintGeometry,
) -> [f32; 4] {
    decoration
        .paint_outsets(geometry.logical_size(), geometry.scale)
        .map(|outset| {
            if outset.is_finite() {
                outset.max(0.0)
            } else {
                0.0
            }
        })
}

pub(super) fn decoration_element<D: Decoration + 'static, W: Widget>(
    decoration: D,
    child: W,
    ctx: &BuildContext,
    name: &'static str,
) -> AnyElement {
    decoration_element_with(decoration, child, ctx, name, ChildMask::None)
}

/// How a decoration box combines its decoration with its child's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildMask {
    /// The decoration paints behind the child.
    None,
    /// The decoration shows only where the child has coverage.
    DecorationByChild,
    /// The child shows only where, and as much as, the decoration has alpha.
    ChildByDecoration,
}

/// Builds a decoration box whose decoration is combined with its child as `mask`
/// says.
pub(crate) fn decoration_element_with<D: Decoration + 'static, W: Widget>(
    decoration: D,
    child: W,
    ctx: &BuildContext,
    name: &'static str,
    mask: ChildMask,
) -> AnyElement {
    RawDecorationBox {
        decoration,
        child: child.to_element(ctx),
        recorded: Cell::new(None),
        name,
        mask,
    }
    .boxed()
}

struct RawDecorationBox<D> {
    decoration: D,
    child: AnyElement,
    recorded: Cell<Option<PaintKey>>,
    name: &'static str,
    mask: ChildMask,
}

/// What a retained decoration recorded last, compared to skip identical repaints.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct PaintKey {
    pub(crate) revision: u64,
    pub(crate) background: aimer_color::prelude::Color,
    pub(crate) geometry: Option<PaintGeometry>,
}

/// A decorated box in device pixels, validated for painting.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct PaintGeometry {
    pub(crate) size: ResolvedSize,
    pub(crate) scale: f32,
}

impl PaintGeometry {
    /// Returns the geometry for a device-pixel `size`, or `None` when the size
    /// or scale cannot be painted (nonfinite, negative, or a nonpositive scale).
    pub(crate) fn new(size: ResolvedSize, scale: f32) -> Option<Self> {
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let geometry = Self { size, scale };
        let logical = geometry.logical_size();
        (size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
            && logical.width.is_finite()
            && logical.height.is_finite()
            && logical.width >= 0.0
            && logical.height >= 0.0)
            .then_some(geometry)
    }

    #[inline]
    pub(crate) fn logical_size(self) -> ResolvedSize {
        ResolvedSize {
            width: self.size.width / self.scale,
            height: self.size.height / self.scale,
        }
    }
}

impl<D: Decoration> RawDecorationBox<D> {
    #[inline]
    fn with_child_paint_context<R>(
        &self,
        ctx: &BuildContext,
        callback: impl FnOnce(&BuildContext) -> R,
    ) -> R {
        let background = aimer_canvas::composite_background(
            self.decoration.background_color(),
            ctx.paint_background,
        );
        if background == ctx.paint_background {
            return callback(ctx);
        }
        let mut child_ctx = ctx.clone();
        child_ctx.paint_background = background;
        callback(&child_ctx)
    }

    fn paint_geometry(&self, ctx: &BuildContext) -> Option<PaintGeometry> {
        PaintGeometry::new(self.child.computed_size(ctx), ctx.scale)
    }

    fn paint_outsets(&self, geometry: PaintGeometry) -> [f32; 4] {
        sanitized_outsets(&self.decoration, geometry)
    }

    #[inline]
    fn paint_key(&self, ctx: &BuildContext) -> PaintKey {
        PaintKey {
            revision: self.decoration.revision(),
            background: ctx.paint_background,
            geometry: self.paint_geometry(ctx),
        }
    }
}

impl<D: Decoration> Rebuildable for RawDecorationBox<D> {}

impl<D: Decoration> Drawable for RawDecorationBox<D> {
    fn update(&self, ctx: &BuildContext) {
        self.with_child_paint_context(ctx, |child_ctx| self.child.update(child_ctx));
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.with_child_paint_context(ctx, |child_ctx| self.child.sync_paint_geometry(child_ctx));
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let key = self.paint_key(ctx);
        let canvas = Canvas::of(ctx);
        // The marker must head the list; see `COVERAGE_MASK_PIPELINE_NAME`.
        match self.mask {
            ChildMask::None => {}
            ChildMask::DecorationByChild => canvas.mask_by_descendants(),
            // A decoration that paints nothing has no alpha to fade by, which
            // leaves the child as it is rather than hiding it.
            ChildMask::ChildByDecoration if self.decoration.is_empty() => {}
            ChildMask::ChildByDecoration => canvas.fade_descendants(),
        }
        if let Some(geometry) = key.geometry {
            let size = geometry.logical_size();
            let [left, top, _, _] = self.paint_outsets(geometry);
            // The render node starts at the outer paint corner. Offset only
            // our commands; the retained walker compensates the child's origin.
            self.decoration
                .paint_layer(&canvas, Vec2d { x: left, y: top }, size, geometry.scale);
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
            self.paint_geometry(ctx)
                .map_or(ResolvedSize::default(), |geometry| geometry.size),
        )
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        Some(
            self.paint_geometry(ctx)
                .map_or([0.0; 4], |geometry| self.paint_outsets(geometry)),
        )
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        // Paint bounds can be larger than content size, but must never feed
        // back into the child's constraints or parent size.
        std::ptr::eq(child, self.child.as_ref()).then(|| {
            let mut child_ctx = ctx.clone();
            child_ctx.paint_background = aimer_canvas::composite_background(
                self.decoration.background_color(),
                ctx.paint_background,
            );
            child_ctx
        })
    }
}

impl<D: Decoration> EventElement for RawDecorationBox<D> {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl<D: Decoration> VisitorElement for RawDecorationBox<D> {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        self.name
    }
}

impl<D: Decoration> LayoutElement for RawDecorationBox<D> {
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

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod magnification_tests;

#[cfg(test)]
mod light_edge_tests;

#[cfg(test)]
mod backdrop_filter_tests;

#[cfg(test)]
mod gradient_tests;

#[cfg(test)]
mod masked_tests;

#[cfg(test)]
mod opacity_gradient_tests;

#[cfg(test)]
mod multi_placement_tests;
