//! The system tray, wired into the event loop.
//!
//! [`aimer_tray`] reports a click or a chosen menu item from whichever thread
//! heard it: the main thread on macOS, a thread of the tray's own on Windows and
//! Linux. This module makes that uniform for an application. [`install`] shows
//! the icon and routes its events to the event loop, which calls your handler on
//! the UI thread, where it can touch widget state like any other callback.
//!
//! Everything [`aimer_tray`] exports is re-exported here, so `Tray`, `TrayMenu`
//! and the rest are reachable from this one module.
//!
//! # Examples
//!
//! ```no_run
//! use aimer_platform::tray::{self, Tray, TrayEvent, TrayIcon, TrayItemId, TrayMenu};
//!
//! const QUIT: TrayItemId = TrayItemId(1);
//!
//! let icon = TrayIcon::from_rgba(2, 2, [40, 120, 255, 255].repeat(4)).unwrap();
//! let tray = tray::install(
//!     Tray::new().icon(icon).tooltip("My app").menu(TrayMenu::new().item(QUIT, "Quit")),
//!     |event| {
//!         if event == TrayEvent::MenuItem(QUIT) {
//!             // Runs inside the event loop.
//!         }
//!     },
//! )?;
//! // The icon stays until `tray` is dropped.
//! # drop(tray);
//! # Ok::<(), tray::TrayError>(())
//! ```

use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

pub use aimer_tray::*;

use crate::{AimerNativePlatformEvent, EVENT_PROXY};

/// Names one installed tray in the event the loop receives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TrayId(u32);

impl TrayId {
    /// The id as a number.
    #[inline]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// The id from [`TrayId::as_raw`].
    #[inline]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

type Handler = Rc<dyn Fn(TrayEvent)>;

aimer_tls::thread_local! {
    /// The handlers of this thread, the UI thread. A handful at most, so a
    /// vector is faster than a map.
    static HANDLERS: RefCell<Vec<(TrayId, Handler)>> = const { RefCell::new(Vec::new()) };
}

/// A handler waiting for the events of one tray. Dropping it unsubscribes.
///
/// It stays on the thread that created it, the UI thread, because the handler
/// is not required to be `Send`.
#[must_use = "the handler is removed as soon as the subscription is dropped"]
pub struct TraySubscription {
    id: TrayId,
    _not_send: PhantomData<*const ()>,
}

impl TraySubscription {
    /// The id events for this subscription carry.
    #[inline]
    pub fn id(&self) -> TrayId {
        self.id
    }
}

impl Drop for TraySubscription {
    fn drop(&mut self) {
        // Torn down while the thread is exiting, the list may be gone already.
        let _ = HANDLERS.try_with(|handlers| {
            if let Ok(mut handlers) = handlers.try_borrow_mut() {
                handlers.retain(|(id, _)| *id != self.id);
            }
        });
    }
}

/// Registers `handler` for the events posted under a fresh [`TrayId`].
///
/// [`install`] does this for you. Call it directly to feed events yourself
/// through [`post`], which is how a test drives a handler without a tray.
pub fn subscribe(handler: impl Fn(TrayEvent) + 'static) -> TraySubscription {
    let id = TrayId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
    HANDLERS.with_borrow_mut(|handlers| handlers.push((id, Rc::new(handler))));
    TraySubscription { id, _not_send: PhantomData }
}

/// Hands `event` of tray `id` to the event loop, from any thread.
///
/// The loop calls the handler of `id` on the UI thread. Nothing happens if the
/// loop is not running yet or has ended; a tray is only clicked once the
/// application is up, so there is nothing worth queueing.
pub fn post(id: TrayId, event: TrayEvent) {
    if let Some(proxy) = EVENT_PROXY.get() {
        let _ = proxy.send_event(AimerNativePlatformEvent::Tray { id, event });
    }
}

/// Runs the handler of tray `id`. Returns whether there was one.
///
/// The event loop calls this on the UI thread when it receives
/// [`AimerNativePlatformEvent::Tray`].
pub fn dispatch(id: TrayId, event: TrayEvent) -> bool {
    // The list is released before the handler runs, so the handler may
    // subscribe, unsubscribe or drop its own tray.
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

/// A tray icon on screen whose events reach the event loop.
///
/// It dereferences to [`TrayHandle`], so the picture, the tooltip and the menu
/// can be changed at any time. Dropping it removes the icon and the handler.
///
/// It stays on the thread that installed it: on macOS the status bar may only
/// be touched from the main thread.
pub struct AppTray {
    // The icon goes first: no event can arrive for a handler already removed.
    handle: TrayHandle,
    subscription: TraySubscription,
}

impl AppTray {
    /// The id the loop reports this tray's events under.
    #[inline]
    pub fn id(&self) -> TrayId {
        self.subscription.id()
    }
}

impl Deref for AppTray {
    type Target = TrayHandle;

    #[inline]
    fn deref(&self) -> &TrayHandle {
        &self.handle
    }
}

impl DerefMut for AppTray {
    #[inline]
    fn deref_mut(&mut self) -> &mut TrayHandle {
        &mut self.handle
    }
}

/// Shows `tray` and calls `handler` on the UI thread for each of its events.
///
/// Any handler already set with [`Tray::on_event`] is replaced. Call it on the
/// UI thread, which on macOS is the main thread.
///
/// # Errors
///
/// See [`Tray::build`].
pub fn install(tray: Tray, handler: impl Fn(TrayEvent) + 'static) -> Result<AppTray, TrayError> {
    let subscription = subscribe(handler);
    let id = subscription.id();
    let handle = tray.on_event(move |event| post(id, event)).build()?;
    Ok(AppTray { handle, subscription })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn dispatch_runs_the_handler_of_that_tray_only() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let (a, b) = (Rc::clone(&first), Rc::clone(&second));
        let one = subscribe(move |_| a.set(a.get() + 1));
        let _two = subscribe(move |_| b.set(b.get() + 1));

        assert!(dispatch(one.id(), TrayEvent::Click(TrayButton::Left)));
        assert_eq!((first.get(), second.get()), (1, 0));
    }

    #[test]
    fn the_handler_receives_the_event() {
        let seen = Rc::new(Cell::new(None));
        let sink = Rc::clone(&seen);
        let sub = subscribe(move |event| sink.set(Some(event)));

        dispatch(sub.id(), TrayEvent::MenuItem(TrayItemId(9)));
        assert_eq!(seen.get(), Some(TrayEvent::MenuItem(TrayItemId(9))));
    }

    #[test]
    fn dropping_the_subscription_removes_the_handler() {
        let sub = subscribe(|_| {});
        let id = sub.id();
        drop(sub);
        assert!(!dispatch(id, TrayEvent::Click(TrayButton::Right)));
    }

    #[test]
    fn unknown_ids_are_ignored() {
        assert!(!dispatch(TrayId::from_raw(u32::MAX), TrayEvent::Click(TrayButton::Left)));
    }

    #[test]
    fn a_handler_may_drop_its_own_subscription() {
        let slot: Rc<RefCell<Option<TraySubscription>>> = Rc::default();
        let inner = Rc::clone(&slot);
        let sub = subscribe(move |_| drop(inner.borrow_mut().take()));
        let id = sub.id();
        *slot.borrow_mut() = Some(sub);

        assert!(dispatch(id, TrayEvent::Click(TrayButton::Left)));
        assert!(!dispatch(id, TrayEvent::Click(TrayButton::Left)));
    }

    #[test]
    fn ids_are_distinct() {
        let (a, b) = (subscribe(|_| {}), subscribe(|_| {}));
        assert_ne!(a.id(), b.id());
    }
}
