//! The light or dark appearance the operating system currently asks for.
//!
//! [`aimer_widget::platform_brightness`] holds the answer and rebuilds the
//! widgets that follow it; this module is where the answer comes from. Every
//! platform has its own idea of where that lives, and only some of them tell
//! winit about it:
//!
//! | Platform            | Read from                                         | Change reported as                       |
//! |---------------------|---------------------------------------------------|------------------------------------------|
//! | macOS, Windows, X11, web | [`Window::theme`]                            | [`winit::event::WindowEvent::ThemeChanged`] |
//! | iOS                 | `UITraitCollection.userInterfaceStyle`             | a UIKit trait change (see `ffi_utils::ios_appearance`) |
//! | Android             | `ACONFIGURATION_UI_MODE_NIGHT_*`                   | a configuration change, which winit forwards as [`winit::event::WindowEvent::ScaleFactorChanged`] |
//!
//! winit answers `None` from [`Window::theme`] on both mobile platforms and
//! never emits `ThemeChanged` there, so each of them is asked directly. The
//! translation from the platform's raw value is kept here, apart from the FFI
//! that fetches it, because it is the part that can be stated exactly — and so
//! the part worth testing on any host.

use aimer_widget::Brightness;
use winit::window::Window;

/// `UIUserInterfaceStyleUnspecified`: UIKit has no answer, e.g. for a trait
/// collection that was never resolved against a window.
///
/// Named for what it rejects rather than matched on — every unknown value is
/// treated the same way.
#[allow(dead_code)]
const UI_USER_INTERFACE_STYLE_UNSPECIFIED: isize = 0;
/// `UIUserInterfaceStyleLight`.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const UI_USER_INTERFACE_STYLE_LIGHT: isize = 1;
/// `UIUserInterfaceStyleDark`.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
const UI_USER_INTERFACE_STYLE_DARK: isize = 2;

/// `ACONFIGURATION_UI_MODE_NIGHT_ANY`: the configuration does not pin a night
/// mode.
///
/// Named for what it rejects rather than matched on — every unknown value is
/// treated the same way.
#[allow(dead_code)]
const UI_MODE_NIGHT_ANY: i32 = 0x00;
/// `ACONFIGURATION_UI_MODE_NIGHT_NO`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const UI_MODE_NIGHT_NO: i32 = 0x01;
/// `ACONFIGURATION_UI_MODE_NIGHT_YES`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
const UI_MODE_NIGHT_DARK: i32 = 0x02;

/// Translates UIKit's `UIUserInterfaceStyle` into an appearance.
///
/// `None` means UIKit did not answer: the style is unspecified, or it is a
/// value this version of the framework does not know. A caller that gets
/// `None` must leave the current appearance alone rather than guess at light —
/// guessing would flip a correctly dark application to light.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub const fn from_user_interface_style(style: isize) -> Option<Brightness> {
    match style {
        UI_USER_INTERFACE_STYLE_LIGHT => Some(Brightness::Light),
        UI_USER_INTERFACE_STYLE_DARK => Some(Brightness::Dark),
        // `UIUserInterfaceStyleUnspecified` and anything newer.
        _ => None,
    }
}

/// Translates Android's `ACONFIGURATION_UI_MODE_NIGHT_*` into an appearance.
///
/// `None` means the configuration pins no night mode (`ANY`), or carries a
/// value this version of the framework does not know; as on iOS, the current
/// appearance is then left as it is.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub const fn from_night_mode(mode: i32) -> Option<Brightness> {
    match mode {
        UI_MODE_NIGHT_NO => Some(Brightness::Light),
        UI_MODE_NIGHT_DARK => Some(Brightness::Dark),
        // `ACONFIGURATION_UI_MODE_NIGHT_ANY` and anything newer.
        _ => None,
    }
}

/// Asks the platform which appearance it is in right now.
///
/// `None` from a platform that has no answer — a headless run, a Wayland
/// compositor without the appearance protocol, a trait collection UIKit has not
/// resolved yet — is not an appearance and must not be treated as one.
pub fn detect(window: &Window) -> Option<Brightness> {
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        crate::ffi_utils::ios_appearance::current_appearance()
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        crate::ffi_utils::android_appearance::current_appearance()
    }
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        window.theme().map(Brightness::from)
    }
}

/// Reports the appearance the platform is in, and answers whether a frame has
/// to be drawn for it.
///
/// Called once when the window appears and again whenever the platform hints
/// that its configuration moved. An appearance that is already in effect, or
/// one no widget follows, asks for no frame — see
/// [`aimer_widget::set_platform_brightness`].
pub fn announce(window: &Window) -> bool {
    match detect(window) {
        Some(brightness) => aimer_widget::set_platform_brightness(brightness) > 0,
        None => false,
    }
}

/// Subscribes to the platform's own appearance notifications, where winit does
/// not deliver them.
///
/// Only iOS needs this: macOS, Windows and X11 arrive as
/// [`winit::event::WindowEvent::ThemeChanged`], and Android arrives as a
/// configuration change. Called once, after the window exists.
pub fn start_observing(window: &Window) {
    #[cfg(target_os = "ios")]
    crate::ffi_utils::ios_appearance::observe_appearance_changes(window);
    #[cfg(not(target_os = "ios"))]
    let _ = window;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uikit_light_and_dark_styles_are_the_two_appearances() {
        assert_eq!(
            from_user_interface_style(UI_USER_INTERFACE_STYLE_LIGHT),
            Some(Brightness::Light)
        );
        assert_eq!(
            from_user_interface_style(UI_USER_INTERFACE_STYLE_DARK),
            Some(Brightness::Dark)
        );
    }

    #[test]
    fn an_unspecified_uikit_style_is_no_answer() {
        assert_eq!(
            from_user_interface_style(UI_USER_INTERFACE_STYLE_UNSPECIFIED),
            None
        );
    }

    #[test]
    fn a_uikit_style_from_a_newer_ios_is_no_answer() {
        assert_eq!(from_user_interface_style(7), None);
        assert_eq!(from_user_interface_style(-1), None);
    }

    #[test]
    fn android_night_mode_no_and_yes_are_the_two_appearances() {
        assert_eq!(from_night_mode(UI_MODE_NIGHT_NO), Some(Brightness::Light));
        assert_eq!(from_night_mode(UI_MODE_NIGHT_DARK), Some(Brightness::Dark));
    }

    #[test]
    fn an_unpinned_android_night_mode_is_no_answer() {
        assert_eq!(from_night_mode(UI_MODE_NIGHT_ANY), None);
    }

    #[test]
    fn an_android_night_mode_from_a_newer_platform_is_no_answer() {
        assert_eq!(from_night_mode(0x03), None);
    }
}
