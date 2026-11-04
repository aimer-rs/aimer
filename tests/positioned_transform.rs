use aimer::canvas::{FrameCanvas, InnerCanvas};
use aimer_cupid::draw_cmd::{
    DrawCommand as V2DrawCommand, Rect, RenderOp, RenderPaintSource, RenderTree,
};
use aimer::widget::base::WindowHandle;
use aimer::{BuildContext, Positioned, ResolvedSize, SizedBox, Transform, Vec2d, Widget};

fn context<'a>(canvas: FrameCanvas<'a>, runtime: &tokio::runtime::Runtime) -> BuildContext<'a> {
    BuildContext::new(
        canvas,
        ResolvedSize {
            width: 100.0,
            height: 100.0,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(winit::dpi::PhysicalSize::new(100, 100), 1.0),
        runtime.handle().clone(),
    )
}

/// The display transform the render tree gives the positioned child after the
/// element synchronized its state, or `None` when it declined to paint.
fn child_transform(transform: Transform) -> Option<aimer_cupid::utilities::Mat3> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime");
    let inner = InnerCanvas::new();
    let context = context(FrameCanvas::new(&inner), &runtime);
    let element = Positioned::new()
        .left(10.0)
        .top(10.0)
        .transform(transform)
        .child(SizedBox::new().width(20.0).height(20.0))
        .to_element(&context);
    element.layout(&context);

    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 10.0, 20.0, 20.0))
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    tree.context(child)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![V2DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
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
    if !synced {
        return None;
    }
    tree.render_order(&[Rect::new(0.0, 0.0, 100.0, 100.0)])
        .into_iter()
        .find_map(|operation| match operation {
            RenderOp::Draw(item) if item.element == child => Some(item.transform),
            _ => None,
        })
}

#[test]
fn a_scale_transform_scales_the_positioned_child_node() {
    let transform = child_transform(Transform::Scale(2.0, 3.0))
        .expect("a scaled Positioned still paints through the render tree");

    // Scale about the child's own origin: the diagonal carries the factors.
    assert!((transform.cols[0][0] - 2.0).abs() < 1e-5, "{transform:?}");
    assert!((transform.cols[1][1] - 3.0).abs() < 1e-5, "{transform:?}");
}

#[test]
fn a_rotate_transform_rotates_the_positioned_child_node() {
    let transform = child_transform(Transform::Rotate(std::f32::consts::FRAC_PI_2))
        .expect("a rotated Positioned still paints through the render tree");

    assert!(transform.cols[0][0].abs() < 1e-5, "{transform:?}");
    assert!(
        (transform.cols[0][1].abs() - 1.0).abs() < 1e-5,
        "{transform:?}"
    );
}

#[test]
fn a_translate_only_transform_keeps_the_child_untransformed() {
    let transform = child_transform(Transform::None).expect("an untransformed Positioned paints");

    assert!((transform.cols[0][0] - 1.0).abs() < 1e-5, "{transform:?}");
    assert!((transform.cols[1][1] - 1.0).abs() < 1e-5, "{transform:?}");
}
