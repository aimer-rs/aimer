use super::*;

#[test]
fn scale_factor_change_updates_window_scale() {
    let mut window_scale = 1.0;

    WindowEventHandler::update_scale_factor(&mut window_scale, 2.0);

    assert_eq!(window_scale, 2.0);
}

#[test]
fn event_result_redraws_for_legacy_handling_or_explicit_request() {
    assert!(WindowEventHandler::should_redraw(
        aimer_widget::EventResult::ignored(),
        true,
    ));
    assert!(WindowEventHandler::should_redraw(
        aimer_widget::EventResult::redraw(),
        false,
    ));
    assert!(!WindowEventHandler::should_redraw(
        aimer_widget::EventResult::ignored(),
        false,
    ));
}

#[test]
fn precise_wheel_delta_preserves_logical_distance() {
    let (delta, kind) = WindowEventHandler::normalize_wheel_delta(
        MouseScrollDelta::PixelDelta(PhysicalPosition::new(24.0, -16.0)),
        2.0,
    );

    assert_eq!(delta.x, 18.0);
    assert_eq!(delta.y, -12.0);
    assert_eq!(kind, aimer_events::element::ScrollDeltaKind::Pixel);
}

#[test]
fn line_wheel_delta_uses_stable_logical_step() {
    let (delta, kind) =
        WindowEventHandler::normalize_wheel_delta(MouseScrollDelta::LineDelta(1.0, -2.0), 2.0);

    assert_eq!(delta.x, 20.0);
    assert_eq!(delta.y, -40.0);
    assert_eq!(kind, aimer_events::element::ScrollDeltaKind::Line);
}

#[test]
fn precise_wheel_delta_uses_safe_scale_fallback() {
    for scale in [0.0, f64::NAN, f64::INFINITY] {
        let (delta, _) = WindowEventHandler::normalize_wheel_delta(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(12.0, -8.0)),
            scale,
        );

        assert_eq!(delta.x, 12.0 * WindowEventHandler::SCROLL_MULTIPLIER);
        assert_eq!(delta.y, -8.0 * WindowEventHandler::SCROLL_MULTIPLIER);
    }
}
