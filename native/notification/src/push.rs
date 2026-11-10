//! The way push registration results reach the application.
//!
//! Push has two halves. [`crate::Notifier::register_push`] asks the platform for
//! a device token. The answer comes back through code the application's
//! launcher owns, not this library: the `UIApplicationDelegate` methods on iOS,
//! a `FirebaseMessagingService` on Android. That launcher code calls the
//! functions below, and whichever [`crate::Notifier`] is alive receives the
//! result as a [`crate::NotificationEvent`].
//!
//! A token that arrives before any notifier exists, as it does at launch, is
//! kept and delivered as soon as one is built.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::{NotificationEvent, NotificationHandler, PushMessage};

struct Sink {
    handler: Option<NotificationHandler>,
    token: Option<String>,
}

static SINK: Mutex<Sink> = Mutex::new(Sink { handler: None, token: None });

fn lock() -> MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Makes `handler` the receiver of push events and replays a token that was
/// reported earlier.
pub(crate) fn attach(handler: &NotificationHandler) {
    let replay = {
        let mut sink = lock();
        sink.handler = Some(Arc::clone(handler));
        sink.token.clone()
    };
    if let Some(token) = replay {
        handler(NotificationEvent::PushToken(token));
    }
}

/// Stops delivering to `handler`, unless another one has taken over.
pub(crate) fn detach(handler: &NotificationHandler) {
    let mut sink = lock();
    if sink.handler.as_ref().is_some_and(|current| Arc::ptr_eq(current, handler)) {
        sink.handler = None;
    }
}

fn deliver(event: NotificationEvent) {
    // Called outside the lock so a handler may report or attach in turn.
    let handler = lock().handler.clone();
    if let Some(handler) = handler {
        handler(event);
    }
}

/// Reports the device token the platform issued. Callable from any thread.
///
/// The latest token is remembered, so one reported before a notifier exists is
/// delivered when it does. Apple's raw token bytes go through [`hex_token`].
pub fn report_token(token: impl Into<String>) {
    let token = token.into();
    lock().token = Some(token.clone());
    deliver(NotificationEvent::PushToken(token));
}

/// Reports a push message that arrived while the application was in the
/// foreground. Callable from any thread.
pub fn report_message(message: PushMessage) {
    deliver(NotificationEvent::PushReceived(message));
}

/// Reports that the platform refused to register for push. Callable from any
/// thread.
pub fn report_failure(reason: impl Into<String>) {
    deliver(NotificationEvent::PushFailed(reason.into()));
}

/// The lowercase hexadecimal form of an APNs device token.
pub fn hex_token(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;

    use super::*;

    /// The sink is process-wide; tests that touch it take turns.
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());

    pub(crate) fn reset() {
        let mut sink = lock();
        sink.handler = None;
        sink.token = None;
    }

    fn collector() -> (NotificationHandler, Arc<Mutex<Vec<NotificationEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let handler: NotificationHandler = Arc::new(move |event| sink.lock().unwrap().push(event));
        (handler, seen)
    }

    #[test]
    fn hex_token_is_lowercase_and_padded() {
        assert_eq!(hex_token(&[0x00, 0x0a, 0xff, 0x10]), "000aff10");
        assert_eq!(hex_token(&[]), "");
    }

    #[test]
    fn events_reach_the_attached_handler() {
        let _turn = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (handler, seen) = collector();
        attach(&handler);
        report_token("abc");
        report_message(PushMessage { title: Some("t".into()), ..PushMessage::default() });
        report_failure("nope");
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                NotificationEvent::PushToken("abc".into()),
                NotificationEvent::PushReceived(PushMessage {
                    title: Some("t".into()),
                    ..PushMessage::default()
                }),
                NotificationEvent::PushFailed("nope".into()),
            ]
        );
        detach(&handler);
        reset();
    }

    #[test]
    fn early_token_is_replayed_on_attach() {
        let _turn = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        report_token("early");
        let (handler, seen) = collector();
        attach(&handler);
        assert_eq!(*seen.lock().unwrap(), vec![NotificationEvent::PushToken("early".into())]);
        detach(&handler);
        reset();
    }

    #[test]
    fn detach_stops_delivery_but_only_for_the_current_handler() {
        let _turn = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (first, first_seen) = collector();
        let (second, second_seen) = collector();
        attach(&first);
        attach(&second);
        detach(&first); // already replaced: must not unseat `second`
        report_failure("x");
        assert!(first_seen.lock().unwrap().is_empty());
        assert_eq!(second_seen.lock().unwrap().len(), 1);
        detach(&second);
        report_failure("y");
        assert_eq!(second_seen.lock().unwrap().len(), 1);
        reset();
    }
}
