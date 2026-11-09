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

/// Whether the badge's icon is painted: the pill holds the one drawing that is
/// not a pointer.
fn badge_shown<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> bool {
    painted(app)
        .into_iter()
        .any(|c| matches!(c, V2DrawCommand::Svg { scene, .. } if Arc::ptr_eq(&scene, crate::attach_overlay::badge_scene())))
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
                .filter_map(|c| match c {
                    // The badge's icon is a drawing too, but it is not a pointer.
                    V2DrawCommand::Svg { scene, .. } if Arc::ptr_eq(scene, crate::attach_overlay::badge_scene()) => None,
                    V2DrawCommand::Svg { destination, .. } => Some((origin.0 + destination.x, origin.1 + destination.y)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Lets the fade that follows a detach run its course: the overlay is timed by the
/// clock, and one frame at the start and one after the end are all it needs.
///
/// Returns the frame after the fade, which is the one that takes the overlay away.
fn fade_out<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) -> FramePacket {
    std::thread::sleep(crate::attach_overlay::PRESENCE_FADE + std::time::Duration::from_millis(60));
    direct_headless_frame_packet(app).1
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
    assert!(!badge_shown(&app));
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
    assert!(badge_shown(&app), "attaching shows the badge");

    attachment::set_attached(Vec::new());
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    assert!(badge_shown(&app), "it fades out rather than vanishing");
    let packet = fade_out(&mut app);
    assert!(!badge_shown(&app), "and is removed once it has");
    assert!(!packet.metadata().damage().is_empty(), "the covered pixels are restored");
}

#[test]
fn the_tool_that_launched_the_app_is_not_someone_controlling_it() {
    let _detach = Detach;
    let mut app = AimerApp::start_headless_with(recording(), options());
    app.render_frame();

    // `aimer run` is connected for the whole run: no badge for that.
    attachment::set_connections(vec!["aimer-cli".to_owned()], Vec::new());
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    assert!(!badge_shown(&app), "supervising is not controlling");

    // An agent connects, and stays connected without doing anything: the badge
    // stays up for as long as it is there, however idle it is.
    attachment::set_connections(vec!["aimer-cli".to_owned(), "mcp:codex".to_owned()], vec!["mcp:codex".to_owned()]);
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    assert!(badge_shown(&app), "an agent is remote-controlling the app");
    std::thread::sleep(std::time::Duration::from_millis(500));
    direct_headless_frame_packet(&mut app);
    assert!(badge_shown(&app), "and idle does not end that");

    // The agent leaves; the launcher stays. The badge goes.
    attachment::set_connections(vec!["aimer-cli".to_owned()], Vec::new());
    app.send_user_event(AimerNativePlatformEvent::AttachmentChanged);
    direct_headless_frame_packet(&mut app);
    fade_out(&mut app);
    assert!(!badge_shown(&app), "the agent is gone, whoever else is still connected");
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
    let tip = crate::attach_overlay::ICON_TIP;
    assert!((x - (300.0 - tip)).abs() < 0.01 && (y - (200.0 - tip)).abs() < 0.01, "({x}, {y})");

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
    assert_eq!(pointer_icons(&app).len(), 1, "it fades out rather than vanishing");
    fade_out(&mut app);
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
