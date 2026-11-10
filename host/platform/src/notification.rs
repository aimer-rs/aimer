//! System notifications, wired into the event loop.
//!
//! [`aimer_notification`] reports a click, a chosen button, a dismissal or a
//! push result from whichever thread the platform heard it on. This module makes
//! that uniform for an application. [`install`] connects to the platform's
//! notification service and routes its events to the event loop, which calls
//! your handler on the UI thread, where it can touch widget state like any
//! other callback.
//!
//! Everything [`aimer_notification`] exports is re-exported here, so
//! `Notification`, `Notifier` and the rest are reachable from this one module.
//! Messages that never leave the window are a different thing: see the toasts in
//! `aimer_feedback`.
//!
//! # Examples
//!
//! ```no_run
//! use aimer_platform::notification::{self, Notification, NotificationEvent, NotificationId};
//!
//! let notifier = notification::install(|event| {
//!     if let NotificationEvent::Clicked(_id) = event {
//!         // Runs inside the event loop.
//!     }
//! })?;
//! notifier.request_permission(Box::new(|_| {}));
//! notifier.show(&Notification::new(NotificationId(1)).title("Hello"))?;
//! # Ok::<(), notification::NotificationError>(())
//! ```

use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

pub use aimer_notification::*;

use crate::{AimerNativePlatformEvent, EVENT_PROXY};

/// Names one installed notifier in the event the loop receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotifierId(u32);

impl NotifierId {
    /// The id as a number.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// The id from [`NotifierId::as_raw`].
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

type Handler = Rc<dyn Fn(NotificationEvent)>;

aimer_tls::thread_local! {
    /// The handlers of this thread, the UI thread. A handful at most, so a
    /// vector is faster than a map.
    static HANDLERS: RefCell<Vec<(NotifierId, Handler)>> = const { RefCell::new(Vec::new()) };
}

/// A handler waiting for the events of one notifier. Dropping it unsubscribes.
///
/// It stays on the thread that created it, the UI thread, because the handler
/// is not required to be `Send`.
#[must_use = "the handler is removed as soon as the subscription is dropped"]
pub struct NotifierSubscription {
    id: NotifierId,
    _not_send: PhantomData<*const ()>,
}

impl NotifierSubscription {
    /// The id events for this subscription carry.
    #[inline]
    pub fn id(&self) -> NotifierId {
        self.id
    }
}

impl Drop for NotifierSubscription {
    fn drop(&mut self) {
        // Torn down while the thread is exiting, the list may be gone already.
        let _ = HANDLERS.try_with(|handlers| {
            if let Ok(mut handlers) = handlers.try_borrow_mut() {
                handlers.retain(|(id, _)| *id != self.id);
            }
        });
    }
}

/// Registers `handler` for the events posted under a fresh [`NotifierId`].
///
/// [`install`] does this for you. Call it directly to feed events yourself
/// through [`post`], which is how a test drives a handler without a platform.
pub fn subscribe(handler: impl Fn(NotificationEvent) + 'static) -> NotifierSubscription {
    let id = NotifierId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
    HANDLERS.with_borrow_mut(|handlers| handlers.push((id, Rc::new(handler))));
    NotifierSubscription { id, _not_send: PhantomData }
}

/// Hands `event` of notifier `id` to the event loop, from any thread.
///
/// The loop calls the handler of `id` on the UI thread. Nothing happens if the
/// loop is not running yet or has ended; a notification is only answered once
/// the application is up, so there is nothing worth queueing.
pub fn post(id: NotifierId, event: NotificationEvent) {
    if let Some(proxy) = EVENT_PROXY.get() {
        let _ = proxy.send_event(AimerNativePlatformEvent::Notification { id, event });
    }
}

/// Runs the handler of notifier `id`. Returns whether there was one.
///
/// The event loop calls this on the UI thread when it receives
/// [`AimerNativePlatformEvent::Notification`].
pub fn dispatch(id: NotifierId, event: NotificationEvent) -> bool {
    // The list is released before the handler runs, so the handler may
    // subscribe, unsubscribe or drop its own notifier.
    let handler = HANDLERS.with_borrow(|handlers| {
        handlers.iter().find(|(held, _)| *held == id).map(|(_, handler)| Rc::clone(handler))
    });
    match handler {
        Some(handler) => {
            handler(event);
            true
        }
        None => false,
    }
}

/// A connection to the notification service whose events reach the event loop.
///
/// It dereferences to [`Notifier`], so notifications can be shown and closed at
/// any time. Dropping it disconnects and removes the handler.
///
/// It stays on the thread that installed it, the UI thread.
pub struct AppNotifier {
    // The notifier goes first: no event can arrive for a handler already removed.
    notifier: Notifier,
    subscription: NotifierSubscription,
}

impl AppNotifier {
    /// The id the loop reports this notifier's events under.
    #[inline]
    pub fn id(&self) -> NotifierId {
        self.subscription.id()
    }
}

impl Deref for AppNotifier {
    type Target = Notifier;

    #[inline]
    fn deref(&self) -> &Notifier {
        &self.notifier
    }
}

/// Connects to the platform's notification service and calls `handler` on the
/// UI thread for each of its events.
///
/// Call it on the UI thread, which on macOS and iOS is the main thread.
///
/// # Errors
///
/// See [`NotifierBuilder::build`].
pub fn install(handler: impl Fn(NotificationEvent) + 'static) -> Result<AppNotifier, NotificationError> {
    let subscription = subscribe(handler);
    let id = subscription.id();
    let notifier = Notifier::new().on_event(move |event| post(id, event)).build()?;
    Ok(AppNotifier { notifier, subscription })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn dispatch_runs_the_handler_of_that_notifier_only() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let (a, b) = (Rc::clone(&first), Rc::clone(&second));
        let one = subscribe(move |_| a.set(a.get() + 1));
        let _two = subscribe(move |_| b.set(b.get() + 1));

        assert!(dispatch(one.id(), NotificationEvent::Clicked(NotificationId(1))));
        assert_eq!((first.get(), second.get()), (1, 0));
    }

    #[test]
    fn the_handler_receives_the_event() {
        let seen = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let sub = subscribe(move |event| *sink.borrow_mut() = Some(event));

        let event = NotificationEvent::Action {
            notification: NotificationId(4),
            action: NotificationActionId(2),
        };
        dispatch(sub.id(), event.clone());
        assert_eq!(*seen.borrow(), Some(event));
    }

    #[test]
    fn dropping_the_subscription_removes_the_handler() {
        let sub = subscribe(|_| {});
        let id = sub.id();
        drop(sub);
        assert!(!dispatch(id, NotificationEvent::Dismissed(NotificationId(1))));
    }

    #[test]
    fn unknown_ids_are_ignored() {
        assert!(!dispatch(
            NotifierId::from_raw(u32::MAX),
            NotificationEvent::Clicked(NotificationId(1))
        ));
    }

    #[test]
    fn a_handler_may_drop_its_own_subscription() {
        let slot: Rc<RefCell<Option<NotifierSubscription>>> = Rc::default();
        let inner = Rc::clone(&slot);
        let sub = subscribe(move |_| drop(inner.borrow_mut().take()));
        let id = sub.id();
        *slot.borrow_mut() = Some(sub);

        assert!(dispatch(id, NotificationEvent::Clicked(NotificationId(1))));
        assert!(!dispatch(id, NotificationEvent::Clicked(NotificationId(1))));
    }

    #[test]
    fn ids_are_distinct() {
        let (a, b) = (subscribe(|_| {}), subscribe(|_| {}));
        assert_ne!(a.id(), b.id());
    }
}
