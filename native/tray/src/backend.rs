//! The contract each platform fulfils, and the choice of platform.

use crate::{TrayError, TrayHandler, TrayIcon, TrayMenu};

/// One live tray icon. Dropping it removes the icon.
pub(crate) trait Backend {
    fn set_icon(&mut self, icon: &TrayIcon);
    fn set_tooltip(&mut self, tooltip: &str);
    fn set_menu(&mut self, menu: Option<&TrayMenu>);
}

#[cfg(any(target_os = "linux", test))]
mod flat_menu;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
use unsupported as platform;

pub(crate) fn create(
    icon: &TrayIcon,
    tooltip: &str,
    menu: Option<&TrayMenu>,
    handler: TrayHandler,
) -> Result<Box<dyn Backend>, TrayError> {
    platform::create(icon, tooltip, menu, handler)
}
