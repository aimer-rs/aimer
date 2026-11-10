#![doc = include_str!("../README.md")]

mod autostart;
mod daemon;
mod run_loop;
mod scheduler;
mod signals;

pub use aimer_tray::{
    IconError, Tray, TrayButton, TrayError, TrayEvent, TrayHandle, TrayHandler, TrayIcon,
    TrayItemId, TrayMenu, TrayMenuEntry, TrayMenuItem,
};
pub use autostart::Autostart;
pub use daemon::{Daemon, DaemonError, DaemonHandle, Running};
pub use scheduler::TaskHandle;
