//! A `CollapsibleList` animates its body through intermediate heights when its
//! header is activated, instead of jumping between the two end states.

use super::*;

use aimer_collapsible::{CollapsibleList, ListBody, ListHeader};
use winit::event::ElementState;

const BODY_HEIGHT: f32 = 100.0;

fn find<'a>(element: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
    if element.debug_name() == name {
        return Some(element);
    }
    let mut found = None;
    element.visit_children(&mut |child| {
        if found.is_none() {
            found = find(child, name);
        }
    });
    found
}

fn body_height(app: &HeadlessAimerApp<impl Widget + 'static>) -> Option<f32> {
    let root = app.app.widget_root.as_ref()?;
    let body = find(root.as_ref(), "AnimatedCollapseElement")?;
    let node = app.app.render_node_for_element(body.id())?;
    Some(app.app.render_tree().element_bounds(node).ok()?.height)
}

fn tap(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
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

/// Taps the header and returns the body height seen after each frame until the
/// frame loop goes idle.
fn tap_and_record(app: &mut HeadlessAimerApp<impl Widget + 'static>) -> Vec<f32> {
    tap(app, 20.0, 12.0);
    let mut heights = Vec::new();
    for _ in 0..120 {
        if app.pump_frames(1) == 0 {
            return heights;
        }
        heights.push(body_height(app).expect("body stays mounted while animating"));
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
    panic!("the animation never settled: {heights:?}");
}

fn assert_animates(heights: &[f32], from: f32, to: f32) {
    let (low, high) = (from.min(to), from.max(to));
    assert!(
        heights.iter().any(|h| *h > low + 1.0 && *h < high - 1.0),
        "must visit intermediate heights, saw {heights:?}"
    );
    assert!(
        heights
            .windows(2)
            .all(|pair| (pair[1] - pair[0]) * (to - from) >= 0.0),
        "heights must move monotonically from {from} to {to}, saw {heights:?}"
    );
    assert_eq!(
        *heights.last().unwrap(),
        to,
        "settles on the target, saw {heights:?}"
    );
}

#[test]
fn collapsing_and_expanding_pass_through_intermediate_body_heights() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let list = CollapsibleList::new()
        .animation_millis(200)
        .header(ListHeader::new().child(SizedBox::new().width(100.0).height(24.0)))
        .body(ListBody::new().child(SizedBox::new().width(100.0).height(BODY_HEIGHT)));
    let mut app = AimerApp::start_headless_with(
        list,
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let full = body_height(&app).expect("expanded body is mounted");
    assert!(
        full >= BODY_HEIGHT,
        "expanded body is full height, got {full}"
    );

    assert_animates(&tap_and_record(&mut app), full, 0.0);
    assert_animates(&tap_and_record(&mut app), 0.0, full);
}
