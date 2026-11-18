//! The pressure of a press on a Force Touch trackpad travels from the windowing system to the `MouseRegion` at the
//! cursor, which is called once for each deep press.

use super::*;

use aimer_events::element::Modifiers;
use aimer_input::mouse_region::MouseRegion;

/// A 200x200 area with a region over its top-left 100x100, which records where it is force-clicked.
struct Area {
    seen: Rc<RefCell<Vec<Vec2d>>>,
}

impl Widget for Area {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let seen = self.seen;
        SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(200.0))
            .child(
                MouseRegion::new()
                    .on_force_press(move |at| seen.borrow_mut().push(at))
                    .child(SizedBox::new().width(Dimension::Px(100.0)).height(Dimension::Px(100.0))),
            )
            .to_element(ctx)
    }
}

fn start() -> (HeadlessAimerApp<impl Widget + 'static>, Rc<RefCell<Vec<Vec2d>>>) {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        Area { seen: Rc::clone(&seen) },
        HeadlessOptions { size: PhysicalSize::new(200, 200), scale_factor: 1.0 },
    );
    app.pump_frames(3);
    (app, seen)
}

fn press<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, pressure: f32, stage: i64) {
    app.send_window_event(WindowEvent::TouchpadPressure { device_id: DeviceId::dummy(), pressure, stage });
}

#[test]
fn a_deep_press_over_a_region_calls_it_once_with_where_the_cursor_is() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 40.0, y: 60.0 };

    press(&mut app, 0.3, 1);
    assert!(seen.borrow().is_empty(), "a click is not a force click");
    press(&mut app, 0.9, 2);
    press(&mut app, 1.0, 2);
    assert_eq!(*seen.borrow(), vec![Vec2d { x: 40.0, y: 60.0 }], "once, however long it is held");
    press(&mut app, 0.0, 0);
    press(&mut app, 0.9, 2);
    assert_eq!(seen.borrow().len(), 2, "and again for the next deep press");
}

#[test]
fn a_deep_press_beside_the_region_is_not_the_regions() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 150.0, y: 150.0 };

    press(&mut app, 1.0, 2);

    assert!(seen.borrow().is_empty());
}

#[test]
fn a_pressure_that_is_not_a_number_does_not_reach_the_region_as_one() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 40.0, y: 60.0 };

    // The stage still says that the press is deep, which is what a region goes by; the number is made safe.
    press(&mut app, f32::NAN, 2);

    assert_eq!(seen.borrow().len(), 1);
}

/// A pressure that arrives as an input injected by the host that drives the app, which says where it is itself.
fn injected<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f32, y: f32, stage: aimer_events::element::PressureStage) {
    app.send_user_event(AimerNativePlatformEvent::NativeInput {
        event: aimer_events::element::ElementEvent::Pressure { pos: Vec2d { x, y }, pressure: 1.0, stage },
        modifiers: Modifiers::default(),
    });
}

#[test]
fn a_pressure_that_a_host_injects_goes_where_it_says_and_not_where_the_cursor_was() {
    use aimer_events::element::PressureStage::{Force, None as Off};
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    // The cursor is beside the region, and the injected press is over it.
    app.app.cursor_pos = Vec2d { x: 150.0, y: 150.0 };

    injected(&mut app, 30.0, 40.0, Force);

    assert_eq!(*seen.borrow(), vec![Vec2d { x: 30.0, y: 40.0 }]);
    injected(&mut app, 30.0, 40.0, Off);
    // And beside the region it is not the region's.
    injected(&mut app, 180.0, 180.0, Force);
    assert_eq!(seen.borrow().len(), 1);
}
