mod anchor;
mod animated_content;
mod animation;
mod floating;
pub(crate) mod host;
mod modal;
mod paint;
mod placement;

pub use anchor::{Anchor, AnchorHandle};
pub use animation::ModalAnimation;
pub use floating::Floating;
pub use host::{
    ModalController, ModalHandle, ModalHost, ModalId, OverlayContentHandle, OverlayLayer,
    OverlayLayerHandle, OverlayPainter, WindowModalState, prepare_retained_render_tree,
};
pub use modal::Modal;
pub use placement::{
    FloatingAlign, FloatingPlacement, FloatingSide, OverflowPolicy, PlacementSpec,
    resolve_placement,
};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_animation::curves;
    use aimer_container::ZeroSizedBox;
    use aimer_space::Alignment;
    use aimer_widget::base::Color;

    use super::{Anchor, Floating, Modal, ModalAnimation, ModalHost};

    #[test]
    fn modal_configuration_survives_child_attachment() {
        let animation = ModalAnimation::new()
            .enter_duration(Duration::from_millis(240))
            .exit_duration(Duration::from_millis(120))
            .enter_curve(curves::EASE_OUT)
            .exit_curve(curves::EASE_IN)
            .content_scale_from(0.9);
        let modal = Modal::new()
            .barrier_color(Color::BLUE.with_opacity(160))
            .alignment(Alignment::BotRight)
            .animation(animation.clone())
            .child(ZeroSizedBox);

        assert_eq!(modal.animation_config(), Some(animation));
        assert_eq!(
            std::mem::discriminant(&modal.alignment_value()),
            std::mem::discriminant(&Alignment::BotRight)
        );
        assert_eq!(modal.barrier_color_value(), Color::BLUE.with_opacity(160));
    }

    #[test]
    fn animation_normalizes_invalid_content_scale() {
        assert_eq!(
            ModalAnimation::new()
                .content_scale_from(f32::NAN)
                .content_scale(),
            1.0
        );
        assert_eq!(
            ModalAnimation::new()
                .content_scale_from(-1.0)
                .content_scale(),
            0.0
        );
        assert_eq!(
            ModalAnimation::new()
                .content_scale_from(2.0)
                .content_scale(),
            1.0
        );
    }

    #[test]
    fn show_and_dismiss_enqueue_framework_commands_immediately() {
        super::host::reset_registry_for_test();

        let handle = Modal::new().child(ZeroSizedBox).show();
        assert_eq!(super::host::pending_command_count_for_test(), 1);

        assert!(handle.dismiss());
        assert!(!handle.dismiss());
        assert_eq!(super::host::pending_command_count_for_test(), 2);
    }

    #[test]
    fn overlay_widgets_publish_derived_required_child_schemas() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <Anchor<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::exactly(1)
        );
        assert_eq!(
            <Floating<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::exactly(1)
        );
        assert_eq!(
            <Modal<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::exactly(1)
        );
        assert_eq!(
            <ModalHost<ZeroSizedBox> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::exactly(1)
        );
    }
}
