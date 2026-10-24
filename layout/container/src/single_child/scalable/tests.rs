use super::*;
use crate::SizedBox;
use aimer_attribute::{BoxConstraint, Dimension};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_widget::base::WindowHandle;

#[tokio::test]
async fn scaling_preserves_layout_size_and_parent_constraints() {
    let inner = InnerCanvas::new();
    let mut ctx = BuildContext::new(
        FrameCanvas::new(&inner), ResolvedSize { width: 100.0, height: 40.0 },
        2.0, Default::default(), Default::default(),
        WindowHandle::headless(Default::default(), 2.0),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0, min_height: 0.0, max_width: 100.0, max_height: 40.0,
    };
    for scale in [0.5, 1.0, 1.5, 2.0] {
        let element = Scalable::new().scale(scale)
            .child(SizedBox::new().width(30).height(12)).to_element(&ctx);
        assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 60.0, height: 24.0 });
        assert_eq!(element.content_size(&ctx), ResolvedSize { width: 60.0, height: 24.0 });
        element.visit_children(&mut |child| {
            let child_ctx = element.retained_v2_child_context_at(&ctx, child, 0).unwrap();
            assert_eq!(child_ctx.scale, ctx.scale);
            assert_eq!(child_ctx.box_constraint, ctx.box_constraint);
        });
    }
    let percent = Scalable::new().scale(2.0)
        .child(SizedBox::new().width(Dimension::Percent(50.0)).height(12))
        .to_element(&ctx);
    assert_eq!(percent.computed_size(&ctx), ResolvedSize { width: 50.0, height: 24.0 });

    ctx.visible_rect = Some((0.0, 0.0, 100.0, 40.0));
    let centered = Scalable::new().scale(2.0)
        .child(SizedBox::new().width(30).height(12)).to_element(&ctx);
    centered.visit_children(&mut |child| {
        let child_ctx = centered.retained_v2_child_context_at(&ctx, child, 0).unwrap();
        assert_eq!(child_ctx.visible_rect, None);
    });
}
