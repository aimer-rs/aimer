use aimer::canvas::{FrameCanvas, InnerCanvas};
use aimer_cupid::draw_cmd::{
    DrawCommand as V2DrawCommand, Rect, RenderOp, RenderPaintSource, RenderTree,
};
use aimer::widget::base::WindowHandle;
use aimer::{BuildContext, Color, Opacity, ResolvedSize, SizedBox, Vec2d, Widget};

fn context<'a>(canvas: FrameCanvas<'a>, runtime: &tokio::runtime::Runtime) -> BuildContext<'a> {
    BuildContext::new(
        canvas,
        ResolvedSize {
            width: 64.0,
            height: 64.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(winit::dpi::PhysicalSize::new(64, 64), 1.0),
        runtime.handle().clone(),
    )
}

/// The opacities the render tree would composite the child with, after the
/// element synchronized its state.
fn child_opacities(opacity: f32) -> (bool, Vec<f32>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime");
    let inner = InnerCanvas::new();
    let context = context(FrameCanvas::new(&inner), &runtime);
    let element = Opacity::new()
        .opacity(opacity)
        .child(
            SizedBox::new()
                .width(32.0)
                .height(32.0)
                .color(Color::Rgba(0, 0, 0, 255)),
        )
        .to_element(&context);
    element.layout(&context);

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(0.0, 0.0, 32.0, 32.0))
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    tree.context(child)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 32.0, 32.0),
            color: aimer_cupid::utilities::Color::white(),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: aimer_cupid::utilities::Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: aimer_cupid::utilities::Color::transparent(),
        }])
        .unwrap();

    let retained = tree.context(root).unwrap();
    let synced = context.with_local_v2_paint_context(retained, |context| {
        element.can_paint_local_v2(context) && element.sync_local_v2_state(context)
    });
    let opacities = tree
        .render_order(&[Rect::new(0.0, 0.0, 64.0, 64.0)])
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) if item.element == child => Some(item.opacity),
            RenderOp::BeginOpacityGroup {
                element, opacity, ..
            } if element == child => Some(opacity),
            _ => None,
        })
        .collect();
    (synced, opacities)
}

#[test]
fn a_translucent_opacity_dims_its_retained_child() {
    let (synced, opacities) = child_opacities(0.4);

    assert!(
        synced,
        "a translucent Opacity must still paint through the render tree"
    );
    assert!(
        opacities.iter().any(|value| (value - 0.4).abs() < 1e-6),
        "the child node should be presented at 0.4, saw {opacities:?}"
    );
}

#[test]
fn a_fully_opaque_opacity_leaves_its_child_untouched() {
    let (synced, opacities) = child_opacities(1.0);

    assert!(synced);
    assert!(
        opacities.iter().all(|value| (value - 1.0).abs() < 1e-6),
        "{opacities:?}"
    );
}
