//! The contract each platform fulfils, and the choice of platform.

use crate::{Notification, NotificationError, NotificationHandler, NotificationId, PermissionCallback};

/// One connection to a platform's notification service.
pub(crate) trait Backend {
    /// Shows the notification, or schedules it when it carries a delay.
    fn show(&self, notification: &Notification) -> Result<(), NotificationError>;
    /// Removes the notification, shown or still scheduled.
    fn close(&self, id: NotificationId) -> Result<(), NotificationError>;
    /// Reports the current permission without prompting.
    fn permission(&self, done: PermissionCallback);
    /// Prompts if needed, then reports the permission.
    fn request_permission(&self, done: PermissionCallback);
    /// Asks the platform for a push token; the answer comes through
    /// [`crate::push`].
    fn register_push(&self) -> Result<(), NotificationError>;
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use apple as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as platform;
#[cfg(target_os = "android")]
pub use android::set_android_activity;

// Always compiled: platform modules that cannot do something fall back to it.
#[allow(dead_code)]
mod unsupported;
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "windows",
    target_os = "linux",
    target_os = "android"
)))]
use unsupported as platform;

pub(crate) fn create(handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    platform::create(handler)
}
