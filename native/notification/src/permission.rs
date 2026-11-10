//! Whether the user lets the application notify them.

/// The user's decision about notifications for this application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// The user has not been asked. [`crate::Notifier::request_permission`]
    /// asks.
    NotDetermined,
    /// Notifications are shown.
    Granted,
    /// Notifications are silently dropped until the user changes the setting.
    Denied,
}

/// Receives a [`Permission`] once the platform has answered, from whichever
/// thread it answers on.
pub type PermissionCallback = Box<dyn FnOnce(Permission) + Send>;
