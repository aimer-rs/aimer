#![doc = include_str!("../README.md")]

mod backend;
mod event;
mod icon;
mod menu;
mod tray;

pub use event::{TrayButton, TrayEvent, TrayHandler};
pub use icon::{IconError, TrayIcon};
pub use menu::{TrayItemId, TrayMenu, TrayMenuEntry, TrayMenuItem};
pub use tray::{Tray, TrayError, TrayHandle};
