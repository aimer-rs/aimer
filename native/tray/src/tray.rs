//! The builder and the handle of a tray icon.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::backend::{self, Backend};
use crate::{TrayEvent, TrayHandler, TrayIcon, TrayMenu};

/// Why a tray icon could not be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayError {
    /// [`Tray::build`] was called without an icon.
    MissingIcon,
    /// This platform has no system tray.
    Unsupported,
    /// The platform refused, with its own explanation: no notification area on
    /// the desktop, a failed system call.
    Platform(String),
}

impl fmt::Display for TrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIcon => f.write_str("a tray icon needs a picture"),
            Self::Unsupported => f.write_str("this platform has no system tray"),
            Self::Platform(reason) => write!(f, "the system tray failed: {reason}"),
        }
    }
}

impl std::error::Error for TrayError {}

/// Describes a tray icon; [`Tray::build`] puts it on screen.
///
/// # Examples
///
/// ```no_run
/// use aimer_tray::{Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};
///
/// const QUIT: TrayItemId = TrayItemId(1);
///
/// let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
/// let tray = Tray::new()
///     .icon(icon)
///     .tooltip("My app")
///     .menu(TrayMenu::new().item(QUIT, "Quit"))
///     .on_event(|event| {
///         if event == TrayEvent::MenuItem(QUIT) {
///             // Hand over to the event loop here.
///         }
///     })
///     .build()
///     .unwrap();
/// // The icon stays until `tray` is dropped.
/// drop(tray);
/// ```
#[derive(Default)]
pub struct Tray {
    icon: Option<TrayIcon>,
    tooltip: String,
    menu: Option<TrayMenu>,
    handler: Option<TrayHandler>,
}

impl Tray {
    /// Starts a tray description with no icon, tooltip or menu.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The picture. Required.
    #[inline]
    pub fn icon(mut self, icon: TrayIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The text shown while the pointer rests on the icon.
    #[inline]
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    /// The menu the icon opens.
    #[inline]
    pub fn menu(mut self, menu: TrayMenu) -> Self {
        self.menu = Some(menu);
        self
    }

    /// Where clicks and chosen items are reported. See [`TrayHandler`] for the
    /// thread it is called on.
    #[inline]
    pub fn on_event(mut self, handler: impl Fn(TrayEvent) + Send + Sync + 'static) -> Self {
        self.handler = Some(Arc::new(handler));
        self
    }

    /// Shows the icon.
    ///
    /// On macOS call this on the main thread, which is where the event loop of
    /// an Aimer application runs.
    ///
    /// # Errors
    ///
    /// [`TrayError::MissingIcon`] without an icon, [`TrayError::Unsupported`]
    /// where there is no tray, and [`TrayError::Platform`] when the system
    /// refuses.
    pub fn build(self) -> Result<TrayHandle, TrayError> {
        let icon = self.icon.ok_or(TrayError::MissingIcon)?;
        let handler = self.handler.unwrap_or_else(|| Arc::new(|_| {}));
        let backend = backend::create(&icon, &self.tooltip, self.menu.as_ref(), handler)?;
        Ok(TrayHandle { backend, _not_send: PhantomData })
    }
}

/// A tray icon on screen. Dropping it removes the icon.
///
/// It stays on the thread that built it, because on macOS the status bar may
/// only be touched from the main thread.
pub struct TrayHandle {
    backend: Box<dyn Backend>,
    _not_send: PhantomData<*const ()>,
}

impl TrayHandle {
    /// Replaces the picture.
    #[inline]
    pub fn set_icon(&mut self, icon: &TrayIcon) {
        self.backend.set_icon(icon);
    }

    /// Replaces the tooltip.
    #[inline]
    pub fn set_tooltip(&mut self, tooltip: &str) {
        self.backend.set_tooltip(tooltip);
    }

    /// Replaces the menu, or removes it with `None`.
    #[inline]
    pub fn set_menu(&mut self, menu: Option<&TrayMenu>) {
        self.backend.set_menu(menu);
    }
}

impl fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrayHandle").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_without_icon_fails() {
        assert_eq!(Tray::new().tooltip("x").build().unwrap_err(), TrayError::MissingIcon);
    }
}
