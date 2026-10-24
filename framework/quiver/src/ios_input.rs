//! UIKit input ABI. Pointer coordinates and scroll deltas are window-local points.

use aimer_attribute::position::Vec2d;
use aimer_events::element::{
    ElementEvent, KeyAction, Modifiers, NamedKey, ScrollDeltaKind, TouchPhase,
};
use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};

fn modifiers(flags: u64) -> Modifiers {
    Modifiers {
        shift: flags & (1 << 17) != 0,
        ctrl: flags & (1 << 18) != 0,
        alt: flags & (1 << 19) != 0,
        meta: flags & (1 << 20) != 0,
    }
}

fn key(usage: u32, text: &str) -> Option<NamedKey> {
    Some(match usage {
        40 | 88 => NamedKey::Enter,
        41 => NamedKey::Escape,
        42 => NamedKey::Backspace,
        43 => NamedKey::Tab,
        73 => NamedKey::Other("Insert".into()),
        74 => NamedKey::Home,
        75 => NamedKey::PageUp,
        76 => NamedKey::Delete,
        77 => NamedKey::End,
        78 => NamedKey::PageDown,
        79 => NamedKey::ArrowRight,
        80 => NamedKey::ArrowLeft,
        81 => NamedKey::ArrowDown,
        82 => NamedKey::ArrowUp,
        58..=69 => NamedKey::Other(format!("F{}", usage - 57)),
        224 | 228 => NamedKey::Other("Control".into()),
        225 | 229 => NamedKey::Other("Shift".into()),
        226 | 230 => NamedKey::Other("Alt".into()),
        227 | 231 => NamedKey::Other("Meta".into()),
        _ if !text.is_empty() && !text.chars().any(char::is_control) => {
            NamedKey::Other(text.into())
        }
        _ => return None,
    })
}

fn action(value: u32) -> Option<KeyAction> {
    match value {
        0 => Some(KeyAction::Pressed),
        1 => Some(KeyAction::Released),
        2 => Some(KeyAction::Repeat),
        _ => None,
    }
}

fn point(x: f64, y: f64) -> Option<Vec2d> {
    let point = Vec2d {
        x: x as f32,
        y: y as f32,
    };
    (point.x.is_finite() && point.y.is_finite()).then_some(point)
}

fn pointer_event(kind: u32, x: f64, y: f64, button: u32) -> Option<ElementEvent> {
    if kind == 1 {
        return Some(ElementEvent::PointerExited(PointerSource::Mouse, 0));
    }
    if kind == 4 {
        return Some(ElementEvent::Cancel);
    }
    let pos = point(x, y)?;
    let button = match button {
        0 => PointerButton::Primary,
        1 => PointerButton::Secondary,
        2 => PointerButton::Middle,
        other => PointerButton::Other(other.try_into().ok()?),
    };
    let pointer = PointerInfo::mouse(pos, button);
    match kind {
        0 => Some(ElementEvent::PointerMove(pointer)),
        2 => Some(ElementEvent::PointerDown(pointer)),
        3 => Some(ElementEvent::PointerUp(pointer)),
        _ => None,
    }
}

/// began/moved/ended/cancelled as UIKit gesture recognizers encode them: 0/1/2/3.
fn gesture_phase(phase: u32) -> Option<TouchPhase> {
    match phase {
        0 => Some(TouchPhase::Started),
        1 => Some(TouchPhase::Moved),
        2 => Some(TouchPhase::Ended),
        3 => Some(TouchPhase::Cancelled),
        _ => None,
    }
}

fn scroll_event(x: f64, y: f64, phase: u32) -> Option<ElementEvent> {
    let phase = gesture_phase(phase)?;
    Some(ElementEvent::Scroll {
        delta: point(x, y)?,
        phase,
        kind: ScrollDeltaKind::Pixel,
        is_direct_manipulation: phase == TouchPhase::Started || phase == TouchPhase::Moved,
    })
}

/// A finite `f32`, or `None` for anything that would poison a widget's
/// accumulated transform.
fn finite_delta(value: f64) -> Option<f32> {
    let value = value as f32;
    value.is_finite().then_some(value)
}

fn pinch_event(x: f64, y: f64, delta: f64, phase: u32) -> Option<ElementEvent> {
    Some(ElementEvent::Pinch {
        pos: point(x, y)?,
        delta: finite_delta(delta)?,
        phase: gesture_phase(phase)?,
    })
}

fn rotation_event(x: f64, y: f64, delta: f64, phase: u32) -> Option<ElementEvent> {
    Some(ElementEvent::Rotate {
        pos: point(x, y)?,
        delta: finite_delta(delta)?,
        phase: gesture_phase(phase)?,
    })
}

