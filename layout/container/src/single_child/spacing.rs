use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_macro::{EventElement, PortableWidget, Rebuildable};
use aimer_style::{LayoutSpacing, Spacing};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, LayoutElement, RequiredChild, VisitorElement, Widget,
};

/// Adds transparent insets around one child.
///
/// The wrapper has no background, border, decoration, or clipping behavior. It
/// only resolves [`LayoutSpacing`], reduces the child's constraints, and
/// translates the child's layout and painting origin.
///
/// # Example
///
/// ```
/// use aimer_container::{Padding, ZeroSizedBox};
/// use aimer_style::LayoutSpacing;
/// use aimer_widget::Widget;
///
/// let widget = Padding::new()
///     .padding(LayoutSpacing::all(8))
///     .child(ZeroSizedBox);
/// let _erased = widget.boxed();
/// ```
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_container::single_child::Padding")]
pub struct Padding<W = RequiredChild> {
    #[portable_optional]
    padding: LayoutSpacing,
    #[portable_child]
    child: W,
}

impl Default for Padding {
    fn default() -> Self {
        Self::new()
    }
}

impl Padding {
    /// Creates a zero-inset padding builder.
    ///
    /// Finish the builder with [`Padding::child`] or [`Padding::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            padding: LayoutSpacing::default(),
            child: RequiredChild,
        }
    }

    /// Replaces the four transparent insets around the child.
    ///
    /// Use [`LayoutSpacing::all`], [`LayoutSpacing::vertical`],
    /// [`LayoutSpacing::horizontal`], or the side-specific builders to create
    /// the value. Insets use the build context's scale and parent constraint.
    #[inline]
    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Attaches the required child and preserves its concrete type.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Padding<W> {
        Padding {
            padding: self.padding,
            child,
        }
    }

    /// Attaches `child` and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

/// Adds transparent outer spacing around one child.
///
/// The wrapper has no background, border, decoration, or clipping behavior. It
/// only resolves [`LayoutSpacing`], reduces the child's constraints, and
/// translates the child's layout and painting origin.
///
/// # Example
///
/// ```
/// use aimer_container::{Margin, ZeroSizedBox};
/// use aimer_style::LayoutSpacing;
/// use aimer_widget::Widget;
///
/// let widget = Margin::new()
///     .margin(LayoutSpacing::all(8))
///     .child(ZeroSizedBox);
/// let _erased = widget.boxed();
/// ```
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_container::single_child::Margin")]
pub struct Margin<W = RequiredChild> {
    #[portable_optional]
    margin: LayoutSpacing,
    #[portable_child]
    child: W,
}

impl Default for Margin {
    fn default() -> Self {
        Self::new()
    }
}

impl Margin {
    /// Creates a zero-inset margin builder.
    ///
    /// Finish the builder with [`Margin::child`] or [`Margin::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            margin: LayoutSpacing::default(),
            child: RequiredChild,
        }
    }

    /// Replaces the four transparent outer insets around the child.
    ///
    /// Use [`LayoutSpacing::all`], [`LayoutSpacing::vertical`],
    /// [`LayoutSpacing::horizontal`], or the side-specific builders to create
    /// the value. Insets use the build context's scale and parent constraint.
    #[inline]
    pub fn margin(mut self, margin: LayoutSpacing) -> Self {
        self.margin = margin;
        self
    }

    /// Attaches the required child and preserves its concrete type.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Margin<W> {
        Margin {
            margin: self.margin,
            child,
        }
    }

    /// Attaches `child` and erases the completed widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> Widget for Padding<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawPadding {
            padding: self.padding,
            child: self.child.to_element(ctx),
        }
        .boxed()
    }
}

impl<W: Widget + 'static> Widget for Margin<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawMargin {
            margin: self.margin,
            child: self.child.to_element(ctx),
        }
        .boxed()
    }
}

#[derive(EventElement, Rebuildable)]
struct RawPadding {
    padding: LayoutSpacing,
    child: AnyElement,
}

#[derive(EventElement, Rebuildable)]
struct RawMargin {
    margin: LayoutSpacing,
    child: AnyElement,
}

#[derive(Clone, Copy)]
struct ResolvedSpacing {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl ResolvedSpacing {
    #[inline]
    fn horizontal(self) -> f32 {
        self.left + self.right
    }

