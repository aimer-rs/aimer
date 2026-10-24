use aimer_attribute::dimension::Dimension;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_cupid::draw_cmd_v2::Rect;
use aimer_macro::{PortableWidget, Rebuildable};
use aimer_widget::base::{Color, *};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, SourceFingerprint};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutCache,
    LayoutElement, VisitorElement, Widget,
};

use crate::ZeroSizedBox;

/// A single-child box with optional explicit dimensions and background color.
///
/// Attach a child with [`SizedBox::child`] to retain its concrete type, or with
/// [`SizedBox::box_child`] when branches need a shared erased type.
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_container::single_child::SizedBox",
    schema_only,
    validate = validate_portable_sized_box
)]
pub struct SizedBox<W: Widget + 'static = ZeroSizedBox> {
    #[portable_optional]
    width: Dimension,
    #[portable_optional]
    height: Dimension,
    #[portable_skip]
    color: Color,
    #[portable_skip]
    child: Option<W>,
}

impl Default for SizedBox {
    fn default() -> Self {
        Self::new()
    }
}

impl SizedBox {
    /// Creates a transparent, automatically sized box without a child.
    ///
    /// The box is already a valid widget; use [`SizedBox::child`] or
    /// [`SizedBox::box_child`] to attach content.
    #[inline]
    pub fn new() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
            color: Color::Transparent,
            child: None,
        }
    }

    /// Sets the preferred width.
    ///
    /// The default is [`Dimension::Auto`], which derives width from the child
    /// or zero when no child exists. Pixel values are logical pixels,
    /// percentages resolve against the parent's maximum width, and
    /// constraints still apply.
    #[inline]
    pub fn width(mut self, width: impl Into<Dimension>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the preferred height.
    ///
    /// The default is [`Dimension::Auto`], which derives height from the child
    /// or zero when no child exists. Pixel values are logical pixels,
    /// percentages resolve against the parent's maximum height, and
    /// constraints still apply.
    #[inline]
    pub fn height(mut self, height: impl Into<Dimension>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets the box's fill color.
    ///
    /// The default is [`Color::Transparent`]. The color fills the box's
    /// resolved bounds before its child is drawn.
    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }

    /// Attaches or replaces the optional child.
    ///
    /// `SizedBox::new()` is already valid without content. This operation
    /// preserves the concrete child type; use [`SizedBox::box_child`] when
    /// branches need an erased type.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> SizedBox<W> {
        SizedBox {
            width: self.width,
            height: self.height,
            color: self.color,
            child: Some(child),
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`SizedBox::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl SizedBox {
    pub const PLACE_HOLDER: Option<ZeroSizedBox> = Some(ZeroSizedBox);
}

impl<W: Widget + 'static> Widget for SizedBox<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self
            .child
            .map(|child| child.to_element(ctx))
            .unwrap_or_else(|| ZeroSizedBox.to_element(ctx));
        RawSizedBox {
            width: self.width,
            height: self.height,
            child,
            color: self.color,
            cache: LayoutCache::new(),
            debug_name: "SizedBox",
        }
        .boxed()
    }
}

#[cfg(feature = "portable-guest")]
fn validate_portable_sized_box<W: Widget + 'static>(
    sized_box: &SizedBox<W>,
    ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    if sized_box.child.is_some() {
        return Err(ctx.unsupported_widget("SizedBox.child", source));
    }
    if sized_box.color != Color::Transparent {
        return Err(ctx.unsupported_widget("SizedBox.color", source));
    }
    for (dimension, property) in [
        (sized_box.width, "SizedBox.width"),
        (sized_box.height, "SizedBox.height"),
    ] {
        let valid = match dimension {
            Dimension::Auto => true,
            Dimension::Px(value) => value.is_finite() && value >= 0.0,
            Dimension::Percent(_) => false,
        };
        if !valid {
            return Err(ctx.unsupported_widget(property, source));
        }
    }
    Ok(())
}
#[derive(Rebuildable)]
#[rebuildable(forwards_marks_only)]
pub struct RawSizedBox<E: Element> {
    pub(crate) width: Dimension,
    pub(crate) height: Dimension,
    pub(crate) color: Color,
    pub(crate) child: E,
    pub(crate) cache: LayoutCache,
    pub(crate) debug_name: &'static str,
}

impl<E: Element> RawSizedBox<E> {
    fn local_v2_paint_size(&self, ctx: &BuildContext) -> Option<(f32, f32)> {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }

        let size = self.computed_size(ctx);
        let width = size.width / scale;
        let height = size.height / scale;
        (width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0)
            .then_some((width, height))
    }
}

impl<E: Element> EventElement for RawSizedBox<E> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }
}

impl<E: Element> Drawable for RawSizedBox<E> {
    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let width = size.width;
        let height = size.height;

        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = ResolvedSize { width, height };
        child_ctx.box_constraint.max_width = width;
        child_ctx.box_constraint.max_height = height;
        self.child.update(&child_ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.local_v2_paint_size(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let (width, height) = self
            .local_v2_paint_size(ctx)
            .expect("SizedBox v2 support must be checked before painting");
        let canvas = aimer_canvas::Canvas::of(ctx);

        let (red, green, blue, alpha) = self.color.to_rgba();
        if alpha != 0 {
            // SizedBox layout sizes are device-scaled; v2 commands use local logical pixels.
            canvas.fill_rect(
                Rect::new(0.0, 0.0, width, height),
                [red, green, blue, alpha],
            );
        }
        canvas.finish();
    }
}

impl<E: Element> VisitorElement for RawSizedBox<E> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(&self.child);
    }

    fn debug_name(&self) -> &'static str {
        self.debug_name
    }
}

impl<E: Element> LayoutElement for RawSizedBox<E> {
    fn size(&self) -> Option<Size> {
        match (self.width, self.height) {
            (Dimension::Px(w), Dimension::Px(h)) => Some(Size {
                width: Dimension::Px(w),
                height: Dimension::Px(h),
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
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_computed(ctx.box_constraint, scale_bits) {
            return cached;
        }

        let scale = ctx.scale;

        let mut child_ctx = BuildContext {
            parent_size: ctx.parent_size,
            canvas: ctx.canvas.clone(),
            scale: ctx.scale,
            parent_pos: ctx.parent_pos,
            cursor_pos: ctx.cursor_pos,
            box_constraint: ctx.box_constraint,
            visible_rect: ctx.visible_rect,
            paint_background: ctx.paint_background,
            window: ctx.window.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            async_handle: ctx.async_handle.clone(),
            inherited_states: ctx.inherited_states.clone(),
        };

        child_ctx.box_constraint.max_width =
            self.width.resolve(ctx.box_constraint.max_width, scale);
        child_ctx.box_constraint.max_height =
            self.height.resolve(ctx.box_constraint.max_height, scale);

        let width = match self.width {
            Dimension::Px(w) => w * scale,
            Dimension::Percent(p) => ctx.box_constraint.max_width * (p / 100.0),
            Dimension::Auto => self.child.computed_size(&child_ctx).width,
        };

        let height = match self.height {
            Dimension::Px(h) => h * scale,
            Dimension::Percent(p) => ctx.box_constraint.max_height * (p / 100.0),
            Dimension::Auto => self.child.computed_size(&child_ctx).height,
        };

        let result = ResolvedSize { width, height };
        self.cache
            .set_computed(ctx.box_constraint, scale_bits, result);
        result
    }

    fn get_size_from_child(&self) -> Option<Size> {
        let mut size = self.child.get_size_from_child().unwrap_or_default();
        if let Dimension::Px(_) = self.width {
            size.width = self.width;
        }
        if let Dimension::Px(_) = self.height {
            size.height = self.height;
        }
        Some(size)
    }

    fn invalidate_layout(&self) {
        self.cache.invalidate();
        self.child.invalidate_layout();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use aimer_cupid::draw_cmd_v2::{
        DrawCommand, Rect as RenderRect, RenderPaintSource, RenderTree,
    };
    use aimer_widget::base::WindowHandle;
    use aimer_widget::{Drawable, EventElement, EventTreeRole, Rebuildable, VisitorElement};

    struct FixedChild;

    impl Drawable for FixedChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    struct DrawChild {
        draws: Rc<Cell<usize>>,
    }

    impl Drawable for DrawChild {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.set(self.draws.get() + 1);
        }
    }

    impl EventElement for DrawChild {}
    impl Rebuildable for DrawChild {}
    impl aimer_widget::PortableWidget for DrawChild {}

    impl VisitorElement for DrawChild {
        fn debug_name(&self) -> &'static str {
            "DrawChild"
        }
    }

    impl LayoutElement for DrawChild {
        fn size(&self) -> Option<Size> {
            Some(Size {
                width: Dimension::Px(24.0),
                height: Dimension::Px(12.0),
            })
        }
    }

    impl Widget for DrawChild {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Element::boxed(self)
        }
    }

    impl EventElement for FixedChild {}
    impl Rebuildable for FixedChild {}
    impl aimer_widget::PortableWidget for FixedChild {}

    impl VisitorElement for FixedChild {
        fn debug_name(&self) -> &'static str {
            "FixedChild"
        }
    }

    impl LayoutElement for FixedChild {
        fn size(&self) -> Option<Size> {
            Some(Size {
                width: Dimension::Px(24.0),
                height: Dimension::Px(12.0),
            })
        }
    }

    impl Widget for FixedChild {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Element::boxed(self)
        }
    }

    fn context() -> BuildContext<'static> {
        context_with_scale(1.0)
    }

    fn context_with_scale(scale: f32) -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            ResolvedSize::default(),
            scale,
            Vec2d::default(),
            Vec2d::default(),
            WindowHandle::headless(Default::default(), f64::from(scale)),
            tokio::runtime::Handle::current(),
        )
    }

    #[tokio::test]
    async fn sized_box_without_explicit_dimensions_uses_child_size() {
        let ctx = context();
        let element = SizedBox::new().child(FixedChild).to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 24.0,
                height: 12.0,
            }
        );
    }

    #[tokio::test]
    async fn sized_box_draws_its_child() {
        let draws = Rc::new(Cell::new(0));
        let ctx = context();
        let element = SizedBox::new()
            .child(DrawChild {
                draws: draws.clone(),
            })
            .to_element(&ctx);

        element.update(&ctx);

        assert_eq!(draws.get(), 1);
    }

    #[tokio::test]
    async fn sized_box_background_records_and_invalidates_only_its_local_v2_list() {
        let ctx = context_with_scale(2.0);
        let background = SizedBox::new()
            .width(24.0)
            .height(12.0)
            .color(Color::RED)
            .to_element(&ctx);
        let sibling = SizedBox::new()
            .width(16.0)
            .height(10.0)
            .color(Color::BLUE)
            .to_element(&ctx);
        let background_id = background.id();
        let sibling_id = sibling.id();
        let tree = RenderTree::new();
        let background_node = tree
            .add_root(RenderRect::new(0.0, 0.0, 24.0, 12.0))
            .unwrap();
        let sibling_node = tree
            .add_root(RenderRect::new(40.0, 0.0, 16.0, 10.0))
            .unwrap();
        let element_nodes = Rc::new(aimer_widget::ElementNodeMap::from([
            (background_id, background_node),
            (sibling_id, sibling_node),
        ]));

        ctx.canvas.begin_frame();
        aimer_widget::begin_paint_frame(100, 80);
        aimer_widget::with_v2_render_tree_context(
            tree.clone(),
            element_nodes.clone(),
            Some(background_id),
            || {
                background.update(&ctx);
                sibling.update(&ctx);
            },
        );

        let background_commands = tree.draw_list_snapshot(background_node).unwrap();
        let sibling_commands = tree.draw_list_snapshot(sibling_node).unwrap();
        assert_eq!(background_commands.revision, 1);
        assert_eq!(sibling_commands.revision, 1);
        assert!(matches!(
            background_commands.commands.as_ref(),
            [DrawCommand::FillRect { rect, color, .. }]
                if *rect == RenderRect::new(0.0, 0.0, 24.0, 12.0)
                    && (color.r, color.g, color.b, color.a) == (255, 0, 0, 255)
        ));
        assert_eq!(
            tree.paint_source(background_node).unwrap(),
            RenderPaintSource::LocalV2
        );

        tree.take_damage();
        tree.invalidate_paint(background_node).unwrap();
        ctx.canvas.begin_frame();
        aimer_widget::begin_paint_frame(100, 80);
        aimer_widget::with_v2_render_tree_context(tree.clone(), element_nodes, None, || {
            background.update(&ctx);
            sibling.update(&ctx);
        });

        assert_eq!(tree.draw_list_revision(background_node).unwrap(), 2);
        assert_eq!(tree.draw_list_revision(sibling_node).unwrap(), 1);
        assert_eq!(
            tree.take_damage(),
            vec![RenderRect::new(0.0, 0.0, 24.0, 12.0)]
        );
    }

    #[tokio::test]
    async fn sized_box_keeps_its_layout_bounds_in_indexed_event_routing() {
        let ctx = context();
        let element = SizedBox::new().child(FixedChild).to_element(&ctx);

        assert_eq!(element.event_tree_role(), EventTreeRole::IndexedTarget);
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::*;
    use aimer_anteros::{
        PROPERTY_SIZED_BOX_HEIGHT, PROPERTY_SIZED_BOX_WIDTH, PropertyValue, Version,
        WIDGET_SIZED_BOX, WidgetDocumentView, WidgetProperty,
    };
    use aimer_widget::PortableWidget;
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableWidgetLimits,
        PortableWidgetResource, PortableWidgetSchema, SourceFingerprint, StableId128,
    };

    fn source(value: u8) -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([value; 16]))
    }

    fn limits() -> PortableWidgetLimits {
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048).with_max_blob_bytes(128)
    }

    fn context(limits: PortableWidgetLimits) -> PortableBuildContext {
        PortableBuildContext::new(7, 11, limits, PortableLimits::new(8, 16, 64, 128, 1_024))
            .unwrap()
    }

    #[test]
    fn sized_box_publishes_its_reflected_schema() {
        let schema = <SizedBox<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA;

        assert_eq!(schema.widget().id(), WIDGET_SIZED_BOX);
        assert_eq!(schema.children().minimum(), 0);
        assert_eq!(schema.children().maximum(), 0);
        assert_eq!(schema.properties().len(), 2);
        assert_eq!(schema.properties()[0].id(), PROPERTY_SIZED_BOX_WIDTH);
        assert_eq!(schema.properties()[1].id(), PROPERTY_SIZED_BOX_HEIGHT);
    }

    #[test]
    fn sized_box_lowers_exact_bounded_widget_ir() {
        let source = source(5);
        let mut ctx = context(limits());
        let expected_key = ctx.slot_for(None, source).to_bytes();
        let root = SizedBox::new()
            .width(32.0)
            .height(16.0)
            .to_portable_node(&mut ctx, source)
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(0).unwrap();

        assert_eq!(view.node_count(), 1);
        assert_eq!(node.widget_type(), WIDGET_SIZED_BOX);
        assert_eq!(node.widget_schema(), Version::new(1, 0));
        assert_eq!(node.key().unwrap().as_bytes(), &expected_key);
        assert_eq!(node.children().count(), 0);
        assert_eq!(
            node.properties().collect::<Vec<_>>(),
            vec![
                WidgetProperty::new(PROPERTY_SIZED_BOX_WIDTH, PropertyValue::BlobRef(0)).optional(),
                WidgetProperty::new(PROPERTY_SIZED_BOX_HEIGHT, PropertyValue::BlobRef(1))
                    .optional(),
            ]
        );
        assert_eq!(view.blob(0), Some(&[1, 1, 0, 0, 0, 66][..]));
        assert_eq!(view.blob(1), Some(&[1, 1, 0, 0, 128, 65][..]));
    }

    #[test]
    fn sized_box_portable_lowering_enforces_property_and_key_limits() {
        let source = source(6);
        let mut properties = context(limits().with_max_properties(1));
        assert!(matches!(
            SizedBox::new()
                .width(10.0)
                .height(20.0)
                .to_portable_node(&mut properties, source),
            Err(PortableBuildError::LimitExceeded {
                resource: PortableWidgetResource::Properties,
                max: 1,
                actual: 2,
            })
        ));

        let mut keys = context(limits().with_max_keys(0));
        assert!(matches!(
            SizedBox::new().to_portable_node(&mut keys, source),
            Err(PortableBuildError::LimitExceeded {
                resource: PortableWidgetResource::Keys,
                max: 0,
                actual: 1,
            })
        ));
    }

    #[test]
    fn sized_box_rejects_schema_unsupported_native_features() {
        let source = source(7);
        let mut child_context = context(limits());
        assert!(matches!(
            SizedBox::new()
                .child(SizedBox::new())
                .to_portable_node(&mut child_context, source),
            Err(PortableBuildError::UnsupportedWidget {
                widget: "SizedBox.child",
                source: actual,
            }) if actual == source
        ));

        let mut color_context = context(limits());
        assert!(matches!(
            SizedBox::new()
                .color(Color::HexA(0xFFFFFFFF))
                .to_portable_node(&mut color_context, source),
            Err(PortableBuildError::UnsupportedWidget {
                widget: "SizedBox.color",
                source: actual,
            }) if actual == source
        ));

        let mut dimension_context = context(limits());
        assert!(matches!(
            SizedBox::new()
                .width(Dimension::Percent(50.0))
                .to_portable_node(&mut dimension_context, source),
            Err(PortableBuildError::UnsupportedWidget {
                widget: "SizedBox.width",
                source: actual,
            }) if actual == source
        ));
    }
}
