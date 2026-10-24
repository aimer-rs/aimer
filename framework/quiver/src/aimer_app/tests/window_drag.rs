//! A press in the titlebar strip drags the window only when no element claimed
//! it.
//!
//! With the content under the titlebar, AppKit moved the window for any press
//! in the strip before the application saw it, so a toggle or a field there
//! dragged the window as well as being pressed. The framework now owns the
//! drag: elements get the press first, and a press nobody consumed or captured
//! starts it. The framework moves the window itself while the button is held,
//! because AppKit decides before the press reaches the content and cannot be told
//! afterwards that an element took it.

use super::*;

use aimer_input::button::Button;
use aimer_native::macos_windowing::MacosWindowing;
use winit::event::{ElementState, MouseButton};

/// The strip, in logical points from the top of the window.
const BAND: f64 = 40.0;

fn headless_with_band(band: Option<f64>) -> HeadlessAimerApp<impl Widget + 'static> {
    let mut app = AimerApp::start_headless_with(
        Column::new().children([
            Row::new()
                .children(vec![
                    Button::new()
                        .on_press(|| {})
                        .child(
                            SizedBox::new()
                                .width(Dimension::Px(60.0))
                                .height(Dimension::Px(40.0)),
                        )
                        .boxed(),
                    SizedBox::new()
                        .width(Dimension::Px(300.0))
                        .height(Dimension::Px(40.0))
                        .boxed(),
                ])
                .boxed(),
            TextField::new().boxed(),
            SizedBox::new()
                .width(Dimension::Px(400.0))
                .height(Dimension::Px(120.0))
                .boxed(),
        ]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 300),
            scale_factor: 1.0,
        },
    );
    if let Some(band) = band {
        app.app.macos_windowing = MacosWindowing::new()
            .fullsize_content_view(true)
            .titlebar_drag_height(band);
    }
    app.pump_frames(3);
    app
}

fn press_at(
    app: &mut HeadlessAimerApp<impl Widget + 'static>,
    x: f64,
    y: f64,
    button: MouseButton,
) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Pressed,
        button,
    });
}

fn dragging(app: &HeadlessAimerApp<impl Widget + 'static>) -> bool {
    app.app.macos_windowing.window_drag_active()
}

fn release(app: &mut HeadlessAimerApp<impl Widget + 'static>, button: MouseButton) {
    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state: ElementState::Released,
        button,
    });
}

fn move_to(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
}

#[test]
fn a_press_nobody_claimed_drags_the_window() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 20.0, MouseButton::Left);

    assert!(dragging(&app));
    assert_eq!(app.app.macos_windowing.window_drag_grab(), Some((250.0, 20.0)));
}

#[test]
fn a_press_on_a_button_in_the_strip_is_the_buttons() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 30.0, 20.0, MouseButton::Left);

    assert!(!dragging(&app), "the button claimed the press");
}

#[test]
fn a_press_below_the_strip_never_drags() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 250.0, MouseButton::Left);

    assert!(!dragging(&app));
}

#[test]
fn only_the_primary_button_drags() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 20.0, MouseButton::Right);

    assert!(!dragging(&app));
}

#[test]
fn a_window_the_framework_does_not_drag_is_left_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(None);

    press_at(&mut app, 250.0, 20.0, MouseButton::Left);

    assert!(!dragging(&app));
}

#[test]
fn the_edge_of_the_strip_is_outside_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, BAND, MouseButton::Left);

    assert!(!dragging(&app));
}

#[test]
fn releasing_the_button_ends_the_drag() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 20.0, MouseButton::Left);
    assert!(dragging(&app));
    release(&mut app, MouseButton::Left);

    assert!(!dragging(&app));
}

#[test]
fn a_drag_whose_release_was_never_seen_does_not_outlive_the_button() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 20.0, MouseButton::Left);
    // The button is up as far as the application knows, but the release itself
    // never arrived: the window must not follow an idle pointer.
    app.app.pressed_button = None;
    move_to(&mut app, 260.0, 24.0);

    assert!(!dragging(&app));
}

#[test]
fn a_drag_carries_on_while_the_pointer_passes_over_an_element() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = headless_with_band(Some(BAND));

    press_at(&mut app, 250.0, 20.0, MouseButton::Left);
    move_to(&mut app, 30.0, 20.0);

    // The pointer is over the button now, but the window is what is moving.
    assert!(dragging(&app));
    release(&mut app, MouseButton::Left);
    assert!(!dragging(&app));
}

#[test]
fn a_press_on_a_text_field_in_the_strip_is_the_fields() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // A field takes focus on a press and consumes nothing, so the window must
    // not treat the press as left over.
    let mut app = AimerApp::start_headless_with(
        Column::new().children([
            TextField::new().boxed(),
            SizedBox::new()
                .width(Dimension::Px(400.0))
                .height(Dimension::Px(200.0))
                .boxed(),
        ]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 300),
            scale_factor: 1.0,
        },
    );
    app.app.macos_windowing = MacosWindowing::new()
        .fullsize_content_view(true)
        .titlebar_drag_height(BAND);
    app.pump_frames(3);

    press_at(&mut app, 100.0, 10.0, MouseButton::Left);

    assert!(!dragging(&app), "the field took the press");
}

#[test]
fn a_press_on_a_focusable_region_in_the_strip_is_the_regions() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // A focusable region takes a press for focus and consumes nothing: the
    // press is still the region's, not a press on the window behind it.
    let mut app = AimerApp::start_headless_with(
        Column::new().children([
            aimer_widget::Focusable::new()
                .child(
                    SizedBox::new()
                        .width(Dimension::Px(400.0))
                        .height(Dimension::Px(40.0)),
                )
                .boxed(),
            SizedBox::new()
                .width(Dimension::Px(400.0))
                .height(Dimension::Px(200.0))
                .boxed(),
        ]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 300),
            scale_factor: 1.0,
        },
    );
    app.app.macos_windowing = MacosWindowing::new()
        .fullsize_content_view(true)
        .titlebar_drag_height(BAND);
    app.pump_frames(3);

    press_at(&mut app, 100.0, 20.0, MouseButton::Left);

    assert!(!dragging(&app), "the region took the press for focus");
}
