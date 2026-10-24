//! Gestures the windowing system recognizes before the application sees any
//! contact: the trackpad's pinch, twist and smart zoom, and the emulation of a
//! pinch for platforms that have none.
//!
//! This module only *translates*. winit reports a magnification in `f64`, a
//! rotation in counterclockwise degrees and a pan in physical pixels; widgets
//! want a finite `f32` fraction, clockwise radians and logical pixels.
//! Dispatching the result is [`WindowEventHandler`]'s business.
//!
//! [`WindowEventHandler`]: crate::handler::event_handler::WindowEventHandler

// macOS and iOS receive real pinch events and never emulate one from the wheel.
#![cfg_attr(any(target_os = "macos", target_os = "ios"), allow(dead_code))]

use aimer_attribute::position::Vec2d;
use aimer_events::element::{ElementEvent, ScrollDeltaKind};
use winit::dpi::PhysicalPosition;
use winit::event::TouchPhase;

/// The largest zoom step a single emulated wheel event may make, as a fraction.
///
/// A free-spinning wheel or a fast trackpad flick can report hundreds of pixels
/// in one event; unclamped, that would zoom the content several times over in a
/// frame.
const MAX_WHEEL_ZOOM_STEP: f32 = 0.5;

/// How much one normalized wheel line zooms by. A notch is normalized to 20, so
/// a notch zooms by 10 %.
const LINE_ZOOM_PER_UNIT: f32 = 0.005;

/// How much one normalized pixel of wheel travel zooms by: 1 % per raw pixel,
/// undoing the scroll multiplier the delta was normalized with.
const PIXEL_ZOOM_PER_UNIT: f32 = 0.01 / crate::handler::event_handler::WindowEventHandler::SCROLL_MULTIPLIER;

/// `value` if it is a number, else `0.0`: winit documents the pinch delta as
/// possibly `NaN`, and a widget accumulating it would be poisoned for good.
#[inline]
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// A trackpad pinch at `pos`; winit's `delta` is already a fractional change.
#[inline]
pub(crate) fn pinch(pos: Vec2d, delta: f64, phase: TouchPhase) -> ElementEvent {
    ElementEvent::Pinch {
        pos,
        delta: finite(delta as f32),
        phase,
    }
}

/// A trackpad rotation at `pos`.
///
/// winit reports degrees, counterclockwise positive. Aimer measures angles in
/// radians, clockwise positive — the direction an angle grows in a y-down
/// coordinate system, and the one a touch rotation reports — so the sign flips.
#[inline]
pub(crate) fn rotation(pos: Vec2d, degrees_counterclockwise: f32, phase: TouchPhase) -> ElementEvent {
    ElementEvent::Rotate {
        pos,
        delta: finite(-degrees_counterclockwise.to_radians()),
        phase,
    }
}

/// A multi-finger pan at `pos`; winit reports physical pixels.
#[inline]
pub(crate) fn pan(
    pos: Vec2d,
    delta: PhysicalPosition<f32>,
    window_scale: f64,
    phase: TouchPhase,
) -> ElementEvent {
    let scale = if window_scale.is_finite() && window_scale > 0.0 {
        window_scale as f32
    } else {
        1.0
    };
    ElementEvent::Pan {
        pos,
        delta: Vec2d {
            x: finite(delta.x / scale),
            y: finite(delta.y / scale),
        },
        phase,
    }
}

/// The smart-zoom gesture at `pos`.
#[inline]
pub(crate) fn smart_zoom(pos: Vec2d) -> ElementEvent {
    ElementEvent::SmartZoom { pos }
}

