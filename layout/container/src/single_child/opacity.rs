use aimer_attribute::size::{ResolvedSize, Size};
use aimer_macro::{EventElement, Rebuildable};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, LayoutElement, RequiredChild, VisitorElement, Widget,
};

/// Paints a single child with a normalized alpha value.
///
/// Attach a child with [`Opacity::child`] to retain its concrete type, or with
/// [`Opacity::box_child`] when branches need a shared erased type.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_container::single_child::Opacity")]
pub struct Opacity<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_optional]
    opacity: f32,
}

impl Opacity {
    /// Creates a fully opaque builder.
    ///
    /// Finish the builder with [`Opacity::child`] or [`Opacity::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            opacity: 1.0,
        }
    }

    /// Sets the alpha multiplier applied while painting the child.
    ///
    /// The default is `1.0`. Finite values are clamped to the inclusive
    /// `0.0..=1.0` range, and `NaN` is normalized to `1.0`. Layout is
    /// unaffected.
    #[inline]
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = normalized_opacity(opacity);
        self
    }

    /// Attaches the required child and completes this builder.
    ///
    /// The concrete child type is preserved and all opacity configuration is
    /// retained. Use [`Opacity::box_child`] for branch type erasure.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Opacity<W> {
        Opacity {
            child,
            opacity: self.opacity,
        }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    ///
    /// This is equivalent to calling [`Opacity::child`] followed by
    /// [`Widget::boxed`]. Use it when different branches must return one
    /// [`AnyWidget`] type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for Opacity {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> Widget for Opacity<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawOpacity {
            child: self.child.to_element(ctx),
            opacity: normalized_opacity(self.opacity),
        }
        .boxed()
    }
}

fn normalized_opacity(opacity: f32) -> f32 {
    if opacity.is_nan() {
        1.0
    } else {
        opacity.clamp(0.0, 1.0)
    }
}

#[derive(EventElement, Rebuildable)]
#[rebuildable(forwards_marks_only)]
struct RawOpacity {
    child: AnyElement,
    opacity: f32,
}

impl Drawable for RawOpacity {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// The alpha is the child node's presentation, so the child keeps its
    /// recorded paint and the render tree dims whatever it shows.
    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        self.child.sync_paint_geometry(ctx);
        ctx.set_local_v2_child_presentation_at(0, aimer_canvas::Mat3::identity(), self.opacity)
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }
}

impl LayoutElement for RawOpacity {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
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

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl VisitorElement for RawOpacity {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Opacity"
    }
}

#[cfg(test)]
mod tests {
    use super::normalized_opacity;

    #[test]
    fn opacity_is_clamped_to_valid_canvas_range() {
        assert_eq!(normalized_opacity(-0.25), 0.0);
        assert_eq!(normalized_opacity(0.4), 0.4);
        assert_eq!(normalized_opacity(1.25), 1.0);
        assert_eq!(normalized_opacity(f32::NAN), 1.0);
    }
}
