use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_color::prelude::Color;
use aimer_container::{Container, SizedBox};
use aimer_cupid::draw_cmd_v2::DrawCommand;
use aimer_cupid::material::{MaterialKind, MaterialRequest};
use aimer_decoration::{BoxDecoration, Decoration, DecorationBox, LightEdge, MultiDecorationBox};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, PortableWidget, Rebuildable,
    VisitorElement, Widget,
};

fn context(inner: &InnerCanvas, scale: f32) -> BuildContext<'_> {
    let mut ctx = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: 200.0,
            height: 160.0,
        },
        scale,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), scale as f64),
        tokio::runtime::Handle::current(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 200.0,
        max_height: 160.0,
    };
    ctx
}

fn colors(element: &dyn Element, ctx: &BuildContext) -> Vec<[f32; 4]> {
    aimer_widget::testing::record_retained_paint(element, ctx)
        .iter()
        .flat_map(|paint| paint.commands.iter())
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => MaterialRequest::decode(data)
                .ok()
                .filter(|request| request.kind == MaterialKind::LightEdge)
                .map(|request| request.border_color),
            _ => None,
        })
        .collect()
}

fn content() -> impl Widget {
    SizedBox::new().width(40).height(20)
}

#[tokio::test]
async fn light_edge_auto_color_uses_contrast_in_typed_runtime_and_mixed_stacks() {
    let inner = InnerCanvas::new();
    for scale in [1.0, 2.0] {
        let ctx = context(&inner, scale);
        for (background, expected) in [
            (Color::WHITE, [0.0, 0.0, 0.0, 1.0]),
            (Color::BLACK, [1.0; 4]),
            (Color::Rgb(24, 42, 76), [1.0; 4]),
            (Color::GREEN, [0.0, 0.0, 0.0, 1.0]),
            (Color::BLUE, [1.0; 4]),
            (Color::Rgb(117, 117, 117), [1.0; 4]),
            (Color::Rgb(118, 118, 118), [0.0, 0.0, 0.0, 1.0]),
        ] {
            let layers: Vec<Box<dyn Decoration>> = vec![
                Box::new(BoxDecoration::new().background_color(background)),
                Box::new(LightEdge::new()),
            ];
            let elements = [
                MultiDecorationBox::new()
                    .decoration(BoxDecoration::new().background_color(background))
                    .decoration(LightEdge::new())
                    .child(content())
                    .to_element(&ctx),
                MultiDecorationBox::new()
                    .decorations(layers)
                    .child(content())
                    .to_element(&ctx),
                MultiDecorationBox::new()
                    .decorations([BoxDecoration::new().background_color(background)])
                    .decoration(LightEdge::new())
                    .child(content())
                    .to_element(&ctx),
            ];
            for element in elements {
                assert_eq!(colors(element.as_ref(), &ctx), vec![expected]);
            }
        }
    }
}

#[tokio::test]
async fn light_edge_auto_color_inherits_containers_and_decoration_boxes() {
    let inner = InnerCanvas::new();
    for scale in [1.0, 2.0] {
        let ctx = context(&inner, scale);
        let elements = [
            Container::new()
                .color(Color::WHITE)
                .decoration(LightEdge::new())
                .child(content())
                .to_element(&ctx),
            Container::new()
                .color(Color::WHITE)
                .child(
                    Container::new().child(
                        DecorationBox::new()
                            .decoration(LightEdge::new())
                            .child(content()),
                    ),
                )
                .to_element(&ctx),
            DecorationBox::new()
                .decoration(BoxDecoration::new().background_color(Color::WHITE))
                .child(
                    MultiDecorationBox::new()
                        .decoration(LightEdge::new())
                        .child(content()),
                )
                .to_element(&ctx),
        ];
        for element in elements {
            assert_eq!(colors(element.as_ref(), &ctx), vec![[0.0, 0.0, 0.0, 1.0]]);
        }
        let dark_inner = Container::new()
            .color(Color::WHITE)
            .child(
                Container::new().color(Color::BLACK).child(
                    DecorationBox::new()
                        .decoration(LightEdge::new())
                        .child(content()),
                ),
            )
            .to_element(&ctx);
        assert_eq!(colors(dark_inner.as_ref(), &ctx), vec![[1.0; 4]]);
    }
}

