use super::*;
use aimer_macro::PortableWidget;
use aimer_widget::portable::__anteros::{WidgetDocumentView, WidgetNodeView};
use aimer_widget::portable::{
    PortableMaterializeError, PortableNativeWidget, PortableWidgetSchema,
};

// The built-in wire schema remains concrete. Native containers retain their
// decoration's type; arbitrary application paint implementations have no codec.
#[derive(PortableWidget)]
#[portable_widget(
    id = "aimer_container::single_child::Container",
    validate = validate_portable_container
)]
struct PortableContainer<T = RequiredChild> {
    #[portable_optional]
    width: Dimension,
    #[portable_optional]
    height: Dimension,
    #[portable_optional]
    padding: LayoutSpacing,
    #[portable_optional]
    margin: LayoutSpacing,
    #[portable_optional]
    box_decoration: BoxDecoration,
    #[portable_skip]
    decoration_source: Option<Rc<dyn RetainedBoxDecoration>>,
    color: Option<Color>,
    #[portable_child]
    child: T,
}

impl PortableContainer {
    // The generated materializer builds the public widget directly.
    fn new() -> Container {
        Container::new()
    }
}

impl<T: Widget> Widget for PortableContainer<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Container {
            width: self.width,
            height: self.height,
            padding: self.padding,
            margin: self.margin,
            box_decoration: self.box_decoration,
            decoration_source: self.decoration_source,
            color: self.color,
            child: self.child,
        }
        .to_element(ctx)
    }
}

impl<T, D: Decoration + 'static> PortableWidgetSchema for Container<T, D> {
    const SCHEMA: aimer_widget::portable::__anteros::PortableWidgetSchemaMetadata<'static> =
        <PortableContainer<T> as PortableWidgetSchema>::SCHEMA;
}

impl PortableNativeWidget for Container {
    fn materialize_widget(
        document: &WidgetDocumentView<'_>,
        node: WidgetNodeView<'_>,
        children: Vec<AnyWidget>,
    ) -> Result<AnyWidget, PortableMaterializeError> {
        <PortableContainer as PortableNativeWidget>::materialize_widget(document, node, children)
    }
}

impl<T: Widget, D: Decoration + 'static> aimer_widget::PortableWidget for Container<T, D> {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut PortableBuildContext,
        source: SourceFingerprint,
    ) -> Result<aimer_widget::portable::PortableNodeId, PortableBuildError> {
        // Type inspection and cloning occur only during guest serialization,
        // never on native rebuild, layout or paint paths.
        let decoration = (&self.box_decoration as &dyn std::any::Any)
            .downcast_ref::<BoxDecoration>()
            .ok_or_else(|| ctx.unsupported_widget("Container.decoration", source))?
            .clone();
        aimer_widget::PortableWidget::to_portable_node(
            PortableContainer {
                width: self.width,
                height: self.height,
                padding: self.padding,
                margin: self.margin,
                box_decoration: decoration,
                decoration_source: self.decoration_source,
                color: self.color,
                child: self.child,
            },
            ctx,
            source,
        )
    }
}

#[cfg(feature = "portable-guest")]
fn validate_portable_container<W: Widget>(
    container: &PortableContainer<W>,
    ctx: &PortableBuildContext,
    source: SourceFingerprint,
) -> Result<(), PortableBuildError> {
    if container.decoration_source.is_some() {
        return Err(ctx.unsupported_widget("Container.retained_box_decoration", source));
    }
    for (dimension, property) in [
        (container.width, "Container.width"),
        (container.height, "Container.height"),
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
