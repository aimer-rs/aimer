use super::*;
use crate::raster::{FrameWorker, WorkerPresenter};
use crate::render_ctx::native_ctx::{BackendOutput, NativeRenderContext};
use std::sync::mpsc;
use std::time::Duration;

struct Stalled {
    gate: mpsc::Receiver<()>,
    entered: mpsc::Sender<()>,
}
impl WorkerPresenter for Stalled {
    type Output = BackendOutput;
    fn resize(&mut self, _: PhysicalSize<u32>) {}
    fn render(&mut self, _: &FramePacket) -> BackendOutput {
        self.entered.send(()).unwrap();
        self.gate.recv_timeout(Duration::from_secs(5)).unwrap();
        BackendOutput::Presented(true)
    }
}

#[test]
fn a_completion_redraw_preserves_pending_animation_and_scroll_requests() {
    let mut app = AimerApp::start_headless(PreeditWidget {
        preedits: Arc::new(Mutex::new(Vec::new())),
    });
    app.render_frame();
    for kind in [
        FrameRequestKind::Scoped,
        FrameRequestKind::ScrollPhysics,
        FrameRequestKind::Full,
    ] {
        app.app.frame_request_reason.set(Some(kind));
        app.app.request_deferred_render_frame();
        assert!(app.take_redraw_request());
        assert_eq!(app.app.frame_request_reason.get(), Some(kind));
    }
}

#[test]
fn native_admission_preserves_pending_work_while_platform_input_is_delivered() {
    let mut app = AimerApp::start_headless(PreeditWidget {
        preedits: Arc::new(Mutex::new(Vec::new())),
    });
    app.render_frame();
    let (release, gate) = mpsc::channel();
    let (entered, rendering) = mpsc::channel();
    let (wake, wakes) = mpsc::channel();
    let worker = FrameWorker::spawn(
        move || Ok(Stalled { gate, entered }),
        move || {
            let _ = wake.send(());
        },
    );
    app.app.render_ctx = NativeRenderContext::with_test_worker(worker, PhysicalSize::new(8, 8));
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    app.app.render_ctx.poll_completions();
    app.app.render_ctx.render_frame(|_, _, _| {});
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    app.app.render_ctx.render_frame(|_, _, _| {});
    app.app.note_frame_request(FrameRequestKind::Full);
    assert!(!app.app.admit_render_frame());
    app.send_user_event(AimerNativePlatformEvent::SetPreedit {
        text: "ni".into(),
        cursor: None,
    });
    assert!(app.is_composing());
    assert_eq!(
        app.app.frame_request_reason.get(),
        Some(FrameRequestKind::Full)
    );
    release.send(()).unwrap();
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    let completion = app.app.render_ctx.poll_completions();
    assert!(completion.wake_frame);
    assert!(app.app.admit_render_frame());
    release.send(()).unwrap();
    app.app.render_ctx.suspend();
}

#[test]
fn a_button_click_is_delivered_while_both_native_frame_slots_are_busy() {
    let presses = Rc::new(Cell::new(0));
    let observed = presses.clone();
    let mut app = AimerApp::start_headless_with(
        aimer_input::button::Button::new()
            .on_press(move || observed.set(observed.get() + 1))
            .child(SizedBox::new().width(128).height(40)),
        HeadlessOptions {
            size: PhysicalSize::new(320, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(2);
    let (release, gate) = mpsc::channel();
    let (entered, rendering) = mpsc::channel();
    let (wake, wakes) = mpsc::channel();
    let worker = FrameWorker::spawn(
        move || Ok(Stalled { gate, entered }),
        move || {
            let _ = wake.send(());
        },
    );
    app.app.render_ctx = NativeRenderContext::with_test_worker(worker, PhysicalSize::new(320, 200));
    wakes.recv_timeout(Duration::from_secs(5)).unwrap();
    app.app.render_ctx.poll_completions();
    app.app.render_ctx.render_frame(|_, _, _| {});
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    app.app.render_ctx.render_frame(|_, _, _| {});
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 20.0),
    });
    assert!(!app.app.admit_render_frame());
    for state in [
        winit::event::ElementState::Pressed,
        winit::event::ElementState::Released,
    ] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: winit::event::MouseButton::Left,
        });
        assert!(!app.app.admit_render_frame());
    }
    // Release the renderer even if the assertion below fails.
    release.send(()).unwrap();
    rendering.recv_timeout(Duration::from_secs(5)).unwrap();
    release.send(()).unwrap();
    app.app.render_ctx.suspend();
    assert_eq!(presses.get(), 1);
}
