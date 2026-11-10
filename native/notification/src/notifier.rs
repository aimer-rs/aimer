//! The handle an application shows notifications through.

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::backend::{self, Backend};
use crate::{
    Notification, NotificationEvent, NotificationHandler, NotificationId, PermissionCallback, push,
};

/// Why a notification operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationError {
    /// The notification has no title, subtitle or body.
    Empty,
    /// This platform has no notification service this library can reach.
    Unsupported,
    /// The platform refused, with its own explanation: no notification daemon
    /// on the desktop, a failed system call.
    Platform(String),
}

impl fmt::Display for NotificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a notification needs a title, subtitle or body"),
            Self::Unsupported => f.write_str("this platform has no notification service"),
            Self::Platform(reason) => write!(f, "the notification service failed: {reason}"),
        }
    }
}

impl std::error::Error for NotificationError {}

/// Describes a [`Notifier`]; [`NotifierBuilder::build`] connects it to the
/// platform.
#[derive(Default)]
pub struct NotifierBuilder {
    handler: Option<NotificationHandler>,
}

impl NotifierBuilder {
    /// Where clicks, buttons, dismissals and push results are reported. See
    /// [`NotificationHandler`] for the thread it is called on.
    #[inline]
    pub fn on_event(mut self, handler: impl Fn(NotificationEvent) + Send + Sync + 'static) -> Self {
        self.handler = Some(Arc::new(handler));
        self
    }

    /// Connects to the platform's notification service.
    ///
    /// On macOS and iOS call this on the main thread, which is where the event
    /// loop of an Aimer application runs. On macOS the application must run
    /// from an app bundle: the system refuses notifications for a bare
    /// executable.
    ///
    /// # Errors
    ///
    /// [`NotificationError::Unsupported`] where there is no service, and
    /// [`NotificationError::Platform`] when the system refuses.
    pub fn build(self) -> Result<Notifier, NotificationError> {
        let handler = self.handler.unwrap_or_else(|| Arc::new(|_| {}));
        let backend = backend::create(Arc::clone(&handler))?;
        push::attach(&handler);
        Ok(Notifier { backend, handler, _not_send: PhantomData })
    }
}

/// A connection to the platform's notification service. Dropping it stops
/// event delivery and discards notifications still waiting for their delay.
///
/// It stays on the thread that built it.
///
/// # Examples
///
/// ```no_run
/// use aimer_notification::{Notification, NotificationEvent, NotificationId, Notifier, Permission};
///
/// let notifier = Notifier::new()
///     .on_event(|event| {
///         if let NotificationEvent::Clicked(id) = event {
///             // Hand over to the event loop here.
///             let _ = id;
///         }
///     })
///     .build()
///     .unwrap();
///
/// notifier.request_permission(Box::new(|_permission: Permission| {}));
/// notifier
///     .show(&Notification::new(NotificationId(1)).title("Hello").body("From Aimer"))
///     .unwrap();
/// ```
pub struct Notifier {
    backend: Box<dyn Backend>,
    handler: NotificationHandler,
    _not_send: PhantomData<*const ()>,
}

impl Notifier {
    /// Starts describing a notifier.
    #[inline]
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> NotifierBuilder {
        NotifierBuilder::default()
    }

