//! A `TextField` inside a `Scrollable` is typed into once it has been pressed.
//!
//! A scrollable routes its content through a dispatcher of its own, and typed
//! text is delivered to whoever the *enclosing* dispatcher says owns focus. A
//! press that only the scrollable's dispatcher took left the field flagged as
//! focused, and so the soft keyboard up, with nobody to receive the text.

use super::*;

use winit::event::{ElementState, Ime, Touch};

fn press_at(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
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

fn tap_at(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        app.send_window_event(WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(x, y),
            force: None,
            id: 1,
        }));
        app.pump_frames(2);
    }
}

struct Form<W: Widget + 'static> {
    app: HeadlessAimerApp<W>,
    first: (TextEditingController, FocusNode),
    second: (TextEditingController, FocusNode),
}

/// Two fields one above the other, 40 px tall each, in a scroll.
fn form() -> Form<impl Widget + 'static> {
    let field = || {
        let controller = TextEditingController::new();
        let node = FocusNode::new();
        (controller, node)
    };
    let (first, second) = (field(), field());
    let build = |(controller, node): &(TextEditingController, FocusNode)| {
        SizedBox::new()
            .height(40.0)
            .box_child(
                TextField::new()
                    .controller(controller.clone())
                    .focus_node(node.clone()),
            )
    };
    let root = Scrollable::new().box_child(Column::new().children([build(&first).boxed(), build(&second).boxed()]));
    let mut app = AimerApp::start_headless_with(
        root,
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    Form { app, first, second }
}

#[test]
fn a_pressed_field_in_a_scrollable_receives_typed_text() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut form = form();

    press_at(&mut form.app, 20.0, 20.0);
    form.app.pump_frames(3);
    form.app.send_user_event(AimerNativePlatformEvent::InsertText("ab".into()));
    form.app.pump_frames(3);
    form.app.send_window_event(WindowEvent::Ime(Ime::Commit("c".into())));
    form.app.pump_frames(3);

    assert_eq!(form.first.0.value().text(), "abc");
    assert!(form.second.0.value().text().is_empty());
}

#[test]
fn a_tapped_field_in_a_scrollable_receives_typed_text() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut form = form();

    tap_at(&mut form.app, 20.0, 20.0);
    form.app.pump_frames(3);
    form.app.send_window_event(WindowEvent::Ime(Ime::Commit("x".into())));
    form.app.pump_frames(3);

    assert_eq!(form.first.0.value().text(), "x");
}

#[test]
fn pressing_another_field_in_a_scrollable_moves_the_text_to_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut form = form();

    press_at(&mut form.app, 20.0, 20.0);
    form.app.pump_frames(3);
    press_at(&mut form.app, 20.0, 60.0);
    form.app.pump_frames(3);
    assert!(!form.first.1.has_focus(), "the first field should have lost focus");
    assert!(form.second.1.has_focus());

    form.app.send_window_event(WindowEvent::Ime(Ime::Commit("y".into())));
    form.app.pump_frames(3);

    assert!(form.first.0.value().text().is_empty());
    assert_eq!(form.second.0.value().text(), "y");
}

#[test]
fn pressing_empty_space_in_a_scrollable_takes_focus_from_the_field() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut form = form();

    press_at(&mut form.app, 20.0, 20.0);
    form.app.pump_frames(3);
    assert!(form.first.1.has_focus());

    press_at(&mut form.app, 20.0, 300.0);
    form.app.pump_frames(3);
    assert!(!form.first.1.has_focus());

    form.app.send_window_event(WindowEvent::Ime(Ime::Commit("z".into())));
    form.app.pump_frames(3);
    assert!(form.first.0.value().text().is_empty());
}