fn pan_event(x: f64, y: f64, delta_x: f64, delta_y: f64, phase: u32) -> Option<ElementEvent> {
    Some(ElementEvent::Pan {
        pos: point(x, y)?,
        delta: Vec2d {
            x: finite_delta(delta_x)?,
            y: finite_delta(delta_y)?,
        },
        phase: gesture_phase(phase)?,
    })
}

fn smart_zoom_event(x: f64, y: f64) -> Option<ElementEvent> {
    Some(ElementEvent::SmartZoom { pos: point(x, y)? })
}

#[cfg(target_os = "ios")]
fn send(event: ElementEvent, flags: u64) {
    if let Some(proxy) = crate::aimer_app::EVENT_PROXY.get() {
        let _ = proxy.send_event(crate::aimer_app::AimerNativePlatformEvent::NativeInput {
            event,
            modifiers: modifiers(flags),
        });
    }
}

/// Hands the app a link UIKit opened it with: a custom-scheme URL or a universal
/// link. The link is queued and delivered on the event loop's own thread, so this
/// may be called at any time, including before the loop is running.
///
/// # Safety
///
/// `ptr` must reference `len` readable, initialized bytes for this call. A null
/// pointer is allowed only for zero length. Invalid UTF-8 is rejected.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimer_ios_open_url(ptr: *const u8, len: usize) {
    if len == 0 || len > isize::MAX as usize || ptr.is_null() {
        return;
    }
    // SAFETY: The caller owns the readable bytes throughout this call; bounds
    // above satisfy slice length and non-null requirements.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Ok(url) = std::str::from_utf8(bytes) {
        crate::link_inbox::post_link(url.to_owned());
    }
}

/// Forwards a UIKit mouse move (0), exit (1), down (2), up (3), or cancellation (4).
/// Coordinates are logical window points; button numbers are zero based.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_pointer_event(kind: u32, x: f64, y: f64, button: u32, flags: u64) {
    if let Some(event) = pointer_event(kind, x, y, button) {
        send(event, flags);
    }
}

/// Forwards a UIKit scroll in logical points, with began/moved/ended/cancelled
/// encoded as 0/1/2/3. UIKit has already applied the user's scroll direction.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_scroll_event(x: f64, y: f64, phase: u32, flags: u64) {
    if let Some(event) = scroll_event(x, y, phase) {
        send(event, flags);
    }
}

/// Forwards a UIKit pinch at (`x`, `y`) in logical points. `delta` is the
/// fractional change in scale since the previous callback — `0.1` grows the
/// content by 10 % — not the recognizer's cumulative `scale`. Phases are
/// 0/1/2/3 for began/changed/ended/cancelled.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_pinch_event(x: f64, y: f64, delta: f64, phase: u32, flags: u64) {
    if let Some(event) = pinch_event(x, y, delta, phase) {
        send(event, flags);
    }
}

/// Forwards a UIKit rotation at (`x`, `y`) in logical points. `delta` is the
/// change since the previous callback in radians, clockwise positive — which is
/// UIKit's own convention for `UIRotationGestureRecognizer.rotation`.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_rotation_event(x: f64, y: f64, delta: f64, phase: u32, flags: u64) {
    if let Some(event) = rotation_event(x, y, delta, phase) {
        send(event, flags);
    }
}

/// Forwards a UIKit multi-finger pan anchored at (`x`, `y`). The deltas are the
/// movement since the previous callback in logical points.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_pan_event(
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
    phase: u32,
    flags: u64,
) {
    if let Some(event) = pan_event(x, y, delta_x, delta_y, phase) {
        send(event, flags);
    }
}

/// Forwards a smart-zoom double tap at (`x`, `y`) in logical points.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn aimer_ios_smart_zoom_event(x: f64, y: f64, flags: u64) {
    if let Some(event) = smart_zoom_event(x, y) {
        send(event, flags);
    }
}

