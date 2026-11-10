//! The user asking a running application to show itself again.
//!
//! On macOS that is a click on the application's Dock icon, or opening it again
//! from Finder, while it is already running. An application that stays alive
//! with no window — a menu-bar application, or one with
//! `ExitPolicy::Never` — is expected to put a window back when that
//! happens, and winit does not report it. [`install`] teaches the native
//! application to, and [`subscribe`] hears it on the UI thread.
//!
//! Other platforms have no such request, so nothing is ever delivered there.
//!
//! # Examples
//!
//! ```
//! use aimer_platform::reopen;
//!
//! let subscription = reopen::subscribe(|has_visible_windows| {
//!     if !has_visible_windows {
//!         // Open a window again.
//!     }
//! });
//! // Heard until `subscription` is dropped.
//! drop(subscription);
//! ```

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::{AimerNativePlatformEvent, EVENT_PROXY};

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

type Handler = Rc<dyn Fn(bool)>;

aimer_tls::thread_local! {
    /// The handlers of this thread, the UI thread, in subscription order.
    static HANDLERS: RefCell<Vec<(u32, Handler)>> = const { RefCell::new(Vec::new()) };
}

/// A handler waiting for reopen requests. Dropping it unsubscribes.
///
/// It stays on the thread that created it, the UI thread, because the handler
/// is not required to be `Send`.
#[must_use = "the handler is removed as soon as the subscription is dropped"]
pub struct ReopenSubscription {
    id: u32,
    _not_send: PhantomData<*const ()>,
}

impl Drop for ReopenSubscription {
    fn drop(&mut self) {
        // Torn down while the thread is exiting, the list may be gone already.
        let _ = HANDLERS.try_with(|handlers| {
            if let Ok(mut handlers) = handlers.try_borrow_mut() {
                handlers.retain(|(id, _)| *id != self.id);
            }
        });
    }
}

/// Registers `handler` for reopen requests.
///
/// The event loop calls it on the UI thread with whether the platform still
/// sees a window on screen. `false` is the case that matters: the application
/// is running and the user can see nothing of it.
pub fn subscribe(handler: impl Fn(bool) + 'static) -> ReopenSubscription {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    HANDLERS.with_borrow_mut(|handlers| handlers.push((id, Rc::new(handler))));
    ReopenSubscription { id, _not_send: PhantomData }
}

/// Hands a reopen request to the event loop, from any thread.
///
/// Nothing happens if the loop is not running yet or has ended.
pub fn post(has_visible_windows: bool) {
    if let Some(proxy) = EVENT_PROXY.get() {
        let _ = proxy.send_event(AimerNativePlatformEvent::Reopen { has_visible_windows });
    }
}

/// Runs every subscribed handler. Returns whether there was one.
///
/// The event loop calls this on the UI thread when it receives
/// [`AimerNativePlatformEvent::Reopen`].
pub fn dispatch(has_visible_windows: bool) -> bool {
    // The list is released before the handlers run, so a handler may subscribe
    // or unsubscribe.
    let handlers: Vec<Handler> =
        HANDLERS.with_borrow(|handlers| handlers.iter().map(|(_, handler)| Rc::clone(handler)).collect());
    for handler in &handlers {
        handler(has_visible_windows);
    }
    !handlers.is_empty()
}

/// Makes the native application report reopen requests through [`post`].
///
/// Call it once on the main thread after the native application exists; calling
/// it again, or on a platform without the request, does nothing. winit's
/// application delegate answers the other application messages and has no
/// reopen one, so the method is added to it here; a delegate that already has
/// one is left alone.
#[cfg(target_os = "macos")]
pub fn install() {
    use objc::runtime::{BOOL, Object, Sel, YES, class_addMethod};
    use objc::{Encode, class, msg_send, sel, sel_impl};

    extern "C" fn should_handle_reopen(
        _this: &Object,
        _selector: Sel,
        _sender: *mut Object,
        has_visible_windows: BOOL,
    ) -> BOOL {
        post(has_visible_windows != objc::runtime::NO);
        // The system's own handling — bringing back a minimized window — goes on.
        YES
    }

    // SAFETY: called on the main thread, where `NSApplication` may be used.
    // `class_addMethod` is given a function whose signature matches the
    // message — object, selector, object, BOOL, returning BOOL — and a
    // NUL-terminated type encoding. Adding to a class that already answers the
    // selector fails and changes nothing.
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut Object = msg_send![app, delegate];
        if delegate.is_null() {
            return;
        }
        let class = (*delegate).class() as *const _ as *mut objc::runtime::Class;
        let boolean = BOOL::encode();
        let types = format!("{0}@:@{0}\0", boolean.as_str());
        let implementation: objc::runtime::Imp = std::mem::transmute(
            should_handle_reopen as extern "C" fn(&Object, Sel, *mut Object, BOOL) -> BOOL,
        );
        let added = class_addMethod(
            class,
            sel!(applicationShouldHandleReopen:hasVisibleWindows:),
            implementation,
            types.as_ptr().cast(),
        );
        if added != objc::runtime::NO {
            // AppKit may have looked at what its delegate answers when it was
            // set, so it is set again to be asked afresh.
            let _: () = msg_send![app, setDelegate: delegate];
        }
    }
}

/// Makes the native application report reopen requests. This platform has none.
#[cfg(not(target_os = "macos"))]
pub fn install() {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn a_reopen_reaches_every_subscriber_with_what_the_platform_saw() {
        let first = Rc::new(Cell::new(None));
        let second = Rc::new(Cell::new(None));
        let (a, b) = (Rc::clone(&first), Rc::clone(&second));
        let _first = subscribe(move |visible| a.set(Some(visible)));
        let _second = subscribe(move |visible| b.set(Some(visible)));

        assert!(dispatch(false));

        assert_eq!(first.get(), Some(false));
        assert_eq!(second.get(), Some(false));
    }

    #[test]
    fn a_dropped_subscription_hears_nothing() {
        let seen = Rc::new(Cell::new(0));
        let sink = Rc::clone(&seen);
        let subscription = subscribe(move |_| sink.set(sink.get() + 1));
        dispatch(true);
        assert_eq!(seen.get(), 1);

        drop(subscription);

        assert!(!dispatch(true), "nobody is left to take it");
        assert_eq!(seen.get(), 1);
    }

    #[test]
    fn a_handler_may_unsubscribe_while_it_runs() {
        let slot = Rc::new(RefCell::new(None));
        let held = Rc::clone(&slot);
        let subscription = subscribe(move |_| drop(held.borrow_mut().take()));
        *slot.borrow_mut() = Some(subscription);

        assert!(dispatch(false));
        assert!(!dispatch(false));
    }
}
