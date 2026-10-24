//! A finger taps a button in a bar that floats over a scrollable page.
//!
//! The post screen of a forum client is a `Stack` of a bar pinned near the top,
//! on a layer above, and a long scrollable column. A phone has no cursor, so the
//! bar's buttons are only ever reached by a touch, and a tap there must press
//! them exactly as a mouse click does.

use super::*;

use aimer_container::Container;
use aimer_flex::Expanded;
use aimer_input::button::Button;
use aimer_style::LayoutSpacing;
use aimer_space::{Positioned, Stack};
use winit::event::Touch;

const BAR_TOP: f32 = 60.0;

/// The bar's first button covers x 12..112, y 72..108.
const BUTTON: (f64, f64) = (60.0, 90.0);

fn start() -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Cell<usize>>) {
    start_with(true)
}

fn start_with(scrollable: bool) -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Cell<usize>>) {
    start_order(scrollable, true)
}

fn start_order(scrollable: bool, bar_first: bool) -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Cell<usize>>) {
    let presses = Rc::new(Cell::new(0));
    let press = presses.clone();
    let bar = Container::new()
        .height(60)
        .padding(LayoutSpacing::all(12))
        .child(
            Row::new().children(vec![
                Button::new()
                    .on_press(move || press.set(press.get() + 1))
                    .child(SizedBox::new().width(100).height(36))
                    .boxed(),
                Expanded::new().box_child(SizedBox::new()),
            ]),
        );
    let rows = (0..40)
        .map(|_| SizedBox::new().height(60).color(Color::Rgb(40, 40, 60)).boxed())
        .collect::<Vec<_>>();
    let mut layers = vec![Positioned::new().layer(3).top(BAR_TOP).box_child(bar).boxed()];
    if scrollable {
        layers.push(
            Scrollable::new()
                .vertical_scroll_bar(None)
                .horizontal_scroll_bar(None)
                .box_child(Column::new().children(rows))
                .boxed(),
        );
    } else {
        layers.push(Column::new().children(rows).boxed());
    }
    if !bar_first {
        layers.reverse();
    }
    let page = Stack::new().children(layers);
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(400, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    (app, presses)
}

fn touch<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, phase: TouchPhase, x: f64, y: f64) {
    app.send_window_event(WindowEvent::Touch(Touch {
        device_id: DeviceId::dummy(),
        phase,
        location: PhysicalPosition::new(x, y),
        force: None,
        id: 1,
    }));
    app.pump_frames(2);
}

#[test]
fn a_mouse_click_presses_the_bars_button() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start();

    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(BUTTON.0, BUTTON.1),
    });
    app.pump_frames(2);
    for state in [winit::event::ElementState::Pressed, winit::event::ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
        app.pump_frames(2);
    }

    assert_eq!(presses.get(), 1);
}

#[test]
fn a_tap_presses_the_bars_button() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start();

    touch(&mut app, TouchPhase::Started, BUTTON.0, BUTTON.1);
    touch(&mut app, TouchPhase::Ended, BUTTON.0, BUTTON.1);

    assert_eq!(presses.get(), 1, "a tap on the bar's button did nothing");
}

#[test]
fn a_tap_still_presses_the_bars_button_after_the_page_was_scrolled() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start();

    // A flick on the page below the bar scrolls it.
    touch(&mut app, TouchPhase::Started, 200.0, 500.0);
    for step in 1..=8 {
        touch(&mut app, TouchPhase::Moved, 200.0, 500.0 - 40.0 * f64::from(step));
    }
    touch(&mut app, TouchPhase::Ended, 200.0, 180.0);
    app.pump_frames(30);

    touch(&mut app, TouchPhase::Started, BUTTON.0, BUTTON.1);
    touch(&mut app, TouchPhase::Ended, BUTTON.0, BUTTON.1);

    assert_eq!(presses.get(), 1, "a tap after a scroll did nothing");
}

#[test]
fn a_tap_presses_the_bars_button_over_a_plain_column() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start_with(false);

    touch(&mut app, TouchPhase::Started, BUTTON.0, BUTTON.1);
    touch(&mut app, TouchPhase::Ended, BUTTON.0, BUTTON.1);

    assert_eq!(presses.get(), 1, "a tap on the bar's button did nothing");
}

#[test]
fn a_tap_presses_the_bars_button_when_it_is_listed_after_the_scrollable() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start_order(true, false);
    touch(&mut app, TouchPhase::Started, BUTTON.0, BUTTON.1);
    touch(&mut app, TouchPhase::Ended, BUTTON.0, BUTTON.1);
    assert_eq!(presses.get(), 1, "a tap on the bar's button did nothing");
}

#[test]
fn a_mouse_press_before_any_hover_frame_presses_the_bars_button() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, presses) = start();
    // Position the cursor, but give the dispatcher no frames to see it.
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(BUTTON.0, BUTTON.1),
    });
    for state in [winit::event::ElementState::Pressed, winit::event::ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
    }
    app.pump_frames(2);
    assert_eq!(presses.get(), 1, "a press before the hover frame did nothing");
}
