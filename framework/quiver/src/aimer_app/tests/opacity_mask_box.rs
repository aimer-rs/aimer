use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand as LocalCommand, RenderOp};
use aimer_decoration::OpacityGradient;
use aimer_space::{Positioned, Stack};

#[test]
fn an_opacity_mask_box_is_planned_as_a_group_of_its_alpha_then_its_content() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let widget = Stack::new().add_child(
        Positioned::new().left(20).top(30).child(
            OpacityGradient::linear()
                .opacities([1.0, 0.0])
                .child(SizedBox::new().width(40).height(30).color(Color::BLUE)),
        ),
    );
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(400, 320),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(3);
    let tree = app.app.render_tree().clone();
    let operations = tree.render_all();

    let begin = operations
        .iter()
        .position(|operation| {
            matches!(
                operation,
                RenderOp::BeginOpacityGroup {
                    fade: true,
                    masked: false,
                    ..
                }
            )
        })
        .expect("a fade group is planned");
    let RenderOp::BeginOpacityGroup {
        element: group,
        opacity,
        bounds,
        ..
    } = &operations[begin]
    else {
        unreachable!()
    };
    assert_eq!(*opacity, 1.0);
    assert_eq!(*bounds, Rect::new(20.0, 30.0, 40.0, 30.0));

    // Inside the group: the decoration's own item first, then the child's.
    let inside: Vec<_> = operations[begin + 1..]
        .iter()
        .take_while(|operation| !matches!(operation, RenderOp::EndOpacityGroup { .. }))
        .map(|operation| match operation {
            RenderOp::Draw(item) => item,
            _ => panic!("only draws sit inside the group"),
        })
        .collect();
    assert_eq!(inside.first().map(|item| item.element), Some(*group));
    assert!(
        matches!(inside[0].snapshot().commands.first(), Some(LocalCommand::DrawCustom { .. })),
        "the marker heads the decoration's list"
    );
    assert!(
        inside[1..].iter().any(|item| item.snapshot().commands.iter().any(|command| matches!(
            command,
            LocalCommand::FillRect { color, .. } if (color.r, color.g, color.b) == (0, 0, 255)
        ))),
        "the blue child is among the content"
    );
    assert!(
        matches!(
            operations.iter().rev().find(|operation| matches!(operation, RenderOp::EndOpacityGroup { .. })),
            Some(RenderOp::EndOpacityGroup { element }) if element == group
        ),
        "the group closes after its descendants"
    );
}
