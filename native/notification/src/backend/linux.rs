//! Linux: the freedesktop `org.freedesktop.Notifications` service on the
//! session bus.
//!
//! Every desktop environment ships a daemon that implements it (GNOME Shell,
//! KDE Plasma, dunst, mako, ...). `Notify` shows or replaces a notification and
//! returns an id the *server* chose; `ActionInvoked` and `NotificationClosed`
//! come back as signals carrying that id, which this module maps to the
//! caller's [`NotificationId`].
//!
//! The desktop has no permission concept, so permission is always granted. A
//! delayed notification is held by the library's [`Scheduler`] and shown at
//! once when due, so it is lost if the process exits first.
//!
//! # Threads
//!
//! One listener thread blocks on the signal stream (no polling, no wake-ups
//! while idle) and calls the handler. Dropping the backend closes the
//! connection, which ends the stream and lets the thread return; a flag checked
//! after every message covers a stream that outlives the close. The scheduler
//! owns its own thread and stops with the backend.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

use super::Backend;
use crate::scheduler::Scheduler;
use crate::{
    Notification, NotificationActionId, NotificationError, NotificationEvent, NotificationHandler,
    NotificationId, Permission, PermissionCallback,
};

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";

/// The action key daemons report when the notification body is clicked.
const DEFAULT_ACTION: &str = "default";

/// `NotificationClosed` reason: the user dismissed it.
const REASON_DISMISSED: u32 = 2;

fn platform(error: impl Display) -> NotificationError {
    NotificationError::Platform(error.to_string())
}

/// What the daemon says it can do, from `GetCapabilities`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Capabilities {
    actions: bool,
    sound: bool,
}

impl Capabilities {
    fn from_names<S: AsRef<str>>(names: &[S]) -> Self {
        let has = |wanted: &[&str]| names.iter().any(|name| wanted.contains(&name.as_ref()));
        Self { actions: has(&["actions"]), sound: has(&["sound", "sound-name", "suppress-sound"]) }
    }
}

/// The `actions` argument of `Notify`: flat `key, label` pairs, the first being
/// the click on the body. A daemon without the `actions` capability gets none.
fn action_list(notification: &Notification, capabilities: Capabilities) -> Vec<String> {
    if !capabilities.actions {
        return Vec::new();
    }
    let mut list = Vec::with_capacity(2 + notification.actions.len() * 2);
    list.push(DEFAULT_ACTION.to_owned());
    list.push(String::new());
    for action in &notification.actions {
        list.push(action.id.0.to_string());
        list.push(action.label.clone());
    }
    list
}

/// Whether the `suppress-sound` hint is worth sending.
fn mutes_sound(notification: &Notification, capabilities: Capabilities) -> bool {
    !notification.sound && capabilities.sound
}

/// An `ActionInvoked` key as an event: `default` is a click on the
/// notification, a decimal number is one of its buttons, anything else is not
/// ours.
fn action_event(notification: NotificationId, key: &str) -> Option<NotificationEvent> {
    if key == DEFAULT_ACTION {
        return Some(NotificationEvent::Clicked(notification));
    }
    key.parse::<u32>()
        .ok()
        .map(|action| NotificationEvent::Action { notification, action: NotificationActionId(action) })
}

/// A `NotificationClosed` reason as an event. Expiry (1), `CloseNotification`
/// (3) and the undefined reasons (4 and up) are not something the user did.
fn closed_event(notification: NotificationId, reason: u32) -> Option<NotificationEvent> {
    (reason == REASON_DISMISSED).then_some(NotificationEvent::Dismissed(notification))
}

/// The two-way map between the server's ids and the caller's.
#[derive(Debug, Default)]
struct Ids {
    by_server: HashMap<u32, NotificationId>,
    by_notification: HashMap<NotificationId, u32>,
}

impl Ids {
    /// The id to pass as `replaces_id`; `0` means "a new notification".
    fn replaces(&self, notification: NotificationId) -> u32 {
        self.by_notification.get(&notification).copied().unwrap_or(0)
    }

    fn resolve(&self, server: u32) -> Option<NotificationId> {
        self.by_server.get(&server).copied()
    }

    /// Records that `server` now shows `notification`, forgetting a different
    /// id it was shown under before.
    fn insert(&mut self, server: u32, notification: NotificationId) {
        if let Some(previous) = self.by_notification.insert(notification, server)
            && previous != server
        {
            self.by_server.remove(&previous);
        }
        if let Some(previous) = self.by_server.insert(server, notification)
            && previous != notification
        {
            self.by_notification.remove(&previous);
        }
    }

    /// Forgets the notification the server called `server`.
    fn remove_server(&mut self, server: u32) -> Option<NotificationId> {
        let notification = self.by_server.remove(&server)?;
        self.by_notification.remove(&notification);
        Some(notification)
    }

    /// Forgets `notification`, returning the id the server knew it by.
    fn remove(&mut self, notification: NotificationId) -> Option<u32> {
        let server = self.by_notification.remove(&notification)?;
        self.by_server.remove(&server);
        Some(server)
    }
}

