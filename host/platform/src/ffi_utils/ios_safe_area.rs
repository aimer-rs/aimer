//! iOS' safe area, read off UIKit's view.
//!
//! The status bar, the notch and the home indicator sit *over* the
//! application's surface: content painted under them is visible but not
//! touchable, because a touch landing there belongs to the system. UIKit states
//! how much of each edge that costs as `UIView.safeAreaInsets`, and winit
//! exposes none of it — neither a query nor an event — so the view is asked
//! directly.
//!
//! There is nothing to observe: the insets only change when the view's geometry
//! does, and a rotation, a resize and a scale-factor change all reach the
//! application as window events already. So this is read from those, never
//! polled and never read per frame — one `objc_msgSend` on the events that can
//! possibly have moved the notch, and nothing at all in between.
//!
//! UIKit measures in points, which are the logical pixels
//! [`aimer_widget::SafeAreaInsets`] is defined in, so nothing is scaled on the
//! way through.

// `objc`'s `msg_send!` / `sel!` expand to a `cfg(cargo-clippy)` check that this
// crate does not declare.
#![allow(unexpected_cfgs)]

extern crate objc;

use objc::runtime::Object;
use objc::{Encode, Encoding, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// UIKit's `UIEdgeInsets`.
///
/// Laid out exactly as UIKit declares it — `top`, `left`, `bottom`, `right`, in
/// that order — so `objc_msgSend` can return it by value. `CGFloat` is `f64` on
/// every 64-bit Apple platform, which is every platform that has a safe area to
/// report.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct UIEdgeInsets {
    /// Points reserved along the top edge.
    top: f64,
    /// Points reserved along the left edge.
    left: f64,
    /// Points reserved along the bottom edge.
    bottom: f64,
    /// Points reserved along the right edge.
    right: f64,
}

// SAFETY: the encoding matches the declaration above — a struct named
// `UIEdgeInsets` of four `double`s — which is what `objc` needs to verify the
// return type of `safeAreaInsets` before handing the message to the runtime.
unsafe impl Encode for UIEdgeInsets {
    fn encode() -> Encoding {
        unsafe { Encoding::from_str("{UIEdgeInsets=dddd}") }
    }
}

/// Reports the region UIKit reserves in `window`, and asks for the one frame
/// that redraws it.
///
/// Called when the window appears and whenever its geometry moves; a
/// reservation that is already in effect asks for no frame, and neither does
/// one no widget follows — see [`aimer_widget::set_safe_area_insets`].
///
/// A window UIKit does not back, or one whose view has not been laid out yet,
/// reports nothing rather than zero: claiming the whole window is usable would
/// pull content under the status bar until the next resize.
///
/// # Examples
///
/// ```ignore
/// // A rotation moves the notch and the home indicator, so the new
/// // reservation is read from the resize the rotation arrives as.
/// fn on_resize(window: &std::sync::Arc<winit::window::Window>) {
///     report_safe_area(window);
/// }
/// ```
pub fn report_safe_area(window: &std::sync::Arc<Window>) {
    let Some(ui_view) = winit_ui_view(window) else {
        return;
    };
    let Some(insets) = safe_area_insets_of(ui_view) else {
        return;
    };
    let insets = crate::system_safe_area::from_ui_edge_insets(
        insets.top,
        insets.left,
        insets.bottom,
        insets.right,
    );
    if aimer_widget::safe_area::set_safe_area_insets_for(
        &aimer_widget::base::WindowHandle::native(window.clone()),
        insets,
    ) > 0
    {
        aimer_events::window::request_animation_frame();
    }
}

/// Recovers winit's `UIView` from the window handle.
fn winit_ui_view(window: &Window) -> Option<*mut Object> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::UiKit(uikit) = handle.as_raw() else {
        return None;
    };
    let ui_view = uikit.ui_view.as_ptr() as *mut Object;
    (!ui_view.is_null()).then_some(ui_view)
}

/// Reads `safeAreaInsets` off a `UIView`.
///
/// `None` on the iOS 10 and earlier that never had the property; every version
/// that can display a notch has it.
fn safe_area_insets_of(ui_view: *mut Object) -> Option<UIEdgeInsets> {
    let responds: bool = unsafe { msg_send![ui_view, respondsToSelector: sel!(safeAreaInsets)] };
    if !responds {
        return None;
    }
    Some(unsafe { msg_send![ui_view, safeAreaInsets] })
}
