//! A `Stack` layer that covers a point keeps the pointer from the layers below.
//!
//! A floating bar sits over a list of cards. The cards react to the pointer, so
//! a pointer resting on the bar, or a press the bar's own button took, must not
//! also light or press the card underneath it.

use super::*;

use aimer_container::Container;
use aimer_input::button::Button;
use aimer_input::mouse_region::MouseRegion;
use aimer_space::{Align, Alignment, Stack};
use winit::event::{ElementState, MouseButton};

#[derive(Default)]
struct Counters {
    card_enters: Cell<usize>,
    card_exits: Cell<usize>,
    card_taps: Cell<usize>,
    bar_taps: Cell<usize>,
}

/// A card that fills the window, under a 200 x 60 bar at the bottom center
/// (x 100..300, y 240..300). The bar's button covers x 110..170, y 250..290.
fn start() -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Counters>) {
    let counters = Rc::new(Counters::default());
    let card = {
        let (enter, exit, tap) = (counters.clone(), counters.clone(), counters.clone());
        MouseRegion::new()
            .on_hover_enter(move || enter.card_enters.set(enter.card_enters.get() + 1))
            .on_hover_exit(move || exit.card_exits.set(exit.card_exits.get() + 1))
            .box_child(
                Button::new()
                    .on_press(move || tap.card_taps.set(tap.card_taps.get() + 1))
                    .child(SizedBox::new().width(Dimension::Percent(100.0))
                        .height(Dimension::Percent(100.0))),
            )
    };
    let bar = {
        let tap = counters.clone();
        Align::new().alignment(Alignment::BotCenter).child(
            Container::new().width(200).height(60).color(Color::Rgb(60, 40, 40)).child(
                Row::new().children(vec![
                    SizedBox::new().width(10).boxed(),
                    Button::new()
                        .on_press(move || tap.bar_taps.set(tap.bar_taps.get() + 1))
                        .child(SizedBox::new().width(60).height(40))
                        .boxed(),
                ]),
            ),
        )
    };
    let mut app = AimerApp::start_headless_with(
        Stack::new().children(vec![card.boxed(), bar.boxed()]),
        HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(-10.0, -10.0),
    });
    app.pump_frames(3);
    (app, counters)
}

fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.pump_frames(2);
}

fn click<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) {
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(2);
    }
}

#[test]
fn a_card_away_from_the_bar_is_hovered_and_pressed() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();
    move_to(&mut app, 50.0, 100.0);
    click(&mut app);
    assert_eq!(counters.card_enters.get(), 1);
    assert_eq!(counters.card_taps.get(), 1);
    assert_eq!(counters.bar_taps.get(), 0);
}

#[test]
fn resting_on_the_bar_does_not_hover_the_card_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();
    // The empty part of the bar, between its button and its right edge.
    move_to(&mut app, 250.0, 270.0);
    assert_eq!(counters.card_enters.get(), 0, "the card under the bar was hovered");
}

#[test]
fn moving_onto_the_bar_ends_the_hover_of_the_card_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();
    move_to(&mut app, 50.0, 100.0);
    assert_eq!(counters.card_enters.get(), 1);
    move_to(&mut app, 250.0, 270.0);
    assert_eq!(counters.card_exits.get(), 1, "the card stayed hovered under the bar");
}

#[test]
fn a_press_on_the_bar_does_not_reach_the_card_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();
    move_to(&mut app, 250.0, 270.0);
    click(&mut app);
    assert_eq!(counters.card_taps.get(), 0, "a press on the bar pressed the card");
}

#[test]
fn a_press_the_bars_button_takes_does_not_also_reach_the_card_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();
    move_to(&mut app, 140.0, 270.0);
    assert_eq!(counters.card_enters.get(), 0, "the card was hovered through the button");
    click(&mut app);
    assert_eq!(counters.bar_taps.get(), 1);
    assert_eq!(counters.card_taps.get(), 0, "the press leaked past the button that took it");
}
