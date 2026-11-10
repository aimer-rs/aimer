//! Blurring what is behind a native window.
//!
//! A window with a *backdrop* shows the desktop, and the windows behind it,
//! blurred through its own background, the way the Finder sidebar or a macOS
//! Spotlight panel does. The blur is drawn by the system compositor, not by the
//! application, so it costs the renderer nothing and follows whatever is moving
//! behind the window.
//!
//! [`set_window_material`] is the one entry point. It is available on every
//! platform and does nothing where the platform has no such effect.

use winit::window::Window;

/// How the system blurs what is behind a window.
///
/// The names are macOS's vocabulary. Windows has fewer looks, so several
/// materials share one: see the variants for the mapping.
///
/// # Examples
///
/// ```
/// use aimer_platform::window_backdrop::WindowMaterial;
///
/// assert_eq!(WindowMaterial::default(), WindowMaterial::Sidebar);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowMaterial {
    /// The material of a sidebar. Windows: Mica.
    #[default]
    Sidebar,
    /// The material of a window's titlebar. Windows: Mica.
    Titlebar,
    /// The material of a window's content background. Windows: Mica.
    Content,
    /// The material of a menu. Windows: Acrylic.
    Menu,
    /// The material of a popover. Windows: Acrylic.
    Popover,
    /// The dark, strongly blurred material of a heads-up display. Windows:
    /// Acrylic.
    Hud,
    /// The material shown under a window's content, behind its other
    /// materials. Windows: Mica Alt.
    Under,
    /// The refractive glass of macOS 26 and later (Liquid Glass), a
    /// `NSGlassEffectView`. It bends and lights what is behind it like a
    /// floating surface, so it suits a small panel, not a window-wide
    /// backdrop; for a sidebar like the Finder's use [`Sidebar`](Self::Sidebar).
    /// Older macOS: the sidebar blur. Windows: Acrylic.
    Glass,
    /// The same glass in its clear style: more transparent, with less of its
    /// own tint. Older macOS: the sidebar blur. Windows: Acrylic.
    ClearGlass,
}

impl WindowMaterial {
    /// The `DWM_SYSTEMBACKDROP_TYPE` Windows draws this material with.
    #[cfg(any(target_os = "windows", test))]
    pub(crate) const fn dwm_backdrop_type(self) -> u32 {
        match self {
            Self::Sidebar | Self::Titlebar | Self::Content => 2,
            Self::Menu | Self::Popover | Self::Hud | Self::Glass | Self::ClearGlass => 3,
            Self::Under => 4,
        }
    }
}

/// Gives `window` a blurred backdrop of `material`, or takes the backdrop away
/// with `None`.
///
/// The window is made see-through so the blur shows: everything the
/// application draws with some transparency lets it through, and what it draws
/// opaquely stays opaque. Taking the backdrop away restores the window to what
/// it was before it was given one.
///
/// Returns whether the platform took the request: `false` off the main thread,
/// for a window whose native handle is not the platform's own, and on a platform
/// without the effect. Setting the same material again is cheap and harmless.
///
/// Must be called on the main thread, which is where the event loop and so every
/// widget runs.
pub fn set_window_material(window: &Window, material: Option<WindowMaterial>) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::ffi_utils::macos_backdrop::apply(window, material)
    }
    #[cfg(target_os = "windows")]
    {
        crate::ffi_utils::windows_backdrop::apply(window, material)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (window, material);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::WindowMaterial;

    #[test]
    fn mica_materials_share_one_windows_backdrop() {
        let mica = WindowMaterial::Sidebar.dwm_backdrop_type();

        assert_eq!(mica, 2);
        assert_eq!(WindowMaterial::Titlebar.dwm_backdrop_type(), mica);
        assert_eq!(WindowMaterial::Content.dwm_backdrop_type(), mica);
    }

    #[test]
    fn floating_materials_use_acrylic_and_under_uses_mica_alt() {
        assert_eq!(WindowMaterial::Menu.dwm_backdrop_type(), 3);
        assert_eq!(WindowMaterial::Popover.dwm_backdrop_type(), 3);
        assert_eq!(WindowMaterial::Hud.dwm_backdrop_type(), 3);
        assert_eq!(WindowMaterial::Glass.dwm_backdrop_type(), 3);
        assert_eq!(WindowMaterial::ClearGlass.dwm_backdrop_type(), 3);
        assert_eq!(WindowMaterial::Under.dwm_backdrop_type(), 4);
    }
}
