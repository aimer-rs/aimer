use std::cell::Cell;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_canvas::Canvas;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement,
    PortableWidget, Rebuildable, VisitorElement, Widget,
};

/// Application code that draws into the box of a [`CustomPaint`].
///
/// A painter records shapes, text and images through the [`Canvas`] it is
/// handed. Coordinates are logical pixels with the origin at the top-left of the
/// box, and `size` is the box's extent in the same unit, so a painter never
/// handles the display scale itself. Keep the drawing inside `size`.
///
/// `paint` runs when the framework needs a fresh recording: the first time the
/// element is painted, after the box is resized, and after the display scale
/// changes. Between those the recorded commands are replayed, so a painter that
/// is expensive to evaluate is not run on every frame. To change what is drawn,
/// rebuild the [`CustomPaint`] with a new painter.
///
/// # Examples
///
/// ```
/// use aimer_attribute::size::ResolvedSize;
/// use aimer_canvas::Canvas;
/// use aimer_custom_paint::CanvasPainter;
///
/// struct Dot;
///
/// impl CanvasPainter for Dot {
///     fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
///         canvas.fill_rect(
///             aimer_canvas::Rect::new(0.0, 0.0, size.width, size.height),
///             [255, 0, 0, 255],
///         );
///     }
/// }
/// ```
pub trait CanvasPainter: 'static {
    /// Records this painter's commands for a box of `size` logical pixels.
    ///
    /// The canvas is committed when this method returns.
    fn paint(&self, canvas: &Canvas, size: ResolvedSize);
}

/// The painter slot of a [`CustomPaint`] that has not been given one yet.
///
/// It is not a widget; finish the builder with [`CustomPaint::painter`].
pub struct RequiredPainter;

/// Reserves a box in the layout and draws into it with a [`CanvasPainter`].
///
/// The box is sized like any other leaf: [`width`](CustomPaint::width) and
/// [`height`](CustomPaint::height) take logical pixels or a percentage of the
/// parent's maximum, and [`Dimension::Auto`], the default, fills the space the
/// parent offers on that axis. The widget draws nothing itself and does not
/// take pointer events, so it can sit under or beside interactive widgets.
///
/// # Examples
///
/// ```
/// use aimer_attribute::size::ResolvedSize;
/// use aimer_canvas::Canvas;
/// use aimer_custom_paint::{CanvasPainter, CustomPaint};
///
/// struct Swatch([u8; 4]);
///
/// impl CanvasPainter for Swatch {
///     fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
///         canvas.fill_rect(
///             aimer_canvas::Rect::new(0.0, 0.0, size.width, size.height),
///             self.0,
///         );
///     }
/// }
///
/// let swatch = CustomPaint::new()
///     .width(48.0)
///     .height(24.0)
///     .painter(Swatch([0, 128, 255, 255]));
/// ```
pub struct CustomPaint<P = RequiredPainter> {
    width: Dimension,
    height: Dimension,
    painter: P,
}

impl CustomPaint {
    /// Creates a builder that fills the space it is offered.
    ///
    /// Finish the builder with [`CustomPaint::painter`] or
    /// [`CustomPaint::box_painter`].
    #[inline]
    pub fn new() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            painter: RequiredPainter,
        }
    }

    /// Attaches the required painter and completes this builder.
    #[inline]
    pub fn painter<P: CanvasPainter>(self, painter: P) -> CustomPaint<P> {
        CustomPaint {
            width: self.width,
            height: self.height,
            painter,
        }
    }

    /// Attaches `painter` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_painter<P: CanvasPainter>(self, painter: P) -> AnyWidget {
        self.painter(painter).boxed()
    }
}

impl Default for CustomPaint {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<P> CustomPaint<P> {
    /// Sets the preferred width.
    ///
    /// The default is [`Dimension::Auto`], which fills the width the parent
    /// offers. Pixel values are logical pixels and percentages resolve against
    /// the parent's maximum width.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the preferred height.
    ///
    /// The default is [`Dimension::Auto`], which fills the height the parent
    /// offers. Pixel values are logical pixels and percentages resolve against
    /// the parent's maximum height.
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }
}

impl<P: CanvasPainter> PortableWidget for CustomPaint<P> {}

