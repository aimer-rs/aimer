use std::cell::RefCell;
use std::rc::Rc;

use aimer_attribute::size::{ResolvedSize, Size};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, CompositorAnimationDecision, CompositorAnimationFrame, CompositorTransform,
    Drawable, Element, LayoutElement, VisitorElement,
};

use crate::ModalAnimation;
use crate::animation::visual_values;
use crate::host::ModalTimeline;

pub(crate) fn wrap_animated_content(
    child: AnyElement,
    animation: Option<ModalAnimation>,
    timeline: Rc<RefCell<ModalTimeline>>,
    scale_content: bool,
) -> AnyElement {
    let Some(animation) = animation else {
        return child;
    };
    AnimatedOverlayContent {
        child,
        animation,
        timeline,
        scale_content,
    }
    .boxed()
}

#[derive(aimer_macro::EventElement, aimer_macro::Rebuildable)]
struct AnimatedOverlayContent {
    child: AnyElement,
    animation: ModalAnimation,
    timeline: Rc<RefCell<ModalTimeline>>,
    scale_content: bool,
}

impl AnimatedOverlayContent {
    fn sample_frame(&self, ctx: &BuildContext) -> CompositorAnimationFrame {
        let (progress, active) = {
            let timeline = self.timeline.borrow();
            (timeline.progress(), timeline.is_active())
        };
        let size = self.child.computed_size(ctx);
        animation_frame(progress, active, &self.animation, self.scale_content, size)
    }
}

impl Drawable for AnimatedOverlayContent {
    fn update(&self, ctx: &BuildContext) {
        // The retained tree applies the sampled animation to this node.
        self.sample_frame(ctx);
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        #[cfg(all(not(target_arch = "wasm32"), not(feature = "portable-guest")))]
        {
            true
        }
        // These targets do not run the retained compositor animation hook, so
        // keep the subtree together for the live canvas transform fallback.
        #[cfg(any(target_arch = "wasm32", feature = "portable-guest"))]
        {
            false
        }
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = aimer_canvas::Canvas::of(ctx);
        canvas.finish();
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        // The retained tree applies the animation to this node. The
        // compatibility walk only updates descendant state and recordings.
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        self.child.prepare_layout(ctx)
    }

    fn compositor_animation(&self, ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = self.sample_frame(ctx);
        if frame.valid {
            CompositorAnimationDecision::Compositor(frame)
        } else {
            CompositorAnimationDecision::Live(frame)
        }
    }
}

impl LayoutElement for AnimatedOverlayContent {
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

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

impl VisitorElement for AnimatedOverlayContent {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn visit_retained_v2_children<'a>(&'a self, visitor: &mut dyn FnMut(usize, &'a dyn Element)) {
        visitor(0, self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedOverlayContent"
    }
}

fn animation_frame(
    progress: f32,
    active: bool,
    animation: &ModalAnimation,
    scale_content: bool,
    size: ResolvedSize,
) -> CompositorAnimationFrame {
    let (opacity, scale) = visual_values(progress, animation.content_scale_from);
    let transform = if scale_content {
        CompositorTransform::Scale {
            sx: scale,
            sy: scale,
            origin_x: size.width / 2.0,
            origin_y: size.height / 2.0,
        }
    } else {
        CompositorTransform::Identity
    };
    let valid = progress.is_finite()
        && opacity.is_finite()
        && (!scale_content
            || (size.width.is_finite()
                && size.height.is_finite()
                && size.width >= 0.0
                && size.height >= 0.0));
    CompositorAnimationFrame::new(progress, transform, Some(opacity), None, active, valid)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use aimer_attribute::size::ResolvedSize;
    use aimer_cupid::utilities::Mat3;
    use aimer_widget::{
        CompositorTransform, Drawable, Element, EventElement, LayoutElement, VisitorElement,
    };

    use super::animation_frame;
    use crate::ModalAnimation;

    #[test]
    fn content_frame_keeps_fade_and_center_scale_as_compositor_properties() {
        let frame = animation_frame(
            0.0,
            true,
            &ModalAnimation::new().content_scale_from(0.8),
            true,
            ResolvedSize {
                width: 200.0,
                height: 80.0,
            },
        );

        assert_eq!(frame.opacity, Some(0.0));
        assert!(frame.active);
        assert_eq!(
            frame.transform,
            CompositorTransform::Scale {
                sx: 0.8,
                sy: 0.8,
                origin_x: 100.0,
                origin_y: 40.0,
            }
        );
        let matrix = Mat3::translate(100.0, 40.0)
            .mul(&Mat3::scale(0.8, 0.8))
            .mul(&Mat3::translate(-100.0, -40.0));
        assert_eq!(matrix.transform_point(100.0, 40.0), (100.0, 40.0));
    }

    #[test]
    fn barrier_frame_fades_without_scaling() {
        let frame = animation_frame(
            0.5,
            true,
            &ModalAnimation::new(),
            false,
            ResolvedSize {
                width: 200.0,
                height: 80.0,
            },
        );

        assert_eq!(frame.opacity, Some(0.5));
        assert_eq!(frame.transform, CompositorTransform::Identity);
    }

    #[test]
    fn animated_wrapper_exposes_the_same_child_to_retained_paint_and_events() {
        let child = TestChild.boxed();
        let timeline = crate::host::ModalTimeline::new_static();
        let wrapper = super::wrap_animated_content(
            child,
            Some(ModalAnimation::new()),
            Rc::new(RefCell::new(timeline)),
            true,
        );

        let mut retained_children = Vec::new();
        wrapper
            .as_ref()
            .visit_retained_v2_children(&mut |_, child| retained_children.push(child.debug_name()));
        let mut event_children = Vec::new();
        wrapper
            .as_ref()
            .event_children(&mut |child| event_children.push(child.debug_name()));

        assert_eq!(retained_children, ["TestChild"]);
        assert_eq!(event_children, retained_children);
    }

    #[derive(aimer_macro::Rebuildable)]
    struct TestChild;

    impl Drawable for TestChild {
        fn update(&self, _ctx: &aimer_widget::base::BuildContext) {}
    }

    impl EventElement for TestChild {}

    impl LayoutElement for TestChild {}

    impl VisitorElement for TestChild {
        fn debug_name(&self) -> &'static str {
            "TestChild"
        }
    }
}