/// What the backend, the listener thread and the scheduler thread share.
struct Shared {
    connection: Connection,
    handler: NotificationHandler,
    app_name: String,
    capabilities: Capabilities,
    ids: Mutex<Ids>,
    stopped: AtomicBool,
}

impl Shared {
    fn ids(&self) -> MutexGuard<'_, Ids> {
        self.ids.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn show_now(&self, notification: &Notification) -> Result<(), NotificationError> {
        let actions = action_list(notification, self.capabilities);
        let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
        if mutes_sound(notification, self.capabilities) {
            hints.insert("suppress-sound", Value::from(true));
        }
        let body = notification.flat_body();
        // Held across the call so a concurrent `show` of the same notification
        // sees the id this one gets, and the listener never hears about an id it
        // has not been told of yet.
        let mut ids = self.ids();
        let reply = self
            .connection
            .call_method(
                Some(DESTINATION),
                PATH,
                Some(INTERFACE),
                "Notify",
                &(
                    self.app_name.as_str(),
                    ids.replaces(notification.id),
                    "",
                    notification.title.as_str(),
                    body.as_str(),
                    actions,
                    hints,
                    -1i32,
                ),
            )
            .map_err(platform)?;
        let server: u32 = reply.body().deserialize().map_err(platform)?;
        ids.insert(server, notification.id);
        Ok(())
    }

    fn close_now(&self, id: NotificationId) -> Result<(), NotificationError> {
        let server = self.ids().remove(id);
        let Some(server) = server else { return Ok(()) };
        self.connection
            .call_method(Some(DESTINATION), PATH, Some(INTERFACE), "CloseNotification", &server)
            .map(drop)
            .map_err(platform)
    }

    /// Turns one signal into an event for the handler, if it concerns a
    /// notification of ours and the user caused it.
    fn handle_signal(&self, message: &zbus::Message) {
        let header = message.header();
        let Some(member) = header.member() else { return };
        let event = match member.as_str() {
            "ActionInvoked" => {
                let Ok((server, key)) = message.body().deserialize::<(u32, String)>() else { return };
                let notification = self.ids().resolve(server);
                notification.and_then(|id| action_event(id, &key))
            }
            "NotificationClosed" => {
                let Ok((server, reason)) = message.body().deserialize::<(u32, u32)>() else { return };
                // Closed for good: the id is gone from the server either way.
                let notification = self.ids().remove_server(server);
                notification.and_then(|id| closed_event(id, reason))
            }
            _ => None,
        };
        if let Some(event) = event {
            (self.handler)(event);
        }
    }
}

/// Blocks on the signal stream until the connection closes or the backend is
/// dropped. A stream error means the bus connection is gone, so it ends too.
fn listen(shared: &Shared, signals: MessageIterator) {
    for message in signals {
        if shared.stopped.load(Ordering::Acquire) {
            return;
        }
        let Ok(message) = message else { return };
        shared.handle_signal(&message);
    }
}

struct LinuxNotifier {
    shared: Arc<Shared>,
    scheduler: Scheduler,
}

impl Backend for LinuxNotifier {
    fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
        match notification.delay {
            Some(delay) => {
                self.scheduler.schedule(notification.clone(), delay);
                Ok(())
            }
            None => {
                // An immediate show supersedes a delayed one of the same id.
                self.scheduler.cancel(notification.id);
                self.shared.show_now(notification)
            }
        }
    }

    fn close(&self, id: NotificationId) -> Result<(), NotificationError> {
        self.scheduler.cancel(id);
        self.shared.close_now(id)
    }

    fn permission(&self, done: PermissionCallback) {
        done(Permission::Granted);
    }

    fn request_permission(&self, done: PermissionCallback) {
        done(Permission::Granted);
    }

    fn register_push(&self) -> Result<(), NotificationError> {
        Err(NotificationError::Unsupported)
    }
}

impl Drop for LinuxNotifier {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        // Ends the listener's stream. The bus is going away with us, so a
        // failure to close it cleanly has nothing to be reported to.
        let _ = self.shared.connection.clone().close();
    }
}

/// The executable's name, which daemons show as the sender of a notification.
fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "aimer".to_owned())
}

