use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{Container, SizedBox};
use aimer_decoration::{Decoration, DecorationBox, MultiDecorationBox};
use aimer_style::{BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxOutline, BoxShadow};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{Drawable, Widget};

#[tokio::test]
async fn style_components_work_directly_in_containers_and_decoration_wrappers() {
    let inner = InnerCanvas::new();
    for scale in [1.0, 2.0] {
        let mut ctx = BuildContext::new(
            FrameCanvas::new(&inner), ResolvedSize { width: 200.0, height: 160.0 }, scale,
            Vec2d::default(), Vec2d::default(),
            WindowHandle::headless(Default::default(), 1.0), tokio::runtime::Handle::current(),
        );
        ctx.box_constraint = BoxConstraint {
            min_width: 0.0, min_height: 0.0, max_width: 200.0, max_height: 160.0,
        };
        let stroke = BorderSlice::new().style(BorderStyle::Solid).stroke(2).color(Color::BLUE);
        let border = Container::new().width(80).height(40).decoration(BoxBorder::all(stroke))
            .child(SizedBox::new()).to_element(&ctx);
        assert_eq!(border.content_size(&ctx), ResolvedSize { width: 76.0 * scale, height: 36.0 * scale });
        let radius = Container::new().width(80).height(40).color(Color::RED)
            .decoration(BorderRadius::from(8)).child(SizedBox::new()).to_element(&ctx);
        assert_eq!(radius.content_size(&ctx), ResolvedSize { width: 80.0 * scale, height: 40.0 * scale });
        radius.visit_children(&mut |child| {
            assert_eq!(radius.retained_v2_child_clip_radius(&ctx, child), [8.0; 4]);
        });
        let decorations: [Box<dyn Decoration>; 4] = [
            Box::new(BoxBorder::all(stroke)), Box::new(BoxOutline::all(stroke)),
            Box::new(BoxShadow::new().blur(3.0)), Box::new(BorderRadius::from(8)),
        ];
        for decoration in decorations {
            let element = DecorationBox::new().decoration(decoration)
                .child(SizedBox::new().width(80).height(40)).to_element(&ctx);
            assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 80.0 * scale, height: 40.0 * scale });
        }
        let stack = MultiDecorationBox::new().decoration(BoxShadow::new().blur(3.0))
            .decoration(BoxBorder::all(stroke)).decoration(BoxOutline::all(stroke))
            .decoration(BorderRadius::from(8))
            .child(SizedBox::new().width(80).height(40)).to_element(&ctx);
        assert_eq!(stack.computed_size(&ctx), ResolvedSize { width: 80.0 * scale, height: 40.0 * scale });
    }
}
