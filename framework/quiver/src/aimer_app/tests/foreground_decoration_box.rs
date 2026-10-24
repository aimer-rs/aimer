use super::*;

use aimer_cupid::draw_cmd_v2::{DrawCommand as LocalCommand, RenderOp};
use aimer_decoration::ForegroundDecorationBox;
use aimer_space::{Positioned, Stack};
use aimer_style::BoxDecoration;

fn is_color_fill(command: &LocalCommand, rgb: (u8, u8, u8)) -> bool {
    matches!(command, LocalCommand::FillRect { color, .. }
        if (color.r, color.g, color.b, color.a) == (rgb.0, rgb.1, rgb.2, 255))
}

#[test]
fn foreground_decoration_headless_paints_after_its_child_without_moving_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let source = Rc::new(BoxDecoration::new().background_color(Color::RED));
    let widget = Stack::new().add_child(
        Positioned::new().left(20).top(30).child(
            ForegroundDecorationBox::new()
                .decoration(source.clone())
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
    let draws: Vec<_> = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item),
            _ => None,
        })
        .collect();
    let position_of = |rgb| {
        draws
            .iter()
            .position(|item| item.snapshot().commands.iter().any(|c| is_color_fill(c, rgb)))
    };
    let blue = position_of((0, 0, 255)).expect("the child paints");
    let red = position_of((255, 0, 0)).expect("the foreground paints");
    assert!(blue < red, "foreground paints after its child");
    assert_eq!(draws[blue].bounds, Rect::new(20.0, 30.0, 40.0, 30.0));
    assert_eq!(
        draws[red].bounds,
        Rect::new(20.0, 30.0, 40.0, 30.0),
        "the foreground covers exactly the child's box"
    );

    let layer = draws[red].element;
    let layer_revision = tree.draw_list_revision(layer).unwrap();
    let child_revision = tree.draw_list_revision(draws[blue].element).unwrap();
    source.update_color(Color::BLACK);
    app.window.request_redraw();
    app.render_frame();
    assert_eq!(tree.draw_list_revision(layer).unwrap(), layer_revision + 1);
    assert!(
        tree.draw_list_snapshot(layer)
            .unwrap()
            .commands
            .iter()
            .any(|c| is_color_fill(c, (0, 0, 0)))
    );
    assert_eq!(
        tree.draw_list_revision(draws[blue].element).unwrap(),
        child_revision,
        "the child is not re-recorded when only the foreground changes"
    );
}

/// The widest node the retained plan draws, in logical pixels.
fn widest(app: &HeadlessAimerApp<impl Widget + 'static>) -> f32 {
    app.render_ops()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.bounds.width),
            _ => None,
        })
        .fold(0.0, f32::max)
}

#[test]
fn a_foreground_decoration_follows_the_window_through_resizes() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let widget = Stack::new().add_child(
        Positioned::new().left(20).top(30).child(
            ForegroundDecorationBox::new()
                .decoration(BoxDecoration::new().background_color(Color::RED))
                .child(SizedBox::new().width(40).height(30).color(Color::BLUE)),
        ),
    );
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(400, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    for width in [600, 250, 520] {
        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, 300)));
        app.pump_frames(3);
        assert_eq!(widest(&app), width as f32, "the tree follows a {width}px window");
    }
}
