//! The trackpad's pinch, twist and smart zoom travel from the windowing system
//! to a `GestureDetector` over the cursor, and stop at the first widget that
//! claims them.

use super::*;

use aimer_input::gesture::gesture_detector::GestureDetector;
use aimer_input::gesture::{GestureEvent, ScaleData, SmartZoomData};

/// What the detectors under test saw.
#[derive(Default)]
struct Seen {
    scales: RefCell<Vec<ScaleData>>,
    ends: Cell<usize>,
    zooms: RefCell<Vec<SmartZoomData>>,
    /// Gestures the inner, observing-only detector heard about.
    observed: RefCell<Vec<GestureEvent>>,
}

/// A 200x200 canvas that claims pinch and smart zoom, with a 100x100 button
/// that only *observes* its gestures sitting in its top-left corner.
struct Canvas {
    seen: Rc<Seen>,
}

impl Widget for Canvas {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let (scale, end, zoom, observe) = (
            self.seen.clone(),
            self.seen.clone(),
            self.seen.clone(),
            self.seen.clone(),
        );
        let button = GestureDetector::new()
            .on_gesture(move |event: GestureEvent| observe.observed.borrow_mut().push(event))
            .child(
                SizedBox::new()
                    .width(Dimension::Px(100.0))
                    .height(Dimension::Px(100.0)),
            );
        GestureDetector::new()
            .on_scale(move |data: ScaleData| scale.scales.borrow_mut().push(data))
            .on_gesture(move |event: GestureEvent| {
                if event == GestureEvent::ScaleEnd {
                    end.ends.set(end.ends.get() + 1);
                }
            })
            .on_smart_zoom(move |at: SmartZoomData| zoom.zooms.borrow_mut().push(at))
            .child(
                SizedBox::new()
                    .width(Dimension::Px(200.0))
                    .height(Dimension::Px(200.0))
                    .child(button),
            )
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Canvas {}

fn start() -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Seen>) {
    let seen = Rc::new(Seen::default());
    let mut app = AimerApp::start_headless_with(
        Canvas { seen: seen.clone() },
        HeadlessOptions {
            size: PhysicalSize::new(200, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    (app, seen)
}

fn pinch<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, delta: f64, phase: TouchPhase) {
    app.send_window_event(WindowEvent::PinchGesture {
        device_id: DeviceId::dummy(),
        delta,
        phase,
    });
}

#[test]
fn a_trackpad_pinch_over_the_canvas_zooms_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 150.0, y: 150.0 };

    pinch(&mut app, 0.0, TouchPhase::Started);
    pinch(&mut app, 0.5, TouchPhase::Moved);
    pinch(&mut app, 0.0, TouchPhase::Ended);

    let scales = seen.scales.borrow();
    assert_eq!(scales.len(), 1, "{scales:?}");
    assert_eq!(scales[0].scale, 1.5);
    assert_eq!((scales[0].focal_x, scales[0].focal_y), (150.0, 150.0));
    assert_eq!(seen.ends.get(), 1, "the pinch must end exactly once");
}

// The button only watches its gestures through `on_gesture`. It hears the
// pinch, but must not keep it from the canvas that zooms.
#[test]
fn a_button_that_merely_observes_does_not_swallow_the_pinch() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 50.0, y: 50.0 };

    pinch(&mut app, 0.25, TouchPhase::Started);

    assert_eq!(
        seen.scales.borrow().len(),
        1,
        "the canvas behind the button never zoomed"
    );
    assert!(
        seen.observed
            .borrow()
            .iter()
            .any(|event| matches!(event, GestureEvent::ScaleStart { .. })),
        "the observing button should still hear about it"
    );
}

#[test]
fn a_pinch_with_the_cursor_off_the_canvas_reaches_nothing() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 900.0, y: 900.0 };

    pinch(&mut app, 0.5, TouchPhase::Started);

    assert!(seen.scales.borrow().is_empty());
}

#[test]
fn a_counterclockwise_trackpad_twist_reports_a_negative_rotation() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 150.0, y: 150.0 };

    app.send_window_event(WindowEvent::RotationGesture {
        device_id: DeviceId::dummy(),
        delta: 90.0,
        phase: TouchPhase::Started,
    });

    let scales = seen.scales.borrow();
    assert_eq!(scales.len(), 1, "{scales:?}");
    assert!(
        (scales[0].rotation + std::f32::consts::FRAC_PI_2).abs() < 1e-5,
        "{}",
        scales[0].rotation
    );
    assert_eq!(scales[0].delta_scale, 1.0);
}

#[test]
fn smart_zoom_reports_the_cursor_position() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, seen) = start();
    app.app.cursor_pos = Vec2d { x: 120.0, y: 160.0 };

    app.send_window_event(WindowEvent::DoubleTapGesture {
        device_id: DeviceId::dummy(),
    });

    assert_eq!(
        *seen.zooms.borrow(),
        vec![SmartZoomData { x: 120.0, y: 160.0 }]
    );
}
