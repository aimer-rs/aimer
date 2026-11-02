#![cfg(feature = "portable-guest")]

use aimer_anteros::{Version, WidgetDocumentView, WidgetSchemaId};
use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::input::{TextArea, TextField};
use aimer_input::mouse_region::MouseRegion;
use aimer_style::{BoxDecoration, LayoutSpacing, Spacing};
use aimer_widget::base::BuildContext;
use aimer_widget::base::Color;
use aimer_widget::portable::{
    PortableBuildContext, PortableBuildError, PortableLimits, PortableNativeWidget, PortableNodeId,
    PortableWidgetLimits, PortableWidgetSchema, SourceFingerprint, StableId128,
};
use aimer_widget::{AnyElement, PortableWidget, Widget};

struct PortableLeaf;

impl Widget for PortableLeaf {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        panic!("portable test leaf must not enter native construction")
    }
}

impl PortableWidget for PortableLeaf {
    fn to_portable_node(
        self,
        ctx: &mut PortableBuildContext,
        source: SourceFingerprint,
    ) -> Result<PortableNodeId, PortableBuildError> {
        ctx.push_node(
            WidgetSchemaId::new(0x504f525441424c45),
            Version::new(1, 0),
            None,
            source,
            &[],
            &[],
        )
    }
}

fn source(value: u8) -> SourceFingerprint {
    SourceFingerprint::new(StableId128::from_bytes([value; 16]))
}

fn context() -> PortableBuildContext {
    PortableBuildContext::new(
        8,
        3,
        PortableWidgetLimits::new(8, 32, 8, 8, 1_024, 16_384)
            .with_max_callbacks(16)
            .with_max_blob_bytes(8_192),
        PortableLimits::new(8, 16, 64, 128, 16_384),
    )
    .unwrap()
}

#[test]
fn input_widgets_publish_derived_schema_contracts() {
    fn assert_native_materializer<T: aimer_widget::portable::PortableNativeWidget>() {}

    assert_native_materializer::<TextField>();
    assert_native_materializer::<TextArea>();

    let gesture = <GestureDetector<PortableLeaf> as PortableWidgetSchema>::SCHEMA;
    assert_eq!(
        gesture.widget().canonical_name(),
        "aimer.widget:aimer_input::GestureDetector"
    );
    assert_eq!(gesture.children().minimum(), 1);
    assert_eq!(gesture.children().maximum(), 1);
    assert_eq!(gesture.callbacks().len(), 5);

    let field = <TextField as PortableWidgetSchema>::SCHEMA;
    assert_eq!(
        field.widget().canonical_name(),
        "aimer.widget:aimer_input::TextField"
    );
    assert_eq!(field.children().maximum(), 0);
    assert_eq!(field.properties().len(), 10);

    let area = <TextArea as PortableWidgetSchema>::SCHEMA;
    assert_eq!(
        area.widget().canonical_name(),
        "aimer.widget:aimer_input::TextArea"
    );
    assert_eq!(area.children().maximum(), 0);
    assert_eq!(area.properties().len(), 3);

    let mouse = <MouseRegion<PortableLeaf> as PortableWidgetSchema>::SCHEMA;
    assert_eq!(
        mouse.widget().canonical_name(),
        "aimer.widget:aimer_input::MouseRegion"
    );
    assert_eq!(mouse.children().minimum(), 1);
    assert_eq!(mouse.children().maximum(), 1);
    assert_eq!(mouse.callbacks().len(), 2);
}