/// The fractional zoom step a `ctrl` + wheel event stands for, or `0.0` if it
/// carries no vertical travel.
///
/// Windows and the browsers have no pinch event: they turn a precision
/// touchpad's pinch into a wheel event with `ctrl` held, which is also what
/// `ctrl` + scroll has always meant on a mouse. Scrolling up, away from the
/// user, zooms in.
///
/// `delta` is the normalized wheel delta, before any platform scroll
/// multiplier.
pub(crate) fn wheel_zoom_step(delta: Vec2d, kind: ScrollDeltaKind) -> f32 {
    let per_unit = match kind {
        ScrollDeltaKind::Line => LINE_ZOOM_PER_UNIT,
        ScrollDeltaKind::Pixel => PIXEL_ZOOM_PER_UNIT,
    };
    (finite(delta.y) * per_unit).clamp(-MAX_WHEEL_ZOOM_STEP, MAX_WHEEL_ZOOM_STEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Vec2d = Vec2d { x: 7.0, y: 9.0 };

    #[test]
    fn a_pinch_keeps_its_fraction_and_phase() {
        assert!(matches!(
            pinch(AT, 0.25, TouchPhase::Moved),
            ElementEvent::Pinch { pos, delta, phase: TouchPhase::Moved }
                if pos == AT && delta == 0.25
        ));
    }

    #[test]
    fn a_non_numeric_pinch_becomes_no_movement() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
            assert!(matches!(
                pinch(AT, bad, TouchPhase::Moved),
                ElementEvent::Pinch { delta, .. } if delta == 0.0
            ), "{bad} must not reach a widget");
        }
    }

    // winit turns counterclockwise positive; a touch rotation, and every y-down
    // angle in the framework, turns clockwise positive.
    #[test]
    fn a_counterclockwise_trackpad_twist_becomes_a_negative_rotation() {
        let ElementEvent::Rotate { delta, .. } = rotation(AT, 90.0, TouchPhase::Moved) else {
            panic!("expected a rotation");
        };

        assert!((delta + std::f32::consts::FRAC_PI_2).abs() < 1e-6, "{delta}");
    }

    #[test]
    fn a_clockwise_trackpad_twist_becomes_a_positive_rotation() {
        let ElementEvent::Rotate { delta, .. } = rotation(AT, -45.0, TouchPhase::Moved) else {
            panic!("expected a rotation");
        };

        assert!((delta - std::f32::consts::FRAC_PI_4).abs() < 1e-6, "{delta}");
    }

    #[test]
    fn a_non_numeric_rotation_becomes_no_turn() {
        assert!(matches!(
            rotation(AT, f32::NAN, TouchPhase::Moved),
            ElementEvent::Rotate { delta, .. } if delta == 0.0
        ));
    }

    #[test]
    fn a_pan_is_converted_from_physical_to_logical_pixels() {
        let ElementEvent::Pan { delta, .. } =
            pan(AT, PhysicalPosition::new(30.0, -10.0), 2.0, TouchPhase::Moved)
        else {
            panic!("expected a pan");
        };

        assert_eq!(delta, Vec2d { x: 15.0, y: -5.0 });
    }

    #[test]
    fn a_pan_with_a_broken_scale_factor_is_left_unscaled() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let ElementEvent::Pan { delta, .. } =
                pan(AT, PhysicalPosition::new(4.0, 6.0), scale, TouchPhase::Moved)
            else {
                panic!("expected a pan");
            };

            assert_eq!(delta, Vec2d { x: 4.0, y: 6.0 }, "scale {scale}");
        }
    }

    #[test]
    fn a_non_numeric_pan_becomes_no_movement() {
        let ElementEvent::Pan { delta, .. } = pan(
            AT,
            PhysicalPosition::new(f32::NAN, f32::INFINITY),
            1.0,
            TouchPhase::Moved,
        ) else {
            panic!("expected a pan");
        };

        assert_eq!(delta, Vec2d::ZERO);
    }

    #[test]
    fn smart_zoom_carries_the_cursor() {
        assert!(matches!(smart_zoom(AT), ElementEvent::SmartZoom { pos } if pos == AT));
    }

    #[test]
    fn one_mouse_wheel_notch_zooms_ten_percent() {
        // A notch is normalized to 20 units.
        let step = wheel_zoom_step(Vec2d { x: 0.0, y: 20.0 }, ScrollDeltaKind::Line);

        assert!((step - 0.1).abs() < 1e-6, "{step}");
    }

    #[test]
    fn scrolling_down_zooms_out() {
        let step = wheel_zoom_step(Vec2d { x: 0.0, y: -20.0 }, ScrollDeltaKind::Line);

        assert!(step < 0.0);
    }

    // The pixel delta arrives multiplied by the scroll multiplier; the zoom must
    // not inherit it, or a pinch would zoom 50 % faster than a notch of the same
    // travel.
    #[test]
    fn a_pixel_delta_zooms_one_percent_per_raw_pixel() {
        let multiplier = crate::handler::event_handler::WindowEventHandler::SCROLL_MULTIPLIER;
        let step = wheel_zoom_step(Vec2d { x: 0.0, y: 10.0 * multiplier }, ScrollDeltaKind::Pixel);

        assert!((step - 0.1).abs() < 1e-6, "{step}");
    }

    #[test]
    fn horizontal_travel_alone_does_not_zoom() {
        assert_eq!(
            wheel_zoom_step(Vec2d { x: 80.0, y: 0.0 }, ScrollDeltaKind::Pixel),
            0.0
        );
    }

    #[test]
    fn a_runaway_wheel_event_is_clamped() {
        let step = wheel_zoom_step(Vec2d { x: 0.0, y: 100_000.0 }, ScrollDeltaKind::Pixel);
        let shrink = wheel_zoom_step(Vec2d { x: 0.0, y: -100_000.0 }, ScrollDeltaKind::Line);

        assert_eq!(step, MAX_WHEEL_ZOOM_STEP);
        assert_eq!(shrink, -MAX_WHEEL_ZOOM_STEP);
    }

    #[test]
    fn a_non_numeric_wheel_event_does_not_zoom() {
        assert_eq!(
            wheel_zoom_step(Vec2d { x: 0.0, y: f32::NAN }, ScrollDeltaKind::Pixel),
            0.0
        );
    }
}