    #[inline]
    fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

#[inline]
fn resolve_spacing(spacing: LayoutSpacing, width: f32, height: f32, scale: f32) -> ResolvedSpacing {
    ResolvedSpacing {
        left: spacing.left.value(width, scale),
        top: spacing.top.value(height, scale),
        right: spacing.right.value(width, scale),
        bottom: spacing.bottom.value(height, scale),
    }
}

#[inline]
fn spacing_basis(value: f32) -> f32 {
    value.max(0.0).min(1_000_000.0)
}

#[inline]
fn child_constraint(ctx: &BuildContext, spacing: ResolvedSpacing) -> BoxConstraint {
    let mut constraint = ctx.box_constraint;
    constraint.max_width = (constraint.max_width - spacing.horizontal()).max(0.0);
    constraint.max_height = (constraint.max_height - spacing.vertical()).max(0.0);
    constraint
}

#[inline]
fn translated_context<'a>(ctx: &BuildContext<'a>, spacing: ResolvedSpacing) -> BuildContext<'a> {
    let mut child_ctx = ctx.clone();
    child_ctx.box_constraint = child_constraint(ctx, spacing);
    child_ctx.parent_size = ResolvedSize {
        width: child_ctx.box_constraint.max_width,
        height: child_ctx.box_constraint.max_height,
    };
    child_ctx.visible_rect = ctx
        .visible_rect
        .map(|(x, y, width, height)| (x - spacing.left, y - spacing.top, width, height));
    child_ctx
}

#[inline]
fn add_spacing_to_size(mut size: Size, spacing: LayoutSpacing) -> Size {
    let width = match (size.width, spacing.left, spacing.right) {
        (aimer_attribute::Dimension::Px(value), Spacing::Px(left), Spacing::Px(right)) => {
            aimer_attribute::Dimension::Px(value + left as f32 + right as f32)
        }
        (value, _, _) => value,
    };
    let height = match (size.height, spacing.top, spacing.bottom) {
        (aimer_attribute::Dimension::Px(value), Spacing::Px(top), Spacing::Px(bottom)) => {
            aimer_attribute::Dimension::Px(value + top as f32 + bottom as f32)
        }
        (value, _, _) => value,
    };
    size.width = width;
    size.height = height;
    size
}

#[inline]
fn is_unbounded(value: f32) -> bool {
    value > 1_000_000.0
}

macro_rules! impl_spacing_element {
    ($raw:ident, $field:ident, $name:literal) => {
        impl Drawable for $raw {
            fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
                let spacing = self.resolved(ctx);
                let size = self.computed_size(ctx);
                ctx.scale.is_finite()
                    && ctx.scale > 0.0
                    && spacing.left.is_finite()
                    && spacing.top.is_finite()
                    && spacing.right.is_finite()
                    && spacing.bottom.is_finite()
                    && spacing.horizontal().is_finite()
                    && spacing.vertical().is_finite()
                    && size.width.is_finite()
                    && size.height.is_finite()
                    && size.width >= 0.0
                    && size.height >= 0.0
                    && size.width <= 1_000_000.0
                    && size.height <= 1_000_000.0
            }

            fn retained_v2_child_context<'a>(
                &self,
                ctx: &BuildContext<'a>,
                _child: &dyn Element,
            ) -> Option<BuildContext<'a>> {
                if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
                    return None;
                }
                Some(translated_context(ctx, self.resolved(ctx)))
            }

            fn retained_v2_child_geometry(
                &self,
                ctx: &BuildContext,
                child: &dyn Element,
            ) -> Option<(
                aimer_cupid::draw_cmd_v2::Rect,
                Option<aimer_cupid::draw_cmd_v2::Rect>,
            )> {
                if !ctx.scale.is_finite() || ctx.scale <= 0.0 {
                    return None;
                }
                let spacing = self.resolved(ctx);
                let child_ctx = translated_context(ctx, spacing);
                let size = child
                    .retained_v2_bounds(&child_ctx)
                    .unwrap_or_else(|| child.content_size(&child_ctx));
                let position = child.pos().unwrap_or_default();
                let bounds = aimer_cupid::draw_cmd_v2::Rect::new(
                    (spacing.left + position.x) / ctx.scale,
                    (spacing.top + position.y) / ctx.scale,
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

            fn update(&self, ctx: &BuildContext) {
                let spacing = self.resolved(ctx);
                let child_ctx = translated_context(ctx, spacing);
                ctx.canvas.save();
                ctx.canvas.translate(Vec2d {
                    x: spacing.left,
                    y: spacing.top,
                });
                self.child.update(&child_ctx);
                ctx.canvas.restore();
            }

            #[inline]
            fn sync_paint_geometry(&self, ctx: &BuildContext) {
                let spacing = self.resolved(ctx);
                let child_ctx = translated_context(ctx, spacing);
                ctx.canvas.save();
                ctx.canvas.translate(Vec2d {
                    x: spacing.left,
                    y: spacing.top,
                });
                self.child.sync_paint_geometry(&child_ctx);
                ctx.canvas.restore();
            }
        }

        impl LayoutElement for $raw {
            #[inline]
            fn is_layout_stable(&self) -> bool {
                self.child.is_layout_stable()
            }

            fn size(&self) -> Option<Size> {
                Some(Size::default())
            }

            fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
                let spacing = self.resolved(ctx);
                let width_unbounded = is_unbounded(ctx.box_constraint.max_width);
                let height_unbounded = is_unbounded(ctx.box_constraint.max_height);
                if !width_unbounded && !height_unbounded {
                    return ResolvedSize {
                        width: ctx.box_constraint.max_width.max(0.0),
                        height: ctx.box_constraint.max_height.max(0.0),
                    };
                }

                let child_ctx = translated_context(ctx, spacing);
                let child_size = self.child.computed_size(&child_ctx);
                ResolvedSize {
                    width: if width_unbounded {
                        child_size.width + spacing.horizontal()
                    } else {
                        ctx.box_constraint.max_width.max(0.0)
                    },
                    height: if height_unbounded {
                        child_size.height + spacing.vertical()
                    } else {
                        ctx.box_constraint.max_height.max(0.0)
                    },
                }
            }

            fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
                let spacing = self.resolved(ctx);
                self.child.content_size(&translated_context(ctx, spacing))
            }

            fn layer(&self) -> u32 {
                self.child.layer()
            }

            fn flex(&self) -> Option<f32> {
                self.child.flex()
            }

            fn get_size_from_child(&self) -> Option<Size> {
                Some(add_spacing_to_size(
                    self.child.get_size_from_child().unwrap_or_default(),
                    self.$field,
                ))
            }

            fn invalidate_layout(&self) {
                self.child.invalidate_layout();
            }

            fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
                self.child.pos_start_end()
            }
        }

        impl VisitorElement for $raw {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(self.child.as_ref());
            }

            fn debug_name(&self) -> &'static str {
                $name
            }
        }

        impl $raw {
            #[inline]
            fn resolved(&self, ctx: &BuildContext) -> ResolvedSpacing {
                resolve_spacing(
                    self.$field,
                    spacing_basis(ctx.box_constraint.max_width),
                    spacing_basis(ctx.box_constraint.max_height),
                    ctx.scale,
                )
            }
        }
    };
}

