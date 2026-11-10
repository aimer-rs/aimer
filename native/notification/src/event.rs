//! What the notification system tells the application.

use crate::{NotificationActionId, NotificationId, Permission};

/// A remote notification that reached the application while it was running.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushMessage {
    pub title: Option<String>,
    pub body: Option<String>,
    /// The custom key and value pairs the sender attached.
    pub data: Vec<(String, String)>,
}

/// Something that happened to a notification or to push registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotificationEvent {
    /// The user opened the notification.
    Clicked(NotificationId),
    /// The user chose one of its buttons.
    Action { notification: NotificationId, action: NotificationActionId },
    /// The user swiped the notification away.
    Dismissed(NotificationId),
    /// The platform answered a [`crate::Notifier::permission`] or
    /// [`crate::Notifier::request_permission`] call. The callback given to the
    /// call runs on the platform's thread; this event carries the same answer
    /// through the handler, which is how an application hears it on its own.
    Permission(Permission),
    /// The platform issued the device token for push: the APNs token as
    /// lowercase hexadecimal, or the FCM registration token. Send it to your
    /// server.
    PushToken(String),
    /// A push message arrived while the application was in the foreground.
    PushReceived(PushMessage),
    /// The platform refused to register for push.
    PushFailed(String),
}

/// Receives [`NotificationEvent`]s from whichever thread heard them.
///
/// On Apple platforms that is a thread of the system's notification center; on
/// Windows and Linux a thread of the library's own. A handler that touches the
/// interface should hand the event over instead (post it to the event loop).
pub type NotificationHandler = std::sync::Arc<dyn Fn(NotificationEvent) + Send + Sync>;
