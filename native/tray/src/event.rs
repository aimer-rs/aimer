//! What the tray tells the application.

use crate::TrayItemId;

/// A mouse button on the tray icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayButton {
    Left,
    Right,
    Middle,
}

/// Something the user did with the tray icon.
///
/// Whether a click is reported depends on the platform: where the icon has a
/// menu and the system opens it by itself on a click (macOS always, Linux
/// hosts on a right click), that click opens the menu and only the chosen item
/// is reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    Click(TrayButton),
    DoubleClick(TrayButton),
    /// An item of the menu was chosen.
    MenuItem(TrayItemId),
}

/// Receives [`TrayEvent`]s from whichever thread heard them.
///
/// On macOS that is the main thread; on Windows and Linux it is a thread of the
/// tray's own, so a handler that touches the interface should hand the event
/// over instead (post it to the event loop, push it into a queue).
pub type TrayHandler = std::sync::Arc<dyn Fn(TrayEvent) + Send + Sync>;
