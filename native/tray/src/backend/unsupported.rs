//! Targets without a system tray: iOS, Android, the web.

use super::Backend;
use crate::{TrayError, TrayHandler, TrayIcon, TrayMenu};

pub(super) fn create(
    _icon: &TrayIcon,
    _tooltip: &str,
    _menu: Option<&TrayMenu>,
    _handler: TrayHandler,
) -> Result<Box<dyn Backend>, TrayError> {
    Err(TrayError::Unsupported)
}
