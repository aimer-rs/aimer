use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use winit::event::ElementState;
use winit::keyboard::{Key, NamedKey};

use super::*;

fn overlay_node<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Option<RenderNodeId> {
    app.app
        .render_tree()
        .render_all()
        .iter()
        .find_map(|operation| {
            let RenderOp::Draw(item) = operation else {
                return None;
            };
            item.snapshot()
                .commands
                .iter()
                .any(|command| {
                    matches!(command,
                        V2DrawCommand::DrawText { text, .. } if text.starts_with("Aimer debug")
                    )
                })
                .then_some(item.element)
        })
}

#[test]
fn debug_overlay_is_automatic_retained_and_removed_with_damage() {
    let builds = Arc::new(AtomicUsize::new(0));
    let mut app = AimerApp::start_headless_with(
        RecordingWidget {
            builds: builds.clone(),
            cancels: Arc::new(AtomicUsize::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(1024, 640),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    assert!(overlay_node(&app).is_none());
    assert!(!app.take_redraw_request());

    assert!(
        app.app
            .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false)
    );
    assert!(app.take_redraw_request());
    let (scale, packet) = direct_headless_frame_packet(&mut app);
    assert_eq!(scale, 2.0);
    let node = overlay_node(&app).expect("F3 paints a retained overlay in every debug app");
    assert!(
        packet
            .render_plan()
            .unwrap()
            .local_v2_revision(node)
            .is_some()
    );
    assert!(!packet.metadata().damage().is_empty());
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert!(
        !app.take_redraw_request(),
        "diagnostics must not keep an idle app rendering"
    );

    assert!(
        app.app
            .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, true)
    );
    assert!(
        app.app
            .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Released, false)
    );
    assert!(!app.take_redraw_request());
    assert!(
        app.app
            .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false)
    );
    let (_, packet) = direct_headless_frame_packet(&mut app);
    assert!(overlay_node(&app).is_none());
    assert!(
        !packet.metadata().damage().is_empty(),
        "hiding restores the covered pixels"
    );
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}

#[test]
fn debug_overlay_survives_resize_and_preserves_ime_and_focus() {
    let focus = FocusNode::new();
    let mut app = AimerApp::start_headless_with(
        TextField::new().focus_node(focus.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 800),
            scale_factor: 1.0,
        },
    );
    app.render_frame();
    focus_headless_field(&mut app, &focus);
    app.app.ime_composing = true;
    app.app
        .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false);
    app.render_frame();
    assert!(app.app.ime_composing);
    assert!(focus.has_focus());
    assert!(overlay_node(&app).is_some());
    app.send_window_event(WindowEvent::Resized(PhysicalSize::new(240, 180)));
    let node = overlay_node(&app).expect("tree synchronization retains the visible overlay");
    let bounds = app.app.render_tree().element_bounds(node).unwrap();
    assert!(bounds.width <= 240.0 && bounds.height <= 180.0);
}

#[test]
fn debug_overlay_does_not_intercept_pointer_input_under_its_panel() {
    let clicks = Rc::new(Cell::new(0));
    let recorded_clicks = clicks.clone();
    let mut app = AimerApp::start_headless_with(
        aimer_input::button::Button::new()
            .on_press(move || recorded_clicks.set(recorded_clicks.get() + 1))
            .child(SizedBox::new().width(360.0).height(200.0)),
        HeadlessOptions {
            size: PhysicalSize::new(1024, 640),
            scale_factor: 1.0,
        },
    );
    app.render_frame();
    app.app
        .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false);
    app.render_frame();
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 20.0),
    });
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
    }
    assert_eq!(clicks.get(), 1);
}

#[cfg(all(feature = "wgpu", target_os = "macos"))]
#[test]
fn debug_overlay_gpu_pixels_restore_exactly_after_hiding() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut gpu =
        resize_gpu::ResizeGpu::new().expect("Metal adapter for overlay pixel verification");
    let mut app = AimerApp::start_headless_with(
        aimer_container::Container::new()
            .color(Color::BLUE)
            .width(1024.0)
            .height(640.0)
            .child(aimer_container::ZeroSizedBox),
        HeadlessOptions {
            size: PhysicalSize::new(1024, 640),
            scale_factor: 1.0,
        },
    );
    let (_, base) = direct_headless_frame_packet(&mut app);
    let base_pixels = gpu.present(&base, 1024, 640);
    app.app
        .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false);
    let (_, visible) = direct_headless_frame_packet(&mut app);
    let overlay_pixels = gpu.present(&visible, 1024, 640);
    assert_ne!(overlay_pixels, base_pixels);
    if let Some(path) = std::env::var_os("AIMER_TEST_OVERLAY_RGBA") {
        std::fs::write(path, &overlay_pixels).unwrap();
    }
    app.app
        .handle_debug_key(&Key::Named(NamedKey::F3), ElementState::Pressed, false);
    let (_, hidden) = direct_headless_frame_packet(&mut app);
    assert_eq!(gpu.present(&hidden, 1024, 640), base_pixels);
}