pub(super) fn create(handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    let connection = Connection::session().map_err(platform)?;
    let reply = connection
        .call_method(Some(DESTINATION), PATH, Some(INTERFACE), "GetCapabilities", &())
        .map_err(|error| NotificationError::Platform(format!("no notification daemon on the session bus: {error}")))?;
    let names: Vec<String> = reply.body().deserialize().map_err(platform)?;

    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(INTERFACE)
        .map_err(platform)?
        .path(PATH)
        .map_err(platform)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, &connection, None).map_err(platform)?;

    let shared = Arc::new(Shared {
        connection,
        handler,
        app_name: app_name(),
        capabilities: Capabilities::from_names(&names),
        ids: Mutex::new(Ids::default()),
        stopped: AtomicBool::new(false),
    });

    let listener = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("aimer-notification-listener".into())
        .spawn(move || listen(&listener, signals))
        .map_err(platform)?;

    let firing = Arc::clone(&shared);
    let scheduler = Scheduler::new(move |notification| {
        // Nobody is waiting on a delayed notification, so a failure to show it
        // has no one to be reported to.
        let _ = firing.show_now(&notification);
    })
    .map_err(platform)?;

    Ok(Box::new(LinuxNotifier { shared, scheduler }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NotificationAction;

    const ALL: Capabilities = Capabilities { actions: true, sound: true };

    fn note() -> Notification {
        Notification::new(NotificationId(7)).title("t")
    }

    #[test]
    fn capabilities_are_read_from_names() {
        assert_eq!(Capabilities::from_names(&["body", "actions"]), Capabilities { actions: true, sound: false });
        assert_eq!(Capabilities::from_names(&["sound"]), Capabilities { actions: false, sound: true });
        assert!(Capabilities::from_names(&["suppress-sound"]).sound);
        assert_eq!(Capabilities::from_names::<&str>(&[]), Capabilities::default());
    }

    #[test]
    fn actions_start_with_the_default_click() {
        assert_eq!(action_list(&note(), ALL), ["default", ""]);
    }

    #[test]
    fn actions_list_buttons_by_decimal_id() {
        let n = note()
            .action(NotificationAction::new(NotificationActionId(1), "Snooze"))
            .action(NotificationAction::new(NotificationActionId(42), "Open"));
        assert_eq!(action_list(&n, ALL), ["default", "", "1", "Snooze", "42", "Open"]);
    }

    #[test]
    fn no_actions_are_sent_without_the_capability() {
        let n = note().action(NotificationAction::new(NotificationActionId(1), "Snooze"));
        assert!(action_list(&n, Capabilities { actions: false, sound: true }).is_empty());
    }

    #[test]
    fn sound_is_muted_only_when_asked_and_supported() {
        assert!(mutes_sound(&note().sound(false), ALL));
        assert!(!mutes_sound(&note(), ALL));
        assert!(!mutes_sound(&note().sound(false), Capabilities { actions: true, sound: false }));
    }

    #[test]
    fn default_key_is_a_click() {
        assert_eq!(action_event(NotificationId(3), "default"), Some(NotificationEvent::Clicked(NotificationId(3))));
    }

    #[test]
    fn numeric_key_is_an_action() {
        assert_eq!(
            action_event(NotificationId(3), "42"),
            Some(NotificationEvent::Action { notification: NotificationId(3), action: NotificationActionId(42) })
        );
    }

    #[test]
    fn foreign_keys_are_ignored() {
        assert_eq!(action_event(NotificationId(3), "inline-reply"), None);
        assert_eq!(action_event(NotificationId(3), ""), None);
        assert_eq!(action_event(NotificationId(3), "-1"), None);
        assert_eq!(action_event(NotificationId(3), "4294967296"), None);
    }

    #[test]
    fn only_a_dismissal_is_an_event() {
        let id = NotificationId(9);
        assert_eq!(closed_event(id, 1), None, "expired");
        assert_eq!(closed_event(id, 2), Some(NotificationEvent::Dismissed(id)));
        assert_eq!(closed_event(id, 3), None, "closed by the application");
        assert_eq!(closed_event(id, 4), None, "undefined");
        assert_eq!(closed_event(id, 99), None);
    }

    #[test]
    fn ids_start_as_a_new_notification() {
        assert_eq!(Ids::default().replaces(NotificationId(1)), 0);
    }

    #[test]
    fn ids_map_both_ways() {
        let mut ids = Ids::default();
        ids.insert(10, NotificationId(1));
        ids.insert(11, NotificationId(2));
        assert_eq!(ids.replaces(NotificationId(1)), 10);
        assert_eq!(ids.resolve(11), Some(NotificationId(2)));
        assert_eq!(ids.resolve(12), None);
    }

    #[test]
    fn reshowing_with_a_new_server_id_forgets_the_old_one() {
        let mut ids = Ids::default();
        ids.insert(10, NotificationId(1));
        ids.insert(20, NotificationId(1));
        assert_eq!(ids.replaces(NotificationId(1)), 20);
        assert_eq!(ids.resolve(10), None);
        assert_eq!(ids.resolve(20), Some(NotificationId(1)));
    }

    #[test]
    fn a_recycled_server_id_moves_to_the_new_notification() {
        let mut ids = Ids::default();
        ids.insert(10, NotificationId(1));
        ids.insert(10, NotificationId(2));
        assert_eq!(ids.resolve(10), Some(NotificationId(2)));
        assert_eq!(ids.replaces(NotificationId(1)), 0);
    }

    #[test]
    fn closing_removes_both_directions() {
        let mut ids = Ids::default();
        ids.insert(10, NotificationId(1));
        assert_eq!(ids.remove_server(10), Some(NotificationId(1)));
        assert_eq!(ids.replaces(NotificationId(1)), 0);
        assert_eq!(ids.remove_server(10), None);

        ids.insert(11, NotificationId(2));
        assert_eq!(ids.remove(NotificationId(2)), Some(11));
        assert_eq!(ids.resolve(11), None);
        assert_eq!(ids.remove(NotificationId(2)), None);
    }
}
