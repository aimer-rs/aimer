//! A field that is told it gained focus can tell a user's touch from a frame
//! that settled focus on its own.
//!
//! The difference matters to the platform keyboard: one the user asked for by
//! tapping has to rise at once, while one raised because a page autofocused its
//! field arrives in the middle of whatever transition brought the page in.

use super::*;

use std::cell::RefCell;
use std::rc::Rc;
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

/// Starts an app whose only field records, on each focus gain, whether the gain
/// was settled between events.
fn recording(auto_focus: bool) -> (HeadlessAimerApp<impl Widget + 'static>, Rc<RefCell<Vec<bool>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let record = Rc::clone(&seen);
    let field = TextField::new()
        .auto_focus(auto_focus)
        .on_focus(move |_: String| {
            record
                .borrow_mut()
                .push(aimer_widget::focus_is_settling_between_events());
        });
    let app = AimerApp::start_headless_with(
        Column::new().children([field.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    (app, seen)
}

#[test]
fn an_autofocused_field_learns_focus_was_settled_between_events() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = recording(true);

    app.pump_frames(3);

    assert_eq!(*seen.borrow(), [true]);
}

#[test]
fn a_tapped_field_learns_focus_came_from_the_user() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = recording(false);
    app.pump_frames(3);
    assert!(seen.borrow().is_empty(), "nothing focuses the field yet");

    click(&mut app, 20.0, 15.0);
    app.pump_frames(3);

    assert_eq!(*seen.borrow(), [false]);
}

#[test]
fn the_answer_is_no_outside_of_a_settling_frame() {
    assert!(!aimer_widget::focus_is_settling_between_events());
}
