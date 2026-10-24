//! A frame delivers the smoothed scroll to the widget tree once.

use super::*;

/// Scrolls a recording widget by one wheel notch and returns how many scroll
/// events each drawn frame delivered, together with the distance delivered.
fn frames_of_one_notch(delta: f64) -> (Vec<usize>, f32) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless(ScrollRecordingWidget {
        events: events.clone(),
    });
    app.render_frame();
    app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };

    app.send_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
        phase: TouchPhase::Moved,
    });

    let mut per_frame = Vec::new();
    let mut seen = 0;
    while app.app.scroll_smoother.is_active() || per_frame.is_empty() {
        app.render_frame();
        let delivered = events.lock().unwrap().len();
        per_frame.push(delivered - seen);
        seen = delivered;
        assert!(per_frame.len() < 200, "the smoother never finished");
    }
    // A closing step may follow the last distance.
    app.render_frame();
    let delivered = events.lock().unwrap().len();
    per_frame.push(delivered - seen);

    let total = events
        .lock()
        .unwrap()
        .iter()
        .map(|(delta, _, _)| delta.y)
        .sum();
    (per_frame, total)
}

#[test]
fn a_drawn_frame_delivers_at_most_one_scroll_step() {
    let (per_frame, _) = frames_of_one_notch(-400.0);

    assert!(
        per_frame.len() > 2,
        "the notch was delivered in {} frames",
        per_frame.len()
    );
    assert!(
        per_frame.iter().all(|delivered| *delivered <= 1),
        "events delivered per frame: {per_frame:?}"
    );
    assert!(per_frame.iter().sum::<usize>() >= 2, "{per_frame:?}");
}

#[test]
fn the_whole_distance_still_arrives_in_the_scrolls_direction() {
    let (_, total) = frames_of_one_notch(-400.0);
    assert!(total < 0.0, "delivered {total}");

    let (_, opposite) = frames_of_one_notch(400.0);
    assert!(opposite > 0.0, "delivered {opposite}");
    assert!(
        (total + opposite).abs() < 1e-3,
        "opposite notches must deliver equal and opposite distance: {total} vs {opposite}"
    );
}
