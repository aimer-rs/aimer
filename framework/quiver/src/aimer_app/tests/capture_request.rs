//! Asking for a picture of the window wakes an app that is not changing: an idle
//! app draws no frames, so without this there would be nothing to copy.

use aimer_platform::capture;

use super::*;

fn recording() -> RecordingWidget {
    RecordingWidget { builds: Arc::new(AtomicUsize::new(0)), cancels: Arc::new(AtomicUsize::new(0)) }
}

/// The frame it asks for need not carry damage: an app that did not change has
/// none. The renderer copies a complete picture anyway, by redrawing the packet's
/// whole retained plan for a capture frame.
#[test]
fn a_capture_request_makes_an_idle_app_draw_a_frame() {
    let mut app = AimerApp::start_headless_with(recording(), HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 });
    app.render_frame();
    assert!(!app.take_redraw_request(), "an idle app asks for nothing");

    app.send_user_event(AimerNativePlatformEvent::CaptureRequested);
    assert!(app.take_redraw_request(), "the request is a reason to draw");
}

#[test]
fn a_request_for_a_window_that_cannot_draw_is_answered_not_awaited() {
    // No event loop is running in a unit test, so there is no window to draw.
    let reply = capture::request_and_wake();
    assert_eq!(reply.recv_timeout(std::time::Duration::from_secs(1)).unwrap().unwrap_err(), capture::Error::NoWindow);
}

/// How many operations the plan of one more frame carries.
fn operations_in_next_frame<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) -> usize {
    app.canvas.begin_frame();
    let window = app.window.clone();
    let (_, mut drawer) = app.app.split_for_frame(window);
    drawer.draw(&app.canvas, 400, 300);
    let plan = app.canvas.take_retained_render_plan().expect("the frame lowers to a plan");
    plan.operations_for_region(aimer_cupid::damage_region::DamageRect::new(0, 0, 400, 300)).count()
}

/// An idle frame is culled to its damage, which is nothing, and the renderer's
/// full-size redraw for a capture then has nothing to draw: the picture comes back
/// one colour. A frame asked for by a capture must carry the whole tree.
#[test]
fn a_frame_for_a_capture_carries_the_whole_tree_even_when_nothing_changed() {
    let picture = SizedBox::new().width(200.0).height(100.0).color(Color::Rgb(200, 30, 30));
    let mut app = AimerApp::start_headless_with(picture, HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 });
    app.render_frame();
    app.render_frame();
    assert_eq!(operations_in_next_frame(&mut app), 0, "premise: an idle frame is culled to nothing");

    let _reply = capture::request();
    assert!(operations_in_next_frame(&mut app) > 0, "a capture frame must not be culled");
}
