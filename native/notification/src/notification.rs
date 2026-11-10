//! What a notification says.

use std::time::Duration;

/// Names a notification, so it can be replaced, closed, and recognised in the
/// events it produces. Showing a notification with an id that is already on
/// screen replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationId(pub u64);

/// Names a button of a notification in [`crate::NotificationEvent::Action`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationActionId(pub u32);

/// A button under a notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationAction {
    pub(crate) id: NotificationActionId,
    pub(crate) label: String,
}

impl NotificationAction {
    /// A button called `label`, reported as `id` when chosen.
    #[inline]
    pub fn new(id: NotificationActionId, label: impl Into<String>) -> Self {
        Self { id, label: label.into() }
    }

    /// The id reported when the button is chosen.
    #[inline]
    pub fn id(&self) -> NotificationActionId {
        self.id
    }

    /// The text on the button.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A message for the system to show outside the application's window.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use aimer_notification::{Notification, NotificationAction, NotificationActionId, NotificationId};
///
/// let reminder = Notification::new(NotificationId(1))
///     .title("Stand up")
///     .body("You have been sitting for an hour.")
///     .action(NotificationAction::new(NotificationActionId(1), "Snooze"))
///     .delay(Duration::from_secs(3600));
/// assert_eq!(reminder.id(), NotificationId(1));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub(crate) id: NotificationId,
    pub(crate) title: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) body: String,
    pub(crate) sound: bool,
    pub(crate) delay: Option<Duration>,
    pub(crate) actions: Vec<NotificationAction>,
}

impl Notification {
    /// An empty notification named `id`. Give it a [`title`](Self::title) or a
    /// [`body`](Self::body) before showing it.
    #[inline]
    pub fn new(id: NotificationId) -> Self {
        Self {
            id,
            title: String::new(),
            subtitle: None,
            body: String::new(),
            sound: true,
            delay: None,
            actions: Vec::new(),
        }
    }

    /// The bold first line.
    #[inline]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// A line between the title and the body, where the platform has one;
    /// elsewhere it is joined to the body.
    #[inline]
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The message.
    #[inline]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Whether the notification plays the default sound. On by default.
    #[inline]
    pub fn sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }

    /// Delivers the notification after `delay` instead of at once.
    ///
    /// Apple platforms hand the delay to the system, which delivers it even if
    /// the application has quit. Elsewhere the library waits, so the
    /// notification is lost if the process exits first.
    #[inline]
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Adds a button. Platforms show as many as they have room for.
    #[inline]
    pub fn action(mut self, action: NotificationAction) -> Self {
        self.actions.push(action);
        self
    }

    /// The id this notification is shown, replaced and closed under.
    #[inline]
    pub fn id(&self) -> NotificationId {
        self.id
    }

    /// The body, with the subtitle in front of it on one line when there is
    /// one. For platforms that have no subtitle of their own.
    #[cfg_attr(not(any(target_os = "linux", target_os = "windows", target_os = "android", test)), allow(dead_code))]
    pub(crate) fn flat_body(&self) -> String {
        match (&self.subtitle, self.body.is_empty()) {
            (Some(subtitle), false) => format!("{subtitle}\n{}", self.body),
            (Some(subtitle), true) => subtitle.clone(),
            (None, _) => self.body.clone(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.title.is_empty() && self.body.is_empty() && self.subtitle.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_body_joins_subtitle_and_body() {
        let n = Notification::new(NotificationId(1)).subtitle("Sub").body("Body");
        assert_eq!(n.flat_body(), "Sub\nBody");
        assert_eq!(Notification::new(NotificationId(1)).subtitle("Sub").flat_body(), "Sub");
        assert_eq!(Notification::new(NotificationId(1)).body("Body").flat_body(), "Body");
    }

    #[test]
    fn empty_notification_is_detected() {
        assert!(Notification::new(NotificationId(1)).is_empty());
        assert!(!Notification::new(NotificationId(1)).title("x").is_empty());
        assert!(!Notification::new(NotificationId(1)).subtitle("x").is_empty());
    }
}
