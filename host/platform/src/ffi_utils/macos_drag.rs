//! Where the cursor is during a macOS file drag.
//!
//! AppKit runs a drag session on its own event loop. While one is in flight the
//! window receives `draggingEntered` / `draggingUpdated` — which winit turns
//! into [`WindowEvent::HoveredFile`] — but no `mouseMoved` at all, so the last
//! cursor position the application saw is from before the drag began. A drop
//! zone hit-tested against that position lands wherever the user happened to
//! leave the mouse.
//!
//! [`NSEvent mouseLocation`] answers the question without a drag delegate, an
//! `NSView` subclass, or a patched winit: it is a *query*, not an event, and it
//! is correct at the instant it is asked — which is exactly the instant the file
//! event is being handled.
//!
//! [`WindowEvent::HoveredFile`]: winit::event::WindowEvent::HoveredFile
//! [`NSEvent mouseLocation`]: https://developer.apple.com/documentation/appkit/nsevent/1530060-mouselocation

use aimer_attribute::position::Vec2d;
use objc::runtime::{Class, Object};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// An AppKit rectangle, laid out as the Objective-C runtime returns it.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NSRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// An AppKit point.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NSPoint {
    x: f64,
    y: f64,
}

/// Converts a screen point and a window frame, both in AppKit's bottom-left
/// coordinate space, into a top-left window-local point.
///
/// Kept separate from the messaging so the arithmetic — the part that is easy
/// to get backwards — can be tested without a window.
fn window_local(screen: NSPoint, frame: NSRect) -> Vec2d {
    Vec2d {
        x: (screen.x - frame.x) as f32,
        y: (frame.y + frame.height - screen.y) as f32,
    }
}

/// Returns the cursor position inside `window`, in logical points measured from
/// the top-left of its content, or `None` if the cursor is elsewhere.
///
/// Returns `None` rather than guessing when the window handle is not an AppKit
/// one, when AppKit has no window object, or when the cursor is outside the
/// window — a caller that cannot be told where the drag is should fall back to
/// the last position it knows, not to a fabricated one.
#[allow(unexpected_cfgs)]
pub fn cursor_in_window(window: &Window) -> Option<Vec2d> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };

    // SAFETY: The AppKit handle comes from winit and this is called on winit's
    // main event-loop thread, where AppKit objects may be messaged. Every
    // message here is a plain accessor; nil messaging is harmless and is guarded
    // against below in any case.
    unsafe {
        let view = appkit.ns_view.as_ptr().cast::<Object>();
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return None;
        }

        let event_class: &Class = class!(NSEvent);
        let screen: NSPoint = msg_send![event_class, mouseLocation];
        let frame: NSRect = msg_send![ns_window, frame];

        // `frame` includes the title bar, which the content view does not, so
        // the position is measured against the content rectangle instead.
        let content: NSRect = msg_send![ns_window, contentLayoutRect];
        let content_screen = NSRect {
            x: frame.x + content.x,
            y: frame.y + content.y,
            width: content.width,
            height: content.height,
        };

        let local = window_local(screen, content_screen);
        let inside = local.x >= 0.0
            && local.y >= 0.0
            && f64::from(local.x) <= content_screen.width
            && f64::from(local.y) <= content_screen.height;

        inside.then_some(local)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AppKit measures from the bottom-left of the screen; the framework
    /// measures from the top-left of the window.
    #[test]
    fn a_screen_point_is_flipped_into_the_window() {
        let frame = NSRect {
            x: 100.0,
            y: 200.0,
            width: 800.0,
            height: 600.0,
        };

        // The window's top-left corner in screen space is (100, 800).
        let top_left = window_local(NSPoint { x: 100.0, y: 800.0 }, frame);
        assert_eq!(top_left.x, 0.0);
        assert_eq!(top_left.y, 0.0);

        // Its bottom-right corner is (900, 200).
        let bottom_right = window_local(NSPoint { x: 900.0, y: 200.0 }, frame);
        assert_eq!(bottom_right.x, 800.0);
        assert_eq!(bottom_right.y, 600.0);
    }
}