impl_spacing_element!(RawPadding, padding, "Padding");
impl_spacing_element!(RawMargin, margin, "Margin");

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_attribute::size::ResolvedSize;
    use aimer_widget::base::WindowHandle;
    use aimer_widget::{EventElement, PortableWidget, Rebuildable};

    use super::*;

    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Observation {
        max_width: f32,
        max_height: f32,
        parent_width: f32,
        parent_height: f32,
        translation_x: f32,
        translation_y: f32,
    }

    struct Probe {
        size: ResolvedSize,
        observation: Rc<Cell<Observation>>,
        draws: Rc<Cell<usize>>,
    }

    impl Drawable for Probe {
        fn update(&self, ctx: &BuildContext) {
            let (translation_x, translation_y) = ctx.canvas.get_transform_translation();
            self.observation.set(Observation {
                max_width: ctx.box_constraint.max_width,
                max_height: ctx.box_constraint.max_height,
                parent_width: ctx.parent_size.width,
                parent_height: ctx.parent_size.height,
                translation_x,
                translation_y,
            });
            self.draws.set(self.draws.get() + 1);
        }
    }

    impl EventElement for Probe {}
    impl Rebuildable for Probe {}
    impl PortableWidget for Probe {}

    impl VisitorElement for Probe {
        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    impl LayoutElement for Probe {
        fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.size
        }
    }

    impl Widget for Probe {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Element::boxed(self)
        }
    }

    fn context(max_width: f32, max_height: f32, scale: f32) -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        let mut ctx = BuildContext::new(
            canvas,
            ResolvedSize {
                width: max_width,
                height: max_height,
            },
            scale,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), scale as f64),
            tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width,
            max_height,
        };
        ctx
    }

    fn probe(observation: &Rc<Cell<Observation>>, draws: &Rc<Cell<usize>>) -> Probe {
        Probe {
            size: ResolvedSize {
                width: 20.0,
                height: 10.0,
            },
            observation: observation.clone(),
            draws: draws.clone(),
        }
    }

    #[tokio::test]
    async fn padding_adds_all_sides_and_reduces_unbounded_child_constraints() {
        let observation = Rc::new(Cell::new(Observation::default()));
        let draws = Rc::new(Cell::new(0));
        let ctx = context(f32::MAX, f32::MAX, 1.0);
        let element = Padding::new()
            .padding(LayoutSpacing::all(8))
            .child(probe(&observation, &draws))
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 36.0,
                height: 26.0,
            }
        );

        element.update(&ctx);
        let observed = observation.get();
        assert_eq!(draws.get(), 1);
        assert_eq!(observed.max_width, f32::MAX - 16.0);
        assert_eq!(observed.max_height, f32::MAX - 16.0);
        assert_eq!(observed.translation_x, 8.0);
        assert_eq!(observed.translation_y, 8.0);
    }

    #[tokio::test]
    async fn margin_adds_all_sides_to_unbounded_layout_footprint() {
        let observation = Rc::new(Cell::new(Observation::default()));
        let draws = Rc::new(Cell::new(0));
        let ctx = context(f32::MAX, f32::MAX, 1.0);
        let element = Margin::new()
            .margin(LayoutSpacing::all(8))
            .child(probe(&observation, &draws))
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 36.0,
                height: 26.0,
            }
        );

        element.update(&ctx);
        let observed = observation.get();
        assert_eq!(draws.get(), 1);
        assert_eq!(observed.max_width, f32::MAX - 16.0);
        assert_eq!(observed.max_height, f32::MAX - 16.0);
        assert_eq!(observed.translation_x, 8.0);
        assert_eq!(observed.translation_y, 8.0);
    }

    #[tokio::test]
    async fn bounded_axes_fill_the_parent_and_reduce_child_constraints() {
        let observation = Rc::new(Cell::new(Observation::default()));
        let draws = Rc::new(Cell::new(0));
        let ctx = context(100.0, 80.0, 2.0);
        let element = Padding::new()
            .padding(LayoutSpacing::all(8))
            .child(probe(&observation, &draws))
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 100.0,
                height: 80.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.max_width, 68.0);
        assert_eq!(observed.max_height, 48.0);
        assert_eq!(observed.parent_width, 68.0);
        assert_eq!(observed.parent_height, 48.0);
        assert_eq!(observed.translation_x, 16.0);
        assert_eq!(observed.translation_y, 16.0);
    }

    #[tokio::test]
    async fn zero_spacing_is_a_transparent_pass_through() {
        let observation = Rc::new(Cell::new(Observation::default()));
        let draws = Rc::new(Cell::new(0));
        let ctx = context(f32::MAX, f32::MAX, 1.0);
        let element = Padding::new()
            .child(probe(&observation, &draws))
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(draws.get(), 1);
        assert_eq!(observed.max_width, f32::MAX);
        assert_eq!(observed.max_height, f32::MAX);
        assert_eq!(observed.translation_x, 0.0);
        assert_eq!(observed.translation_y, 0.0);
    }

    #[tokio::test]
    async fn one_scrollable_axis_is_intrinsic_and_the_other_remains_bounded() {
        let observation = Rc::new(Cell::new(Observation::default()));
        let draws = Rc::new(Cell::new(0));
        let ctx = context(f32::MAX, 80.0, 1.0);
        let element = Margin::new()
            .margin(LayoutSpacing::all(8))
            .child(probe(&observation, &draws))
            .to_element(&ctx);

        assert_eq!(
            element.computed_size(&ctx),
            ResolvedSize {
                width: 36.0,
                height: 80.0,
            }
        );
        element.update(&ctx);

        let observed = observation.get();
        assert_eq!(observed.max_width, f32::MAX - 16.0);
        assert_eq!(observed.max_height, 64.0);
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::*;
    use aimer_widget::portable::PortableWidgetSchema;

    #[test]
    fn spacing_schemas_have_one_optional_property_and_one_required_child() {
        let padding = <Padding<RequiredChild> as PortableWidgetSchema>::SCHEMA;
        assert_eq!(padding.children().minimum(), 1);
        assert_eq!(padding.children().maximum(), 1);
        assert_eq!(padding.properties().len(), 1);
        assert_eq!(
            padding.properties()[0].canonical_name(),
            "aimer.property:aimer_container::single_child::Padding:padding"
        );
        assert!(padding.properties()[0].is_optional());

        let margin = <Margin<RequiredChild> as PortableWidgetSchema>::SCHEMA;
        assert_eq!(margin.children().minimum(), 1);
        assert_eq!(margin.children().maximum(), 1);
        assert_eq!(margin.properties().len(), 1);
        assert_eq!(
            margin.properties()[0].canonical_name(),
            "aimer.property:aimer_container::single_child::Margin:margin"
        );
        assert!(margin.properties()[0].is_optional());
    }
}