impl<P: CanvasPainter> Widget for CustomPaint<P> {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawCustomPaint {
            width: self.width,
            height: self.height,
            painter: self.painter,
            recorded: Cell::new(None),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "CustomPaint"
    }
}

/// What a recording was made for; a different key means it is stale.
#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintKey {
    width: u32,
    height: u32,
    scale: u32,
}

struct RawCustomPaint<P> {
    width: Dimension,
    height: Dimension,
    painter: P,
    recorded: Cell<Option<PaintKey>>,
}

impl<P: CanvasPainter> RawCustomPaint<P> {
    /// Returns the key and logical extent to record for, or `None` while the
    /// scale or size is not a usable number.
    fn paint_target(&self, ctx: &BuildContext) -> Option<(PaintKey, ResolvedSize)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let size = self.computed_size(ctx);
        if !size.width.is_finite()
            || !size.height.is_finite()
            || size.width < 0.0
            || size.height < 0.0
        {
            return None;
        }
        let key = PaintKey {
            width: size.width.to_bits(),
            height: size.height.to_bits(),
            scale: scale.to_bits(),
        };
        // Layout sizes are device-scaled; recorded commands use logical pixels.
        let logical = ResolvedSize {
            width: size.width / scale,
            height: size.height / scale,
        };
        Some((key, logical))
    }
}

impl<P: CanvasPainter> Drawable for RawCustomPaint<P> {
    #[inline]
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.paint_target(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let (key, size) = self
            .paint_target(ctx)
            .expect("CustomPaint v2 support must be checked before painting");
        let canvas = Canvas::of(ctx);
        self.painter.paint(&canvas, size);
        canvas.finish();
        self.recorded.set(Some(key));
    }

    #[inline]
    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.recorded.get() != self.paint_target(ctx).map(|(key, _)| key)
    }
}

impl<P: CanvasPainter> EventElement for RawCustomPaint<P> {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl<P: CanvasPainter> VisitorElement for RawCustomPaint<P> {
    fn debug_name(&self) -> &'static str {
        "CustomPaint"
    }
}

impl<P: CanvasPainter> Rebuildable for RawCustomPaint<P> {}

impl<P: CanvasPainter> LayoutElement for RawCustomPaint<P> {
    fn size(&self) -> Option<Size> {
        match (self.width, self.height) {
            (Dimension::Px(_), Dimension::Px(_)) => Some(Size {
                width: self.width,
                height: self.height,
            }),
            _ => None,
        }
    }