/// Forwards a hardware key independently of committed text and IME edits.
/// `usage` is a USB keyboard HID usage; action is down/up/repeat (0/1/2).
///
/// # Safety
/// `ptr` must reference `len` readable, initialized bytes for this call. A null
/// pointer is allowed only for zero length. Invalid UTF-8 is rejected.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimer_ios_key_event(
    usage: u32,
    state: u32,
    flags: u64,
    ptr: *const u8,
    len: usize,
) {
    let Some(action) = action(state) else { return };
    if len > isize::MAX as usize || (len != 0 && ptr.is_null()) {
        return;
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        // SAFETY: The caller owns the readable bytes throughout this call;
        // bounds above satisfy slice length and non-null requirements.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Some(mut key) = key(usage, text) else {
        return;
    };
    let modifiers = modifiers(flags);
    if (modifiers.ctrl || modifiers.meta)
        && let NamedKey::Other(text) = &mut key
    {
        text.make_ascii_lowercase();
    }
    send(
        ElementEvent::KeyInput {
            key,
            action,
            modifiers,
        },
        flags,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uikit_modifier_bits_are_not_winit_modifier_bits() {
        assert_eq!(
            modifiers((1 << 17) | (1 << 18) | (1 << 19) | (1 << 20)),
            Modifiers {
                shift: true,
                ctrl: true,
                alt: true,
                meta: true
            }
        );
        assert_eq!(
            modifiers(1 << 16),
            Modifiers::default(),
            "caps lock is not a shortcut modifier"
        );
    }

    #[test]
    fn hardware_keys_include_navigation_editing_and_layout_characters() {
        for (usage, expected) in [
            (40, NamedKey::Enter),
            (41, NamedKey::Escape),
            (42, NamedKey::Backspace),
            (43, NamedKey::Tab),
            (73, NamedKey::Other("Insert".into())),
            (74, NamedKey::Home),
            (75, NamedKey::PageUp),
            (76, NamedKey::Delete),
            (77, NamedKey::End),
            (78, NamedKey::PageDown),
            (79, NamedKey::ArrowRight),
            (80, NamedKey::ArrowLeft),
            (81, NamedKey::ArrowDown),
            (82, NamedKey::ArrowUp),
        ] {
            assert_eq!(key(usage, ""), Some(expected));
        }
        assert_eq!(key(4, "ф"), Some(NamedKey::Other("ф".into())));
        assert_eq!(key(44, " "), Some(NamedKey::Other(" ".into())));
        assert_eq!(key(0, ""), None);
        assert_eq!(action(0), Some(KeyAction::Pressed));
        assert_eq!(action(1), Some(KeyAction::Released));
        assert_eq!(action(2), Some(KeyAction::Repeat));
        assert_eq!(action(3), None);
    }

    #[test]
    fn malformed_pointer_and_scroll_payloads_are_ignored() {
        assert!(pointer_event(0, f64::NAN, 0.0, 0).is_none());
        assert!(pointer_event(2, 0.0, f64::MAX, 0).is_none());
        assert!(pointer_event(2, 0.0, 0.0, u32::MAX).is_none());
        assert!(pointer_event(5, 0.0, 0.0, 0).is_none());
        assert!(scroll_event(0.0, 1.0, 4).is_none());
        assert!(scroll_event(f64::INFINITY, 0.0, 1).is_none());
        assert!(
            matches!(pointer_event(2, -1.0, 2.0, 2), Some(ElementEvent::PointerDown(p))
            if p.button == PointerButton::Middle && p.pos.x == -1.0)
        );
        assert!(matches!(
            scroll_event(0.0, -12.0, 2),
            Some(ElementEvent::Scroll {
                phase: TouchPhase::Ended,
                ..
            })
        ));
    }

    #[test]
    fn uikit_gestures_keep_their_deltas_and_phases() {
        assert!(matches!(
            pinch_event(10.0, 20.0, 0.25, 1),
            Some(ElementEvent::Pinch { pos, delta, phase: TouchPhase::Moved })
                if pos == (Vec2d { x: 10.0, y: 20.0 }) && delta == 0.25
        ));
        assert!(matches!(
            rotation_event(1.0, 2.0, -0.5, 0),
            Some(ElementEvent::Rotate { delta, phase: TouchPhase::Started, .. }) if delta == -0.5
        ));
        assert!(matches!(
            pan_event(1.0, 2.0, 3.0, -4.0, 2),
            Some(ElementEvent::Pan { delta, phase: TouchPhase::Ended, .. })
                if delta == (Vec2d { x: 3.0, y: -4.0 })
        ));
        assert!(matches!(
            smart_zoom_event(5.0, 6.0),
            Some(ElementEvent::SmartZoom { pos }) if pos == (Vec2d { x: 5.0, y: 6.0 })
        ));
    }

    #[test]
    fn malformed_gesture_payloads_are_ignored() {
        assert!(pinch_event(0.0, 0.0, f64::NAN, 1).is_none());
        assert!(pinch_event(0.0, 0.0, f64::INFINITY, 1).is_none());
        assert!(pinch_event(f64::NAN, 0.0, 0.1, 1).is_none());
        assert!(pinch_event(0.0, 0.0, 0.1, 4).is_none());
        assert!(rotation_event(0.0, 0.0, f64::NAN, 1).is_none());
        assert!(rotation_event(0.0, 0.0, 0.1, 9).is_none());
        assert!(pan_event(0.0, 0.0, f64::NAN, 0.0, 1).is_none());
        assert!(pan_event(0.0, 0.0, 0.0, f64::INFINITY, 1).is_none());
        assert!(pan_event(0.0, 0.0, 0.0, 0.0, 4).is_none());
        assert!(smart_zoom_event(f64::MAX, 0.0).is_none());
    }
}
