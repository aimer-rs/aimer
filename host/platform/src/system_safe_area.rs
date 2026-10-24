//! The region of the window the platform keeps for itself.
//!
//! [`aimer_widget::safe_area`] holds the answer and rebuilds the widgets that
//! follow it; this module is where the answer comes from. Only two platforms
//! reserve anything, and neither of them tells winit about it:
//!
//! | Platform | Read from | Change reported as |
//! |----------|-----------|--------------------|
//! | iOS      | `UIView.safeAreaInsets` | a resize or a scale-factor change — a rotation moves the notch and the home indicator (see `ffi_utils::ios_safe_area`) |
//! | web      | the `env(safe-area-inset-*)` CSS variables | `resize` / `orientationchange` on the browser window (see `ffi_utils::web_safe_area`) |
//! | others   | nothing — the whole window is usable | never |
//!
//! Everywhere else this compiles to nothing at all: [`announce`] has an empty
//! body, so a desktop build carries no query, no branch and no allocation for a
//! reservation that is always zero.
//!
//! The translation from each platform's raw numbers is kept here, apart from
//! the FFI that fetches them, because it is the part that can be stated exactly
//! — and so the part worth testing on any host.

use aimer_widget::SafeAreaInsets;
use winit::window::Window;

/// Translates UIKit's `UIEdgeInsets` into safe-area insets.
///
/// UIKit measures in points, which are already the logical pixels
/// [`SafeAreaInsets`] is defined in, so nothing is scaled. The argument order
/// is `UIEdgeInsets`' own — `top`, `left`, `bottom`, `right` — and is
/// deliberately not reordered on the way in: the struct is read straight off
/// the runtime and any reshuffling belongs in one place, which is this
/// function.
///
/// A platform that answers a negative or a non-finite point reserves nothing,
/// courtesy of [`SafeAreaInsets::new`].
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub fn from_ui_edge_insets(top: f64, left: f64, bottom: f64, right: f64) -> SafeAreaInsets {
    SafeAreaInsets::new(left as f32, top as f32, right as f32, bottom as f32)
}

/// Reads a CSS length the browser resolved for us, in pixels.
///
/// `getComputedStyle` answers a *used* value, which for a padding is always an
/// absolute `<length>` in `px` — `"34px"`, or `"0px"` where the environment
/// variable is undefined. An empty string, a unit this does not expect, or
/// anything unparseable is no reservation rather than a guess: mis-reading a
/// length here would push content off the screen.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn from_css_pixels(value: &str) -> f32 {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value);
    match number.trim().parse::<f32>() {
        Ok(pixels) if pixels.is_finite() && pixels > 0.0 => pixels,
        _ => 0.0,
    }
}

/// Reports the region the platform reserves in the window right now.
///
/// Called once when the window appears and again whenever the window's shape
/// moves — a resize, a rotation, a scale-factor change — because that is when
/// and only when the reservation can differ. Insets that are already in effect
/// cost nothing beyond the query, and a frame is asked for only when a widget
/// actually follows them; see [`aimer_widget::set_safe_area_insets`].
pub fn announce(window: &std::sync::Arc<Window>) {
    #[cfg(target_os = "ios")]
    crate::ffi_utils::ios_safe_area::report_safe_area(window);
    #[cfg(target_arch = "wasm32")]
    crate::ffi_utils::web_safe_area::report_safe_area();
    #[cfg(not(target_os = "ios"))]
    let _ = window;
}

/// Subscribes to the platform's own safe-area notifications, where the window
/// events do not carry them.
///
/// Only the web needs this: a browser resizes its viewport without winit
/// hearing about the environment variables that came with it, so `resize` and
/// `orientationchange` are listened to directly. Called once, after the window
/// exists; repeated calls are ignored.
pub fn start_observing(window: &Window) {
    #[cfg(target_arch = "wasm32")]
    crate::ffi_utils::web_safe_area::observe_safe_area_changes();
    let _ = window;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uikit_edge_insets_keep_their_edges() {
        // An iPhone in portrait: status bar on top, home indicator below.
        let insets = from_ui_edge_insets(59.0, 0.0, 34.0, 0.0);

        assert_eq!(insets.top, 59.0);
        assert_eq!(insets.bottom, 34.0);
        assert_eq!(insets.left, 0.0);
        assert_eq!(insets.right, 0.0);
    }

    #[test]
    fn uikit_edge_insets_are_not_transposed_in_landscape() {
        // Landscape left: the notch is on one side and the indicator below.
        let insets = from_ui_edge_insets(0.0, 59.0, 21.0, 0.0);

        assert_eq!(insets.left, 59.0);
        assert_eq!(insets.bottom, 21.0);
        assert_eq!(insets.top, 0.0);
        assert_eq!(insets.right, 0.0);
    }

    #[test]
    fn nonsense_from_uikit_reserves_nothing() {
        let insets = from_ui_edge_insets(f64::NAN, -8.0, f64::INFINITY, 0.0);

        assert_eq!(insets, SafeAreaInsets::ZERO);
    }

    #[test]
    fn a_computed_padding_is_a_pixel_length() {
        assert_eq!(from_css_pixels("34px"), 34.0);
        assert_eq!(from_css_pixels(" 47.5px "), 47.5);
    }

    #[test]
    fn an_undefined_environment_variable_reserves_nothing() {
        assert_eq!(from_css_pixels("0px"), 0.0);
        assert_eq!(from_css_pixels(""), 0.0);
    }

    #[test]
    fn a_length_the_browser_states_in_another_unit_reserves_nothing() {
        assert_eq!(from_css_pixels("2em"), 0.0);
        assert_eq!(from_css_pixels("auto"), 0.0);
        assert_eq!(from_css_pixels("-4px"), 0.0);
    }
}