    fn is_layout_stable(&self) -> bool {
        matches!(
            (self.width, self.height),
            (Dimension::Px(_), Dimension::Px(_))
        )
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let axis = |dimension: Dimension, max: f32, parent: f32| match dimension {
            Dimension::Auto => parent,
            dimension => dimension.resolve(max, ctx.scale),
        };
        ResolvedSize {
            width: axis(
                self.width,
                ctx.box_constraint.max_width,
                ctx.parent_size.width,
            ),
            height: axis(
                self.height,
                ctx.box_constraint.max_height,
                ctx.parent_size.height,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aimer_attribute::position::Vec2d;
    use aimer_cupid::draw_cmd_v2::{
        DrawCommand, Rect as RenderRect, RenderPaintSource, RenderTree,
    };
    use aimer_widget::base::WindowHandle;

    use super::*;

    /// Fills its whole box and remembers every size it was asked to paint.
    struct Fill {
        sizes: Rc<RefCell<Vec<ResolvedSize>>>,
    }

    impl CanvasPainter for Fill {
        fn paint(&self, canvas: &Canvas, size: ResolvedSize) {
            self.sizes.borrow_mut().push(size);
            canvas.fill_rect(
                aimer_canvas::Rect::new(0.0, 0.0, size.width, size.height),
                [255, 0, 0, 255],
            );
        }
    }

    fn fill() -> (Fill, Rc<RefCell<Vec<ResolvedSize>>>) {
        let sizes = Rc::new(RefCell::new(Vec::new()));
        (
            Fill {
                sizes: sizes.clone(),
            },
            sizes,
        )
    }

    fn context(parent: ResolvedSize, scale: f32) -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            parent,
            scale,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), f64::from(scale)),
            tokio::runtime::Handle::current(),
        )
    }

    /// Runs one retained paint frame for `element` and returns its draw list.
    fn paint_frame(
        ctx: &BuildContext,
        element: &AnyElement,
        tree: &RenderTree,
        node: aimer_cupid::draw_cmd_v2::RenderNodeId,
    ) {
        let nodes = Rc::new(aimer_widget::ElementNodeMap::from([(element.id(), node)]));
        ctx.canvas.begin_frame();
        aimer_widget::begin_paint_frame(100, 80);
        aimer_widget::with_v2_render_tree_context(tree.clone(), nodes, Some(element.id()), || {
            element.update(ctx)
        });
    }

    #[tokio::test]
    async fn pixel_size_is_scaled_and_stable() {
        let ctx = context(ResolvedSize::default(), 2.0);
        let (painter, _) = fill();
        let element = CustomPaint::new()
            .width(24.0)
            .height(12.0)
            .painter(painter)
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 48.0,
                height: 24.0,
            }
        );
        assert_eq!(
            element.size(),
            Some(Size {
                width: Dimension::Px(24.0),
                height: Dimension::Px(12.0),
            })
        );
        assert!(element.is_layout_stable());
    }

    #[tokio::test]
    async fn auto_axis_fills_the_parent_and_is_not_stable() {
        let ctx = context(
            ResolvedSize {
                width: 300.0,
                height: 200.0,
            },
            1.0,
        );
        let (painter, _) = fill();
        let element = CustomPaint::new().height(40.0).painter(painter).to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 300.0,
                height: 40.0,
            }
        );
        assert_eq!(element.size(), None);
        assert!(!element.is_layout_stable());
    }

    #[tokio::test]
    async fn painter_receives_the_logical_size_and_records_into_the_local_list() {
        let ctx = context(ResolvedSize::default(), 2.0);
        let (painter, sizes) = fill();
        let element = CustomPaint::new()
            .width(24.0)
            .height(12.0)
            .painter(painter)
            .to_element(&ctx);
        let tree = RenderTree::new();
        let node = tree.add_root(RenderRect::new(0.0, 0.0, 24.0, 12.0)).unwrap();

        paint_frame(&ctx, &element, &tree, node);

        assert_eq!(
            *sizes.borrow(),
            [ResolvedSize {
                width: 24.0,
                height: 12.0,
            }]
        );
        let list = tree.draw_list_snapshot(node).unwrap();
        assert!(matches!(
            list.commands.as_ref(),
            [DrawCommand::FillRect { rect, color, .. }]
                if *rect == RenderRect::new(0.0, 0.0, 24.0, 12.0)
                    && (color.r, color.g, color.b, color.a) == (255, 0, 0, 255)
        ));
        assert_eq!(tree.paint_source(node).unwrap(), RenderPaintSource::LocalV2);
    }

    #[tokio::test]
    async fn recording_is_kept_until_the_size_or_scale_changes() {
        let ctx = context(ResolvedSize::default(), 1.0);
        let (painter, sizes) = fill();
        let element = CustomPaint::new()
            .width(24.0)
            .height(12.0)
            .painter(painter)
            .to_element(&ctx);
        let tree = RenderTree::new();
        let node = tree.add_root(RenderRect::new(0.0, 0.0, 24.0, 12.0)).unwrap();

        assert!(element.local_v2_paint_needs_recording(&ctx));
        paint_frame(&ctx, &element, &tree, node);
        assert!(!element.local_v2_paint_needs_recording(&ctx));
        assert_eq!(sizes.borrow().len(), 1);

        let rescaled = context(ResolvedSize::default(), 2.0);
        assert!(element.local_v2_paint_needs_recording(&rescaled));
    }

    #[tokio::test]
    async fn an_unusable_scale_is_not_painted() {
        let (painter, _) = fill();
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let ctx = context(ResolvedSize::default(), scale);
            let element = CustomPaint::new()
                .width(24.0)
                .height(12.0)
                .painter(painter_like(&painter))
                .to_element(&ctx);

            assert!(!element.can_paint_local_v2(&ctx), "scale {scale}");
        }
    }

    fn painter_like(painter: &Fill) -> Fill {
        Fill {
            sizes: painter.sizes.clone(),
        }
    }

    #[tokio::test]
    async fn it_does_not_take_pointer_events() {
        let ctx = context(ResolvedSize::default(), 1.0);
        let (painter, _) = fill();
        let element = CustomPaint::new().painter(painter).to_element(&ctx);

        assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
    }
}