    /// Shows `notification`, or schedules it when it has a delay. A
    /// notification with the id of one already shown replaces it.
    ///
    /// # Errors
    ///
    /// [`NotificationError::Empty`] for a notification with no text, and
    /// [`NotificationError::Platform`] when the service refuses.
    pub fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
        if notification.is_empty() {
            return Err(NotificationError::Empty);
        }
        self.backend.show(notification)
    }

    /// Removes the notification named `id`, whether it is on screen or still
    /// waiting for its delay.
    ///
    /// # Errors
    ///
    /// [`NotificationError::Platform`] when the service refuses.
    pub fn close(&self, id: NotificationId) -> Result<(), NotificationError> {
        self.backend.close(id)
    }

    /// Asks the platform what the user decided, without prompting. `done` runs
    /// once, on whichever thread the platform answers on.
    pub fn permission(&self, done: PermissionCallback) {
        self.backend.permission(self.report_permission(done));
    }

    /// Asks the user to allow notifications if they have not been asked, and
    /// reports the decision. `done` runs once, on whichever thread the platform
    /// answers on. Where permission is not a concept (most desktops) it
    /// reports [`crate::Permission::Granted`] at once.
    pub fn request_permission(&self, done: PermissionCallback) {
        self.backend.request_permission(self.report_permission(done));
    }

    /// Asks the platform for a push device token. The answer arrives as
    /// [`NotificationEvent::PushToken`] or [`NotificationEvent::PushFailed`]
    /// through the handler, by way of [`crate::push`].
    ///
    /// # Errors
    ///
    /// [`NotificationError::Unsupported`] where push does not exist, such as
    /// desktop Linux and Windows.
    pub fn register_push(&self) -> Result<(), NotificationError> {
        self.backend.register_push()
    }

    /// Wraps `done` so the answer also reaches the handler as an event.
    fn report_permission(&self, done: PermissionCallback) -> PermissionCallback {
        let handler = Arc::clone(&self.handler);
        Box::new(move |permission| {
            done(permission);
            handler(NotificationEvent::Permission(permission));
        })
    }
}

impl Drop for Notifier {
    fn drop(&mut self) {
        push::detach(&self.handler);
    }
}

impl fmt::Debug for Notifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Notifier").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::Permission;

    /// Answers every question at once, remembering what it was asked to show.
    struct Fake {
        shown: Arc<Mutex<Vec<NotificationId>>>,
    }

    impl Backend for Fake {
        fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
            self.shown.lock().unwrap().push(notification.id());
            Ok(())
        }

        fn close(&self, _id: NotificationId) -> Result<(), NotificationError> {
            Ok(())
        }

        fn permission(&self, done: PermissionCallback) {
            done(Permission::NotDetermined);
        }

        fn request_permission(&self, done: PermissionCallback) {
            done(Permission::Granted);
        }

        fn register_push(&self) -> Result<(), NotificationError> {
            Err(NotificationError::Unsupported)
        }
    }

    type Shown = Arc<Mutex<Vec<NotificationId>>>;
    type Events = Arc<Mutex<Vec<NotificationEvent>>>;

    fn notifier() -> (Notifier, Shown, Events) {
        let shown = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let handler: NotificationHandler = Arc::new(move |event| sink.lock().unwrap().push(event));
        let notifier = Notifier {
            backend: Box::new(Fake { shown: Arc::clone(&shown) }),
            handler,
            _not_send: PhantomData,
        };
        (notifier, shown, events)
    }

    #[test]
    fn an_empty_notification_never_reaches_the_backend() {
        let (notifier, shown, _) = notifier();
        assert_eq!(
            notifier.show(&Notification::new(NotificationId(1))),
            Err(NotificationError::Empty)
        );
        assert!(shown.lock().unwrap().is_empty());
        notifier.show(&Notification::new(NotificationId(2)).title("x")).unwrap();
        assert_eq!(*shown.lock().unwrap(), vec![NotificationId(2)]);
    }

    #[test]
    fn permission_answers_reach_the_callback_and_the_handler() {
        let (notifier, _, events) = notifier();
        let asked = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&asked);
        notifier.request_permission(Box::new(move |p| *slot.lock().unwrap() = Some(p)));
        assert_eq!(*asked.lock().unwrap(), Some(Permission::Granted));
        notifier.permission(Box::new(|_| {}));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                NotificationEvent::Permission(Permission::Granted),
                NotificationEvent::Permission(Permission::NotDetermined),
            ]
        );
    }

    #[test]
    fn errors_display() {
        assert_eq!(
            NotificationError::Empty.to_string(),
            "a notification needs a title, subtitle or body"
        );
        assert!(NotificationError::Platform("boom".into()).to_string().contains("boom"));
    }
}
