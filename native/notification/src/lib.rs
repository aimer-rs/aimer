#![doc = include_str!("../README.md")]

mod backend;
mod event;
mod notification;
mod notifier;
mod permission;
pub mod push;
// Only the backends without a native delayed delivery use it.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "android", test))]
mod scheduler;

#[cfg(target_os = "android")]
pub use backend::set_android_activity;
pub use event::{NotificationEvent, NotificationHandler, PushMessage};
pub use notification::{Notification, NotificationAction, NotificationActionId, NotificationId};
pub use notifier::{Notifier, NotifierBuilder, NotificationError};
pub use permission::{Permission, PermissionCallback};
