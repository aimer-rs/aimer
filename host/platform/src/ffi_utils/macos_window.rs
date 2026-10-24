//! AppKit window behaviour that has to be set per window.

use objc::runtime::Object;
use objc::{msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// `NSWindowAnimationBehaviorNone`.
const ANIMATION_BEHAVIOR_NONE: isize = 2;

/// Makes `window` disappear without the system's close animation.
///
/// Meant for a window that is about to be hidden and destroyed. AppKit runs
/// that animation in a run loop on a background thread, and a window that is
/// destroyed while it plays leaves the thread parked forever: one thread, and
/// the memory behind its stack, for every window ever closed. A window that
/// skips the animation leaves nothing behind. The animation that plays when a
/// window opens is not affected.
///
/// Returns whether the setting was applied. Must be called on the main thread,
/// which is where the event loop and so every window operation runs.
#[allow(unexpected_cfgs)]
pub fn skip_close_animation(window: &Window) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return false;
    };
    // SAFETY: the view belongs to a live winit window, and this runs on the
    // main thread, where AppKit objects may be messaged. The window is checked
    // for null before it is messaged.
    unsafe {
        let view = appkit.ns_view.as_ptr().cast::<Object>();
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return false;
        }
        let _: () = msg_send![ns_window, setAnimationBehavior: ANIMATION_BEHAVIOR_NONE];
    }
    true
}
