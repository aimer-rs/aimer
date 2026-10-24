use aimer_attribute::ResolvedSize;
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{Container, SizedBox};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, Widget};

// Keep this as an integration test: unit binaries also link a second copy of
// their crate through dev-dependencies, which duplicates linker registrations.
#[tokio::test]
async fn linked_container_materializer_preserves_the_box_decoration_wire_contract() {
    use aimer_widget::portable::__anteros::{
        ModelLimits, PropertyId, PropertyValue, Version, WidgetDocument, WidgetDocumentView,
        WidgetNode, WidgetProperty, WidgetSchemaId,
    };
    use aimer_widget::portable::{
        PortableWidgetSchema, linked_portable_native_widget_registrations,
    };
    let widget_type = <Container as PortableWidgetSchema>::SCHEMA.widget().id();
    let registrations: Vec<_> = linked_portable_native_widget_registrations()
        .iter()
        .filter(|registration| registration.widget_type() == widget_type)
        .collect();
    assert_eq!(
        registrations.len(),
        1,
        "the built-in container must have one linked constructor"
    );
    let property = |name: &str| PropertyId::from_canonical_name(name);
    let properties = [
        WidgetProperty::new(
            property("aimer.property:aimer_container::single_child::Container:width"),
            PropertyValue::BlobRef(0),
        ),
        WidgetProperty::new(
            property("aimer.property:aimer_container::single_child::Container:height"),
            PropertyValue::BlobRef(1),
        ),
        WidgetProperty::new(
            property("aimer.property:aimer_container::single_child::Container:box_decoration"),
            PropertyValue::BlobRef(2),
        ),
    ];
    let width = [1, 1, 0, 0, 160, 66]; // version one, Px(80)
    let height = [1, 1, 0, 0, 32, 66]; // version one, Px(40)
    let mut decoration = vec![1];
    for _ in 0..4 {
        decoration.extend_from_slice(&[1, 1]); // solid, pixels
        decoration.extend_from_slice(&4.0f32.to_le_bytes());
        decoration.extend_from_slice(&Color::BLACK.as_u32().to_le_bytes());
    }
    for _ in 0..4 {
        decoration.extend_from_slice(&[0, 0]); // no outline, auto
        decoration.extend_from_slice(&0.0f32.to_le_bytes());
        decoration.extend_from_slice(&Color::Transparent.as_u32().to_le_bytes());
    }
    for _ in 0..4 {
        decoration.push(1); // pixel radius
        decoration.extend_from_slice(&12.0f32.to_le_bytes());
    }
    decoration.extend_from_slice(&0u32.to_le_bytes()); // no shadows
    decoration.push(1); // background present
    decoration.extend_from_slice(&Color::BLUE.as_u32().to_le_bytes());
    let children = [1];
    let nodes = [
        WidgetNode::new(widget_type, Version::new(1, 0))
            .properties(&properties)
            .children(&children),
        WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0)),
    ];
    let limits = ModelLimits::new(4_096, 4, 64, 4_096);
    let image = WidgetDocument::new(1, 1, 0, &nodes, &[], &[&width, &height, &decoration])
        .encode(limits)
        .unwrap();
    let document = WidgetDocumentView::decode(&image, limits).unwrap();
    let widget = (registrations[0].materialize())(
        &document,
        document.node(0).unwrap(),
        vec![SizedBox::new().boxed()],
    )
    .unwrap();
    let (ctx, _) = recording_context();
    let element = widget.to_element(&ctx);
    assert_eq!(element.debug_name(), "Container");
    assert_eq!(
        element.content_size(&ctx),
        ResolvedSize {
            width: 72.0,
            height: 32.0
        }
    );
    element.visit_children(&mut |child| {
        assert_eq!(element.retained_v2_child_clip_radius(&ctx, child), [8.0; 4]);
    });
    assert!(matches!(record_local_v2(&element, &ctx).as_ref(),
        [aimer_cupid::draw_cmd_v2::DrawCommand::FillRect { color, border_radius, .. }]
        if color.b == 255 && color.a == 255 && *border_radius == [12.0; 4]));
}

fn recording_context() -> (BuildContext<'static>, &'static InnerCanvas) {
    let inner = Box::leak(Box::new(InnerCanvas::new()));
    let ctx = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    );
    (ctx, inner)
}

fn record_local_v2(
    element: &impl Drawable,
    ctx: &BuildContext,
) -> std::sync::Arc<[aimer_cupid::draw_cmd_v2::DrawCommand]> {
    use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        element.paint_local_v2(ctx)
    });
    tree.draw_list_snapshot(root).unwrap().commands
}

#[cfg(feature = "portable-guest")]
#[test]
fn custom_container_decoration_rejects_portable_lowering() {
    use aimer_decoration::BackdropFilter;
    use aimer_widget::PortableWidget;
    use aimer_widget::portable::{
        PortableBuildContext, PortableBuildError, PortableLimits, PortableWidgetLimits,
        SourceFingerprint, StableId128,
    };
    let mut ctx = PortableBuildContext::new(
        7,
        11,
        PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
        PortableLimits::new(8, 16, 64, 128, 1_024),
    )
    .unwrap();
    let source = SourceFingerprint::new(StableId128::from_path("test", "container-decoration"));
    let result = Container::new()
        .decoration(BackdropFilter::new())
        .child(SizedBox::new())
        .to_portable_node(&mut ctx, source);
    assert!(matches!(result, Err(PortableBuildError::UnsupportedWidget {
        widget: "Container.decoration", source: actual,
    }) if actual == source));
}