#[tokio::test]
async fn light_edge_auto_color_composites_transparency_and_respects_layer_order() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    for (overlay, expected) in [
        (Color::Transparent, [0.0, 0.0, 0.0, 1.0]),
        (Color::Rgba(0, 0, 0, 64), [0.0, 0.0, 0.0, 1.0]),
        (Color::Rgba(0, 0, 0, 128), [0.0, 0.0, 0.0, 1.0]),
        (Color::Rgba(0, 0, 0, 192), [1.0; 4]),
    ] {
        let element = MultiDecorationBox::new()
            .decoration(BoxDecoration::new().background_color(Color::WHITE))
            .decoration(BoxDecoration::new().background_color(overlay))
            .decoration(LightEdge::new())
            .child(content())
            .to_element(&ctx);
        assert_eq!(colors(element.as_ref(), &ctx), vec![expected]);
        let layers: Vec<Box<dyn Decoration>> = vec![
            Box::new(BoxDecoration::new().background_color(Color::WHITE)),
            Box::new(BoxDecoration::new().background_color(overlay)),
            Box::new(LightEdge::new()),
        ];
        let runtime = MultiDecorationBox::new()
            .decorations(layers)
            .child(content())
            .to_element(&ctx);
        assert_eq!(colors(runtime.as_ref(), &ctx), vec![expected]);
    }
    // A later background cannot affect an earlier highlight.
    let reversed = MultiDecorationBox::new()
        .decoration(LightEdge::new())
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .child(content())
        .to_element(&ctx);
    assert_eq!(colors(reversed.as_ref(), &ctx), vec![[1.0; 4]]);
    let transparent = MultiDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::Transparent))
        .decoration(LightEdge::new())
        .child(content())
        .to_element(&ctx);
    assert_eq!(colors(transparent.as_ref(), &ctx), vec![[1.0; 4]]);
    let explicit = MultiDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .decoration(LightEdge::new().color(Color::Rgba(255, 0, 0, 128)))
        .child(content())
        .to_element(&ctx);
    assert_eq!(
        colors(explicit.as_ref(), &ctx),
        vec![[1.0, 0.0, 0.0, 128.0 / 255.0]]
    );
    let automatic = MultiDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .decoration(LightEdge::new().color(Color::RED).color(None))
        .child(content())
        .to_element(&ctx);
    assert_eq!(colors(automatic.as_ref(), &ctx), vec![[0.0, 0.0, 0.0, 1.0]]);
}

#[tokio::test]
async fn light_edge_auto_color_repaints_retained_children_when_surface_color_changes() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    let background = Rc::new(BoxDecoration::new().background_color(Color::BLACK));
    let element = DecorationBox::new()
        .decoration(background.clone())
        .child(
            DecorationBox::new()
                .decoration(LightEdge::new())
                .child(content()),
        )
        .to_element(&ctx);
    assert_eq!(colors(element.as_ref(), &ctx), vec![[1.0; 4]]);
    background.update_color(Color::WHITE);
    element.visit_children(&mut |child| {
        let child_ctx = element.retained_v2_child_context(&ctx, child).unwrap();
        assert!(
            child.local_v2_paint_needs_recording(&child_ctx),
            "ancestor color must invalidate the rim"
        );
    });
    assert_eq!(colors(element.as_ref(), &ctx), vec![[0.0, 0.0, 0.0, 1.0]]);
    element.visit_children(&mut |child| {
        let child_ctx = element.retained_v2_child_context(&ctx, child).unwrap();
        assert!(!child.local_v2_paint_needs_recording(&child_ctx));
    });
}

#[tokio::test]
async fn light_edge_auto_color_keeps_sibling_surfaces_independent() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    let element = aimer_flex::Row::new()
        .children([
            Container::new()
                .width(40)
                .height(20)
                .color(Color::WHITE)
                .child(
                    DecorationBox::new()
                        .decoration(LightEdge::new())
                        .child(content()),
                )
                .boxed(),
            Container::new()
                .width(40)
                .height(20)
                .color(Color::BLACK)
                .child(
                    DecorationBox::new()
                        .decoration(LightEdge::new())
                        .child(content()),
                )
                .boxed(),
            DecorationBox::new()
                .decoration(LightEdge::new())
                .box_child(content()),
        ])
        .to_element(&ctx);
    assert_eq!(
        colors(element.as_ref(), &ctx),
        vec![[0.0, 0.0, 0.0, 1.0], [1.0; 4], [1.0; 4]]
    );
}

struct BackgroundProbe(Rc<Cell<Color>>);
impl PortableWidget for BackgroundProbe {}
impl Widget for BackgroundProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }
}
impl VisitorElement for BackgroundProbe {
    fn debug_name(&self) -> &'static str {
        "BackgroundProbe"
    }
}
impl EventElement for BackgroundProbe {}
impl Rebuildable for BackgroundProbe {}
impl LayoutElement for BackgroundProbe {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: 40.0,
            height: 20.0,
        }
    }
}
impl Drawable for BackgroundProbe {
    fn update(&self, ctx: &BuildContext) {
        self.0.set(ctx.paint_background);
    }
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.0.set(ctx.paint_background);
    }
}

#[tokio::test]
async fn light_edge_background_reaches_live_child_traversal_and_geometry_sync() {
    let inner = InnerCanvas::new();
    let ctx = context(&inner, 1.0);
    let observed = Rc::new(Cell::new(Color::Transparent));
    let element = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::WHITE))
        .child(BackgroundProbe(observed.clone()))
        .to_element(&ctx);
    element.update(&ctx);
    assert_eq!(observed.get(), Color::WHITE);
    observed.set(Color::Transparent);
    element.sync_paint_geometry(&ctx);
    assert_eq!(observed.get(), Color::WHITE);
}
