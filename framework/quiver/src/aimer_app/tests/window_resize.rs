use super::*;
use aimer_widget::WindowMetrics;

#[test]
fn uikit_resize_and_redraw_share_one_deferred_frame() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 1600),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    for size in [PhysicalSize::new(1300, 1500), PhysicalSize::new(1800, 1100)] {
        app.window.update_headless_metrics(size, 2.0);
        assert!(WindowEventHandler::defer_uikit_frame_event(
            &mut app.app,
            &WindowEvent::Resized(PhysicalSize::new(2732, 2048)),
        ));
        assert_eq!(
            app.app.pending_resize,
            Some(size),
            "resize must wait for the display tick"
        );
    }
    app.window.take_redraw_request();
    assert!(WindowEventHandler::defer_uikit_frame_event(
        &mut app.app,
        &WindowEvent::RedrawRequested
    ));
    assert!(
        app.window.take_redraw_request(),
        "UIKit redraw must schedule a display tick"
    );
    assert_eq!(app.app.pending_resize, Some(PhysicalSize::new(1800, 1100)));
    assert!(!WindowEventHandler::defer_uikit_frame_event(
        &mut app.app,
        &WindowEvent::Focused(true)
    ));
    assert!(
        !app.window.take_redraw_request(),
        "non-frame events stay on their normal path"
    );
    app.render_frame();
    assert_eq!(
        app.last_frame_result.as_ref().unwrap().1.target_size(),
        (1800, 1100)
    );
}

#[test]
fn display_tick_delivers_latest_resize_without_a_second_redraw_request() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 1600),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    app.window.take_redraw_request();
    for size in [PhysicalSize::new(1300, 1500), PhysicalSize::new(1800, 1100)] {
        app.window.update_headless_metrics(size, 2.0);
        WindowEventHandler::refresh_uikit_viewport(&mut app.app);
    }
    let mut presented_size = None;
    crate::handler::user_events::handle_user_event(
        &mut app.app,
        AimerNativePlatformEvent::FrameReady,
        |handler, _kind| {
            // The platform chooses to render here, on the display tick,
            // rather than waiting for another UIView draw callback.
            presented_size = handler.pending_resize.take();
        },
    );
    assert_eq!(presented_size, Some(PhysicalSize::new(1800, 1100)));
    assert_eq!(app.app.pending_resize, None);
    assert!(
        !app.window.take_redraw_request(),
        "display tick must not enqueue another redraw"
    );
}

#[test]
fn ios_redraw_refreshes_a_viewport_before_its_resize_event_is_delivered() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 1600),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    let next = PhysicalSize::new(1800, 1100);
    // UIKit has applied new bounds, but winit has not delivered Resized yet.
    app.window.update_headless_metrics(next, 2.0);
    WindowEventHandler::refresh_uikit_viewport(&mut app.app);
    assert_eq!(app.app.pending_resize, Some(next));
    app.render_frame();
    assert_eq!(app.physical_size(), next);
    assert_eq!(
        app.last_frame_result.as_ref().unwrap().1.target_size(),
        (1800, 1100)
    );
    // Ordinary frames must not repeatedly invalidate layout or enqueue resize.
    WindowEventHandler::refresh_uikit_viewport(&mut app.app);
    assert_eq!(app.app.pending_resize, None);
}

#[test]
fn ios_redraw_refreshes_scale_and_recovers_from_zero_bounds() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 1600),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    for (size, scale) in [
        (PhysicalSize::new(1200, 1600), 3.0),
        (PhysicalSize::new(0, 0), 3.0),
        (PhysicalSize::new(1800, 1200), 2.0),
    ] {
        app.window.update_headless_metrics(size, scale);
        WindowEventHandler::refresh_uikit_viewport(&mut app.app);
        assert_eq!(app.app.pending_resize, Some(size));
        app.render_frame();
        assert_eq!(app.physical_size(), size);
        assert_eq!(app.app.window_scale, scale);
    }
}

#[test]
fn ios_window_size_follows_resize_orientation() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1179, 2556),
            scale_factor: 3.0,
        },
    );
    for size in [PhysicalSize::new(1179, 2556), PhysicalSize::new(2556, 1179)] {
        app.send_window_event(WindowEvent::Resized(size));
        assert_eq!(app.physical_size(), size);
        assert_eq!(app.window.inner_size(), size);
        assert_eq!(
            app.logical_size(),
            ResolvedSize {
                width: size.width as f32 / 3.0,
                height: size.height as f32 / 3.0,
            }
        );
    }
}

#[test]
fn window_resize_tracks_current_viewport_instead_of_device_resolution() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        SizedBox::new().color(Color::RED),
        HeadlessOptions {
            size: PhysicalSize::new(2732, 2048),
            scale_factor: 2.0,
        },
    );
    // A window opened full screen can subsequently enter Split View or be
    // freely resized without the display's resolution changing.
    app.app.native_window_size = Some(ResolvedSize {
        width: 2732.0,
        height: 2048.0,
    });
    for size in [
        PhysicalSize::new(1200, 1600),
        PhysicalSize::new(1800, 1100),
        PhysicalSize::new(1100, 1100),
        PhysicalSize::new(2732, 2048),
    ] {
        app.send_window_event(WindowEvent::Resized(size));
        assert_eq!(app.app.pending_resize, None, "resize renders immediately");
        assert_eq!(
            app.app.native_window_size,
            Some(ResolvedSize {
                width: size.width as f32,
                height: size.height as f32,
            })
        );
        let metrics = WindowMetrics::of(&app.window);
        assert_eq!(metrics.physical_size, size);
        assert_eq!(
            metrics.logical_size(),
            ResolvedSize {
                width: size.width as f32 / 2.0,
                height: size.height as f32 / 2.0,
            }
        );
        let (_, packet) = direct_headless_frame_packet(&mut app);
        assert_eq!(
            (packet.frame().width, packet.frame().height),
            (size.width, size.height)
        );
        assert_eq!(
            app.last_frame_result.as_ref().unwrap().1.target_size(),
            (size.width, size.height)
        );
    }
}

#[test]
fn window_resize_recovers_after_zero_bounds_without_screen_fallback() {
    let mut app = AimerApp::start_headless_with(
        SizedBox::new(),
        HeadlessOptions {
            size: PhysicalSize::new(1200, 1600),
            scale_factor: 2.0,
        },
    );
    for size in [
        PhysicalSize::new(0, 0),
        PhysicalSize::new(0, 1600),
        PhysicalSize::new(1200, 0),
    ] {
        app.send_window_event(WindowEvent::Resized(size));
        assert_eq!(app.app.pending_resize, None);
        assert_eq!(app.physical_size(), size);
    }
    let restored = PhysicalSize::new(1400, 1000);
    app.send_window_event(WindowEvent::Resized(restored));
    assert_eq!(
        app.app.native_window_size,
        Some(ResolvedSize {
            width: 1400.0,
            height: 1000.0
        })
    );
    assert_eq!(
        app.logical_size(),
        ResolvedSize {
            width: 700.0,
            height: 500.0
        }
    );
}
