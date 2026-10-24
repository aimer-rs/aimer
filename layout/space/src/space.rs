//! Wait why i am naming this file space.rs
//!
//! because this is a family of containers that have used the z-axis to position
//! their children

pub mod align;
mod hug;
pub mod positioned;
pub mod stack;

#[cfg(all(test, feature = "portable-guest"))]
mod portable_layout_tests {
    use aimer_widget::base::BuildContext;
    use aimer_widget::portable::__anteros::{Version, WIDGET_SIZED_BOX, WidgetDocumentView};
    use aimer_widget::portable::{
        PortableBuildContext, PortableLimits, PortableNativeWidget, PortableWidgetLimits,
        PortableWidgetSchema, SourceFingerprint, StableId128,
    };
    use aimer_widget::{AnyElement, ErrorWidget, PortableWidget, Widget};

    use super::align::{Align, Alignment};
    use super::positioned::Positioned;
    use super::stack::Stack;

    struct Leaf;

    impl Widget for Leaf {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            ErrorWidget::new("portable leaf").to_element(ctx)
        }
    }

    impl PortableWidget for Leaf {
        fn to_portable_node(
            self,
            ctx: &mut PortableBuildContext,
            source: SourceFingerprint,
        ) -> Result<
            aimer_widget::portable::PortableNodeId,
            aimer_widget::portable::PortableBuildError,
        > {
            ctx.push_node(WIDGET_SIZED_BOX, Version::new(1, 0), None, source, &[], &[])
        }
    }

    fn context() -> PortableBuildContext {
        PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(32, 32, 32, 32, 1_024, 8_192),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap()
    }

    fn source() -> SourceFingerprint {
        SourceFingerprint::new(StableId128::from_bytes([0x24; 16]))
    }

    #[test]
    fn align_lowers_alignment_child_and_layer() {
        let mut ctx = context();
        let root = Align::new()
            .alignment(Alignment::MidCenter)
            .layer(2)
            .child(Leaf)
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <Align<Leaf> as PortableWidgetSchema>::SCHEMA.widget().id()
        );
        assert_eq!(node.properties().count(), 1);
        assert_eq!(node.children().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn positioned_lowers_optional_offsets_and_required_child() {
        let mut ctx = context();
        let root = Positioned::new()
            .left(12.0)
            .layer(3)
            .child(Leaf)
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <Positioned<Leaf> as PortableWidgetSchema>::SCHEMA
                .widget()
                .id()
        );
        assert_eq!(node.properties().count(), 2);
        assert_eq!(node.children().count(), 1);
    }

    #[test]
    fn positioned_exposes_a_native_materializer_for_hot_reload_hosts() {
        fn assert_native_materializer<T: PortableNativeWidget>() {}

        assert_native_materializer::<Positioned<aimer_container::ZeroSizedBox>>();
    }

    #[test]
    fn stack_lowers_erased_collection_children() {
        let mut ctx = context();
        let root = Stack::new()
            .add_child(Leaf)
            .to_portable_node(&mut ctx, source())
            .unwrap();
        let document = ctx.finish_document(root).unwrap();
        let bytes = document.encode().unwrap();
        let view = WidgetDocumentView::decode(&bytes, document.model_limits()).unwrap();
        let node = view.node(root.index()).unwrap();
        assert_eq!(
            node.widget_type(),
            <Stack as PortableWidgetSchema>::SCHEMA.widget().id()
        );
        assert_eq!(node.properties().count(), 0);
        assert_eq!(node.children().count(), 1);
    }
}
