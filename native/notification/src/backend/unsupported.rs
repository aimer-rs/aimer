//! Platforms with no notification service this library can reach.

use super::Backend;
use crate::{
    Notification, NotificationError, NotificationHandler, NotificationId, Permission,
    PermissionCallback,
};

struct Unsupported;

impl Backend for Unsupported {
    fn show(&self, _notification: &Notification) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }

    fn close(&self, _id: NotificationId) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }

    fn permission(&self, done: PermissionCallback) {
        done(Permission::Denied);
    }

    fn request_permission(&self, done: PermissionCallback) {
        done(Permission::Denied);
    }

    fn register_push(&self) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }
}

/// Always fails: there is nothing to connect to.
pub(super) fn create(_handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    Err(NotificationError::Unsupported)
}

/// What `Notifier` falls back to for a call a platform cannot serve.
#[allow(dead_code)]
pub(super) fn backend() -> Box<dyn Backend> {
    Box::new(Unsupported)
}
