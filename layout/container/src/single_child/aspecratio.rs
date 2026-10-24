use aimer_attribute::size::ResolvedSize;
use aimer_attribute::{BoxConstraint, Size};
use aimer_macro::{EventElement, PortableValue, PortableWidget, Rebuildable};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, LayoutElement, RequiredChild, VisitorElement, Widget,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_container::single_child::RatioOption",
    max_encoded_bytes = 16
)]
pub enum RatioOption {
    #[portable_value(tag = 0)]
    Width,
    #[portable_value(tag = 1)]
    Height,
}

impl Default for RatioOption {
    fn default() -> Self {
        Self::Width
    }
}

#[allow(dead_code)]
/// Sizes its child to a configured width-to-height ratio within its
/// constraints.
///
/// Attach a child with [`AspectRatio::child`] to retain its concrete type, or
/// with [`AspectRatio::box_child`] when branches need a shared erased type.
#[derive(PortableWidget)]
#[portable_widget(id = "aimer_container::single_child::AspectRatio")]
pub struct AspectRatio<W = RequiredChild> {
    #[portable_optional]
    pub aspect_ratio: f32,
    #[portable_optional]
    ratio_option: RatioOption,
    #[portable_child]
    pub child: W,
}

impl AspectRatio {
    /// Creates an aspect-ratio builder with a `1.0` width-driven ratio.
    ///
    /// Finish the builder with [`AspectRatio::child`] or
    /// [`AspectRatio::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            aspect_ratio: 1.0,
            child: RequiredChild,
            ratio_option: RatioOption::Width,
        }
    }

    /// Sets the desired width divided by height.
    ///
    /// The default is `1.0`. The value is made positive when the widget is
    /// built; non-finite or zero values resolve as `1.0` during layout. The
    /// resulting size is clamped to the parent's minimum and maximum
    /// constraints.
    #[inline]
    pub fn aspect_ratio(mut self, aspect_ratio: f32) -> Self {
        self.aspect_ratio = aspect_ratio;
        self
    }

    /// Selects which bounded axis drives ratio calculation.
    ///
    /// [`RatioOption::Width`] is the default. If the preferred axis is
    /// unbounded, layout falls back to the bounded axis and still honors all
    /// parent constraints.
    #[inline]
    pub fn ratio_option(mut self, ratio_option: RatioOption) -> Self {
        self.ratio_option = ratio_option;
        self
    }

    /// Attaches the required child and completes this builder.
    ///
    /// The child receives the ratio-constrained size, and its concrete type is
    /// preserved. Use [`AspectRatio::box_child`] for branch type erasure.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> AspectRatio<C> {
        AspectRatio {
            aspect_ratio: self.aspect_ratio,
            ratio_option: self.ratio_option,
            child,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`AspectRatio::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for AspectRatio {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget> Widget for AspectRatio<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawAspectRatio {
            child: self.child.to_element(ctx),
            aspect_ratio: self.aspect_ratio.abs(),
            ratio_option: self.ratio_option,
        }
        .boxed()
    }
}

#[allow(dead_code)]
fn resolve_ratio_size(constraints: BoxConstraint, aspect_ratio: f32) -> (f32, f32) {
    resolve_ratio_size_with_option(constraints, aspect_ratio, RatioOption::Width)
}
#[inline]
fn resolve_ratio_size_with_option(
    constraints: BoxConstraint,
    aspect_ratio: f32,
    ratio_option: RatioOption,
) -> (f32, f32) {
    let ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        1.0
    };
    let width_bounded = constraints.max_width.is_finite() && constraints.max_width < f32::MAX;
    let height_bounded = constraints.max_height.is_finite() && constraints.max_height < f32::MAX;

    let (mut width, mut height) = if matches!(ratio_option, RatioOption::Width) && width_bounded {
        let width = constraints.max_width;
        let height = width / ratio;
        if height_bounded && height > constraints.max_height {
            (constraints.max_height * ratio, constraints.max_height)
        } else {
            (width, height)
        }
    } else if height_bounded {
        let height = constraints.max_height;
        let width = height * ratio;
        if width_bounded && width > constraints.max_width {
            (constraints.max_width, constraints.max_width / ratio)
        } else {
            (width, height)
        }
    } else if width_bounded {
        (constraints.max_width, constraints.max_width / ratio)
    } else if constraints.min_width > 0.0 {
        (constraints.min_width, constraints.min_width / ratio)
    } else {
        (constraints.min_height * ratio, constraints.min_height)
    };

    if width < constraints.min_width {
        width = constraints.min_width;
        height = width / ratio;
    }
    if height < constraints.min_height {
        height = constraints.min_height;
        width = height * ratio;
    }

    (
        width.min(constraints.max_width),
        height.min(constraints.max_height),
    )
}

#[derive(EventElement, Rebuildable)]
struct RawAspectRatio {
    child: AnyElement,
    aspect_ratio: f32,
    ratio_option: RatioOption,
}

impl Drawable for RawAspectRatio {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let size = self.computed_size(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
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
        let size = self.computed_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        Some(child_ctx)
    }

    fn update(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        self.child.update(&child_ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let size = self.computed_size(ctx);
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: size.width,
            max_height: size.height,
        };
        self.child.sync_paint_geometry(&child_ctx);
    }
}

impl LayoutElement for RawAspectRatio {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let (width, height) = resolve_ratio_size_with_option(
            ctx.box_constraint,
            self.aspect_ratio,
            self.ratio_option,
        );
        ResolvedSize { width, height }
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl VisitorElement for RawAspectRatio {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AspectRatio"
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::BoxConstraint;

    use super::{RatioOption, resolve_ratio_size, resolve_ratio_size_with_option};

    #[test]
    fn ratio_size_uses_largest_size_inside_constraints() {
        let constraints = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 320.0,
            max_height: 200.0,
        };

        assert_eq!(resolve_ratio_size(constraints, 16.0 / 9.0), (320.0, 180.0));
        assert_eq!(resolve_ratio_size(constraints, 0.5), (100.0, 200.0));
    }

    #[test]
    fn ratio_size_honors_minimum_constraints() {
        let constraints = BoxConstraint {
            min_width: 150.0,
            min_height: 100.0,
            max_width: 300.0,
            max_height: 300.0,
        };

        assert_eq!(resolve_ratio_size(constraints, 2.0), (300.0, 150.0));
    }

    #[test]
    fn height_driven_ratio_still_fits_the_width_constraint() {
        let constraints = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: 300.0,
            max_height: 200.0,
        };

        assert_eq!(
            resolve_ratio_size_with_option(constraints, 2.0, RatioOption::Height),
            (300.0, 150.0)
        );
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::{AspectRatio, RatioOption};
    use aimer_widget::RequiredChild;
    use aimer_widget::portable::{PortableValue, PortableWidgetSchema};

    #[test]
    fn aspect_ratio_schema_reflects_both_layout_controls() {
        let schema = <AspectRatio<RequiredChild> as PortableWidgetSchema>::SCHEMA;

        assert_eq!(
            schema.widget().canonical_name(),
            "aimer.widget:aimer_container::single_child::AspectRatio"
        );
        assert_eq!(schema.properties().len(), 2);
        assert_eq!(
            schema.properties()[0].canonical_name(),
            "aimer.property:aimer_container::single_child::AspectRatio:aspect_ratio"
        );
        assert_eq!(
            schema.properties()[1].canonical_name(),
            "aimer.property:aimer_container::single_child::AspectRatio:ratio_option"
        );
        assert_eq!(
            schema.children(),
            aimer_anteros::ChildCardinality::exactly(1)
        );
        assert_eq!(
            <RatioOption as PortableValue>::SCHEMA.canonical_name(),
            "aimer.value:aimer_container::single_child::RatioOption"
        );
    }
}
