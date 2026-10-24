//! Clicking a `Focusable` region gives its `FocusNode` the keyboard, including
//! when the region sits below a forwarding element such as `AnimatedTheme`.

use super::*;

use aimer_widget::Focusable;
use winit::event::ElementState;

fn click(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
    }
}

fn focus_at_scale(scale: f64) {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let node = FocusNode::new();
    let root = Column::new().children([
        Focusable::new()
            .node(node.clone())
            .box_child(SizedBox::new().width(100.0).height(40.0)),
        SizedBox::new().width(100.0).height(40.0).boxed(),
    ]);
    let mut app = AimerApp::start_headless_with(
        root,
        HeadlessOptions {
            size: PhysicalSize::new((300.0 * scale) as u32, (400.0 * scale) as u32),
            scale_factor: scale,
        },
    );
    app.pump_frames(3);
    assert!(!node.has_focus());

    click(&mut app, 20.0 * scale, 20.0 * scale);
    app.pump_frames(3);
    assert!(
        node.has_focus(),
        "a click inside the region must focus it at scale {}",
        scale
    );
}

#[test]
fn clicking_a_focusable_region_focuses_its_node() {
    focus_at_scale(1.0);
}

#[test]
fn clicking_a_focusable_region_focuses_its_node_on_a_retina_display() {
    focus_at_scale(2.0);
}

#[test]
fn clicking_a_focusable_region_under_an_animated_theme_focuses_its_node() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let node = FocusNode::new();
    let root = aimer_style::AnimatedTheme::new()
        .data(aimer_style::ThemeData::default())
        .child(
            Column::new().children([
                Focusable::new()
                    .node(node.clone())
                    .box_child(SizedBox::new().width(100.0).height(40.0)),
                SizedBox::new().width(100.0).height(40.0).boxed(),
            ]),
        );
    let mut app = AimerApp::start_headless_with(
        root,
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    click(&mut app, 20.0, 20.0);
    app.pump_frames(3);
    assert!(
        node.has_focus(),
        "a click under an AnimatedTheme must focus the region"
    );
}
