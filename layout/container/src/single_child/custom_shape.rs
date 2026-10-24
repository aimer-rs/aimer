use std::marker::PhantomData;
use std::sync::Arc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::shape::DrawShape;
use aimer_macro::{EventElement, Rebuildable};
use aimer_shape::{
    FillStyle, ShapeClip, ShapeFit, ShapeHitTest, ShapePath, ShapeSize, StrokeStyle,
};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Drawable, Element, LayoutElement, PortableWidget,
    RequiredChild, VisitorElement, Widget,
};

/// A renderer-neutral shape background that retains one child subtree.
///
/// `CustomShape` is intentionally a visual container, not a free-form drawing
/// surface: the path is validated by `aimer_shape`, the child is retained by a
/// [`ChildBuilder`], and the canvas receives a typed [`DrawShape`] request.
/// Child layout, events, focus, and semantics continue through the wrapper.
///
/// Finish the type-state builder with [`CustomShape::child`] or
/// [`CustomShape::box_child`]. A missing path is a safe no-paint fallback, so a
/// partially configured value cannot reach a platform drawing API.
pub struct CustomShape<W = RequiredChild> {
    path: Option<Arc<ShapePath>>,
    fill: Option<FillStyle>,
    stroke: Option<StrokeStyle>,
    clip: ShapeClip,
    fit: ShapeFit,
    hit_test: ShapeHitTest,
    opacity: f32,
    child: ChildBuilder,
    marker: PhantomData<fn() -> W>,
}

impl Default for CustomShape {
    fn default() -> Self {
        Self::new()
    }
}

impl CustomShape {
    /// Creates a shape builder with no path, paint, or child attached.
    ///
    /// The final [`CustomShape::child`] call supplies the retained subtree and
    /// completes the type-state transition.
    #[inline]
    pub fn new() -> Self {
        Self {
            path: None,
            fill: None,
            stroke: None,
            clip: ShapeClip::None,
            fit: ShapeFit::None,
            hit_test: ShapeHitTest::None,
            opacity: 1.0,
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }

    /// Sets an already validated path.
    #[inline]
    pub fn path(mut self, path: ShapePath) -> Self {
        self.path = Some(Arc::new(path));
        self
    }

    /// Sets an already retained path handle without copying its commands.
    #[inline]
    pub fn shared_path(mut self, path: Arc<ShapePath>) -> Self {
        self.path = Some(path);
        self
    }

    /// Sets the optional fill.
    #[inline]
    pub const fn fill(mut self, fill: FillStyle) -> Self {
        self.fill = Some(fill);
        self
    }

    /// Removes the fill.
    #[inline]
    pub const fn without_fill(mut self) -> Self {
        self.fill = None;
        self
    }

    /// Sets the optional stroke.
    #[inline]
    pub fn stroke(mut self, stroke: StrokeStyle) -> Self {
        self.stroke = Some(stroke);
        self
    }

    /// Removes the stroke.
    #[inline]
    pub fn without_stroke(mut self) -> Self {
        self.stroke = None;
        self
    }

    /// Sets the shape-owned clipping policy.
    #[inline]
    pub fn clip(mut self, clip: ShapeClip) -> Self {
        self.clip = clip;
        self
    }

    /// Sets how the path is fitted into the retained child's resolved size.
    #[inline]
    pub const fn fit(mut self, fit: ShapeFit) -> Self {
        self.fit = fit;
        self
    }

    /// Sets pointer hit-test metadata. This never changes keyboard focus.
    #[inline]
    pub const fn hit_test(mut self, hit_test: ShapeHitTest) -> Self {
        self.hit_test = hit_test;
        self
    }

    /// Sets the shape alpha; invalid values safely skip shape paint.
    #[inline]
    pub const fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Attaches the required child and retains its concrete type in the builder.
    #[inline]
    pub fn child<W: Widget + 'static>(self, child: W) -> CustomShape<W> {
        CustomShape {
            path: self.path,
            fill: self.fill,
            stroke: self.stroke,
            clip: self.clip,
            fit: self.fit,
            hit_test: self.hit_test,
            opacity: self.opacity,
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches a child and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for CustomShape<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawCustomShape {
            path: self.path,
            fill: self.fill,
            stroke: self.stroke,
            clip: self.clip,
            fit: self.fit,
            hit_test: self.hit_test,
            opacity: self.opacity,
            child: self.child.build(ctx),
        }
        .boxed()
    }
}

impl<W: Widget + 'static> PortableWidget for CustomShape<W> {}

#[derive(EventElement, Rebuildable)]
struct RawCustomShape {
    path: Option<Arc<ShapePath>>,
    fill: Option<FillStyle>,
    stroke: Option<StrokeStyle>,
    clip: ShapeClip,
    fit: ShapeFit,
    hit_test: ShapeHitTest,
    opacity: f32,
    child: AnyElement,
}

impl Drawable for RawCustomShape {
    fn update(&self, ctx: &BuildContext) {
        // The shape is painted by `paint_local_v2`; the retained child remains
        // the canonical event, focus, semantics, and visual subtree.
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        let scale = if ctx.scale.is_finite() && ctx.scale > 0.0 {
            ctx.scale
        } else {
            1.0
        };
        let size = self.child.computed_size(ctx);
        let viewport = ShapeSize::new(size.width / scale, size.height / scale);
        if size.width > 0.0
            && size.height > 0.0
            && viewport.is_valid()
            && let Some(path) = self.path.as_ref()
            && let Ok(transform) = self.fit.transform(path.bounds(), viewport)
        {
            let mut request = DrawShape::new(Arc::clone(path))
                .transform(transform)
                .opacity(self.opacity)
                .hit_test(self.hit_test)
                .clip(self.clip.clone());
            if let Some(fill) = self.fill {
                request = request.fill(fill);
            }
            if let Some(stroke) = self.stroke.as_ref() {
                request = request.stroke(stroke.clone());
            }
            let _ = canvas.draw_shape(&request, viewport);
        }
        canvas.finish();
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }
}

impl RawCustomShape {}

impl VisitorElement for RawCustomShape {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "CustomShape"
    }
}

impl LayoutElement for RawCustomShape {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

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

    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
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
