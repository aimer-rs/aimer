//! A finger that rests on the scroll bar thumb takes it, with nothing but frames
//! to notice.
//!
//! A finger at rest sends no events, so the hold can only complete on a frame.
//! These tests run the real frame loop to see how long that takes.

use super::*;

use aimer_time::{Duration, Instant};

use aimer_scroll::constants::SCROLLBAR_THUMB_HOLD_MS;
use winit::event::Touch;

const VIEWPORT: u32 = 200;

struct Tall {
    controller: ScrollController,
}

impl Widget for Tall {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        SizedBox::new()
            .width(Dimension::Px(VIEWPORT as f32))
            .height(Dimension::Px(400.0))
            .child(
                Scrollable::new()
                    .controller(self.controller)
                    .child(SizedBox::new().width(Dimension::Px(VIEWPORT as f32)).height(Dimension::Px(2000.0))),
            )
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Tall {}

fn touch<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, phase: TouchPhase, x: f64, y: f64) {
    app.send_window_event(WindowEvent::Touch(Touch {
        device_id: DeviceId::dummy(),
        phase,
        location: PhysicalPosition::new(x, y),
        force: None,
        id: 1,
    }));
}

/// Runs frames, one at a time, until `done` or `limit` passes. Returns how long
/// it took, or `None` if it never finished.
fn frames_until<W: Widget + 'static>(
    app: &mut HeadlessAimerApp<W>,
    limit: Duration,
    mut done: impl FnMut() -> bool,
) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < limit {
        app.pump_frames(1);
        if done() {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    None
}

#[test]
fn a_finger_resting_on_the_thumb_takes_it_on_the_frames_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        Tall {
            controller: controller.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(VIEWPORT, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // Scrolling shows the bar; let it finish fading in so a finger can find it.
    controller.jump_to(Vec2d { x: 0.0, y: 100.0 });
    frames_until(&mut app, Duration::from_millis(400), || false);
    assert!(controller.scroll_bar_opacity() > 0.9, "the bar is showing");

    // Content 2000 high in a 400 viewport: an 80 px thumb, 20 px down at this
    // offset, in the 12 px strip at the right edge.
    touch(&mut app, TouchPhase::Started, f64::from(VIEWPORT) - 6.0, 60.0);
    assert!(!controller.is_thumb_grabbed(), "pressing is not yet taking");

    let waited = frames_until(&mut app, Duration::from_millis(1500), || {
        controller.is_thumb_grabbed()
    });

    let waited = waited.expect("the hold never completed: the frames stopped asking for the next one");
    let hold = Duration::from_millis(SCROLLBAR_THUMB_HOLD_MS);
    assert!(
        waited < hold + Duration::from_millis(120),
        "took {waited:?} for a {hold:?} hold"
    );

    // The thumb then grows over the next frames, and keeps being held.
    frames_until(&mut app, Duration::from_millis(300), || false);
    assert!(controller.is_thumb_grabbed());
}

#[test]
fn a_finger_on_a_hidden_thumb_brings_the_bar_back_and_takes_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        Tall {
            controller: controller.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(VIEWPORT, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    assert!(
        controller.scroll_bar_opacity() < 0.05,
        "nothing has scrolled, so the bar is not showing"
    );

    // Offset zero: the 80 px thumb sits at the top of the 12 px strip.
    touch(&mut app, TouchPhase::Started, f64::from(VIEWPORT) - 6.0, 40.0);
    assert!(!controller.is_thumb_grabbed());

    let waited = frames_until(&mut app, Duration::from_millis(1500), || {
        controller.is_thumb_grabbed()
    })
    .expect("the hold never completed: a hidden thumb was not brought back");

    assert!(controller.scroll_bar_opacity() >= 0.3, "it was taken in plain sight");
    let hold = Duration::from_millis(SCROLLBAR_THUMB_HOLD_MS);
    assert!(
        waited < hold + Duration::from_millis(120),
        "took {waited:?} for a {hold:?} hold"
    );
}

#[test]
fn a_finger_resting_on_the_content_never_takes_the_thumb() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        Tall {
            controller: controller.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(VIEWPORT, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    controller.jump_to(Vec2d { x: 0.0, y: 100.0 });
    frames_until(&mut app, Duration::from_millis(400), || false);

    // Far from the 12 px strip at the right edge, and well clear of its margin.
    touch(&mut app, TouchPhase::Started, 60.0, 200.0);
    let taken = frames_until(&mut app, Duration::from_millis(500), || {
        controller.is_thumb_grabbed()
    });

    assert!(taken.is_none(), "resting on the content took the thumb after {taken:?}");
}
