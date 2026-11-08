//! A driven app says so: the badge names who is attached, and the injected
//! mouse is drawn where the driver put it.

use aimer_cupid::draw_cmd::{DrawCommand as V2DrawCommand, RenderOp};
use aimer_platform::attachment::{self, MarkerPhase, SyntheticPointer};

use super::*;

/// Clears the process-wide attachment list when a test ends, pass or fail.
struct Detach;
impl Drop for Detach {
    fn drop(&mut self) {
        attachment::set_attached(Vec::new());
    }
}

fn options() -> HeadlessOptions {
    HeadlessOptions { size: PhysicalSize::new(800, 600), scale_factor: 1.0 }
}

fn recording() -> RecordingWidget {
    RecordingWidget { builds: Arc::new(AtomicUsize::new(0)), cancels: Arc::new(AtomicUsize::new(0)) }
}

/// Every command of every painted node, flattened.
fn painted<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<V2DrawCommand> {
    app.app
        .render_tree()
        .render_all()
        .iter()
        .filter_map(|operation| if let RenderOp::Draw(item) = operation { Some(item.snapshot().commands.to_vec()) } else { None })
        .flatten()
        .collect()
}

fn badge_text<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Option<String> {
    painted(app).into_iter().find_map(|c| match c {
        V2DrawCommand::DrawText { text, .. } if text.starts_with("driven by") => Some(text.to_string()),
        _ => None,
    })
}

/// Where each pointer icon's tip is in the window: its commands are local to the
/// node that draws them, so the node's origin is added back.
fn pointer_icons<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<(f32, f32)> {
    let tree = app.app.render_tree();
    tree.render_all()
        .iter()
        .filter_map(|operation| if let RenderOp::Draw(item) = operation { Some(item) } else { None })
        .flat_map(|item| {
            let origin = tree.element_bounds(item.element).map(|b| (b.x, b.y)).unwrap_or((0.0, 0.0));
            item.snapshot()
                .commands
                .iter()
                .filter_map(|c| if let V2DrawCommand::Svg { destination, .. } = c { Some((origin.0 + destination.x, origin.1 + destination.y)) } else { None })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn moved(x: f32, y: f32) -> AimerNativePlatformEvent {
    AimerNativePlatformEvent::SyntheticPointer(SyntheticPointer {
        touch: false,
        id: 0,
        phase: MarkerPhase::Moved,
        pos: Vec2d { x, y },
    })
}

#[test]
fn an_app_nobody_drives_shows_nothing_extra() {
    let _detach = Detach;
    let mut app = AimerApp::start_headless_with(recording(), options());
    app.render_frame();
    assert_eq!(badge_text(&app), None);
    assert!(pointer_icons(&app).is_empty());
}

#[test]
fn attaching_shows_the_badge_and_detaching_removes_it() {
    let _detach = Detach;
    let mut app = AimerApp::start_headless_with(recording(), options());
    app.render_frame();

    attachment::set_attached(vec!["aimer-cli".to_owned()]);
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    assert_eq!(badge_text(&app).as_deref(), Some("driven by aimer-cli"));

    attachment::set_attached(Vec::new());
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    let (_, packet) = direct_headless_frame_packet(&mut app);
    assert_eq!(badge_text(&app), None);
    assert!(!packet.metadata().damage().is_empty(), "the covered pixels are restored");
}

#[test]
fn the_injected_mouse_is_drawn_with_its_tip_where_the_driver_put_it_and_leaves_with_it() {
    let _detach = Detach;
    let mut app = AimerApp::start_headless_with(recording(), options());
    app.render_frame();
    attachment::set_attached(vec!["aimer-cli".to_owned()]);
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);

    app.send_user_event(moved(300.0, 200.0));
    direct_headless_frame_packet(&mut app);
    let [(x, y)] = pointer_icons(&app)[..] else { panic!("one pointer: {:?}", pointer_icons(&app)) };
    assert!((x - (300.0 - 3.96)).abs() < 0.01 && (y - (200.0 - 3.96)).abs() < 0.01, "({x}, {y})");

    app.send_user_event(moved(500.0, 100.0));
    direct_headless_frame_packet(&mut app);
    assert_eq!(pointer_icons(&app).len(), 1, "it moved; it was not duplicated");

    app.send_user_event(AimerNativePlatformEvent::SyntheticPointer(SyntheticPointer {
        touch: false,
        id: 0,
        phase: MarkerPhase::Left,
        pos: Vec2d::default(),
    }));
    direct_headless_frame_packet(&mut app);
    assert!(pointer_icons(&app).is_empty());
}

#[test]
fn a_tool_that_leaves_takes_its_pointer_with_it() {
    let _detach = Detach;
    let mut app = AimerApp::start_headless_with(recording(), options());
    app.render_frame();
    attachment::set_attached(vec!["aimer-cli".to_owned()]);
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    app.send_user_event(moved(10.0, 10.0));
    direct_headless_frame_packet(&mut app);
    assert_eq!(pointer_icons(&app).len(), 1);

    attachment::set_attached(Vec::new());
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    assert!(pointer_icons(&app).is_empty());
}

#[test]
fn the_overlay_does_not_intercept_input_under_the_badge_or_the_pointer() {
    let _detach = Detach;
    let clicks = Rc::new(Cell::new(0));
    let recorded = clicks.clone();
    let mut app = AimerApp::start_headless_with(
        aimer_input::button::Button::new().on_press(move || recorded.set(recorded.get() + 1)).child(aimer_container::SizedBox::new().width(800.0).height(600.0)),
        options(),
    );
    app.render_frame();
    attachment::set_attached(vec!["aimer-cli".to_owned()]);
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    app.send_user_event(moved(400.0, 300.0));
    direct_headless_frame_packet(&mut app);
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: winit::event::DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(400.0, 300.0),
    });
    app.send_window_event(WindowEvent::MouseInput {
        device_id: winit::event::DeviceId::dummy(),
        state: winit::event::ElementState::Pressed,
        button: winit::event::MouseButton::Left,
    });
    app.send_window_event(WindowEvent::MouseInput {
        device_id: winit::event::DeviceId::dummy(),
        state: winit::event::ElementState::Released,
        button: winit::event::MouseButton::Left,
    });
    assert_eq!(clicks.get(), 1, "the marker is a picture, not a target");
}