#[test]
fn derived_input_widgets_lower_bounded_nodes_with_children_and_properties() {
    let mut gesture_context = context();
    let gesture_root = GestureDetector::new()
        .on_tap(|| {})
        .child(PortableLeaf)
        .to_portable_node(&mut gesture_context, source(1))
        .unwrap();
    let gesture_document = gesture_context.finish_document(gesture_root).unwrap();
    let gesture_bytes = gesture_document.encode().unwrap();
    let gesture_view =
        WidgetDocumentView::decode(&gesture_bytes, gesture_document.model_limits()).unwrap();
    let gesture_node = gesture_view.node(gesture_root.index()).unwrap();
    assert_eq!(gesture_node.children().collect::<Vec<_>>(), vec![0]);
    assert_eq!(gesture_node.callbacks().count(), 5);

    let mut field_context = context();
    let field_root = TextField::new()
        .auto_focus(true)
        .enable(false)
        .read_only(true)
        .to_portable_node(&mut field_context, source(2))
        .unwrap();
    let field_document = field_context.finish_document(field_root).unwrap();
    let field_bytes = field_document.encode().unwrap();
    let field_view =
        WidgetDocumentView::decode(&field_bytes, field_document.model_limits()).unwrap();
    let field_node = field_view.node(field_root.index()).unwrap();
    assert!(field_node.properties().count() >= 3);

    let mut area_context = context();
    let area_root = TextArea::new()
        .expand(true)
        .to_portable_node(&mut area_context, source(3))
        .unwrap();
    let area_document = area_context.finish_document(area_root).unwrap();
    let area_bytes = area_document.encode().unwrap();
    let area_view = WidgetDocumentView::decode(&area_bytes, area_document.model_limits()).unwrap();
    let area_node = area_view.node(area_root.index()).unwrap();
    assert_eq!(area_node.properties().count(), 2);

    let mut mouse_context = context();
    let mouse_root = MouseRegion::new()
        .on_hover_enter(|| {})
        .on_hover_exit(|| {})
        .child(PortableLeaf)
        .to_portable_node(&mut mouse_context, source(4))
        .unwrap();
    let mouse_document = mouse_context.finish_document(mouse_root).unwrap();
    let mouse_bytes = mouse_document.encode().unwrap();
    let mouse_view =
        WidgetDocumentView::decode(&mouse_bytes, mouse_document.model_limits()).unwrap();
    let mouse_node = mouse_view.node(mouse_root.index()).unwrap();
    assert_eq!(mouse_node.children().collect::<Vec<_>>(), vec![0]);
    assert_eq!(mouse_node.callbacks().count(), 2);
}

#[test]
fn text_input_contract_round_trips_bounded_configuration_without_native_handles() {
    let field_schema = <TextField as PortableWidgetSchema>::SCHEMA;
    let field_properties = field_schema
        .properties()
        .iter()
        .map(|property| property.canonical_name())
        .collect::<Vec<_>>();
    assert!(
        field_properties
            .iter()
            .any(|name| { name.ends_with("TextField:max_length_wire") })
    );

    let area_schema = <TextArea as PortableWidgetSchema>::SCHEMA;
    let area_properties = area_schema
        .properties()
        .iter()
        .map(|property| property.canonical_name())
        .collect::<Vec<_>>();
    assert!(
        area_properties
            .iter()
            .any(|name| name.ends_with("TextArea:min_lines_wire"))
    );
    assert!(
        area_properties
            .iter()
            .any(|name| name.ends_with("TextArea:max_lines_wire"))
    );

    let source = source(5);
    let mut field_context = context();
    let field_root = TextField::new()
        .controller(aimer_input::TextEditingController::with_text("native-only"))
        .max_length(Some(42))
        .decoration(BoxDecoration::new().background_color(Color::Rgba(1, 2, 3, 4)))
        .padding(LayoutSpacing::all(Spacing::Px(7)))
        .to_portable_node(&mut field_context, source)
        .unwrap();
    let field_document = field_context.finish_document(field_root).unwrap();
    let field_image = field_document.encode().unwrap();
    let field_view =
        WidgetDocumentView::decode(&field_image, field_document.model_limits()).unwrap();
    let field_node = field_view.node(field_root.index()).unwrap();
    assert_eq!(field_node.children().count(), 0);
    assert!(
        field_node
            .properties()
            .any(|property| { property.property_id() == field_schema.properties()[0].id() })
    );
    <TextField as PortableNativeWidget>::materialize_widget(&field_view, field_node, Vec::new())
        .expect("TextField's checked native materializer should rebuild fresh native state");
    assert!(
        (0..field_view.string_count())
            .all(|index| { field_view.string(index) != Some("native-only") })
    );

    let mut area_context = context();
    let area_root = TextArea::new()
        .min_lines(5)
        .max_lines(Some(9))
        .expand(true)
        .to_portable_node(&mut area_context, source.child(1))
        .unwrap();
    let area_document = area_context.finish_document(area_root).unwrap();
    let area_image = area_document.encode().unwrap();
    let area_view = WidgetDocumentView::decode(&area_image, area_document.model_limits()).unwrap();
    let area_node = area_view.node(area_root.index()).unwrap();
    assert_eq!(area_node.children().count(), 0);
    <TextArea as PortableNativeWidget>::materialize_widget(&area_view, area_node, Vec::new())
        .expect("TextArea's checked native materializer should rebuild fresh native state");
}
