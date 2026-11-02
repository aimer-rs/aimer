//! Links that open the app at a particular place: a custom-scheme URL, a
//! universal link, an Android `VIEW` intent.
//!
//! The platform hands the app a URL; what it means belongs to whatever knows the
//! app's routes. So the URL is offered to registered [`LinkHandler`]s, **most
//! recently registered first** (the same order as [`crate::back`]), and the
//! first to recognise it takes it. A navigator registers one that turns the URL
//! into a route.
//!
//! The URL that *launches* the app arrives before the widget tree, and so any
//! handler, exists. A link nobody could take is therefore kept, the latest one
//! only, for a handler's owner to [claim](claim_waiting_link) once it is ready.
//!
//! Handlers live on the UI thread, which is also where links must be
//! delivered, so the registry is per thread and holds no locks.
//!
//! # Examples
//!
//! ```
//! use std::cell::RefCell;
//! use std::rc::Rc;
//!
//! use aimer_events::link::{LinkHandler, claim_waiting_link, deliver_link, register};
//!
//! struct Recorder(Rc<RefCell<Vec<String>>>);
//! impl LinkHandler for Recorder {
//!     fn on_link(&self, url: &str) -> bool {
//!         self.0.borrow_mut().push(url.to_owned());
//!         true
//!     }
//! }
//!
//! // The app was launched by a link: it arrives before anything can take it.
//! assert!(!deliver_link("myapp://topic/42"));
//!
//! // Whatever opens the first screen claims it...
//! let mut launch = None;
//! claim_waiting_link(|url| {
//!     launch = Some(url.to_owned());
//!     true
//! });
//! assert_eq!(launch.as_deref(), Some("myapp://topic/42"));
//!
//! // ...and later links go to the registered handlers.
//! let seen = Rc::new(RefCell::new(Vec::new()));
//! let _registration = register(Recorder(seen.clone()));
//! assert!(deliver_link("myapp://topic/7"));
//! assert_eq!(*seen.borrow(), ["myapp://topic/7"]);
//! ```

use std::cell::RefCell;
use std::rc::Rc;

/// Something that can open a link.
pub trait LinkHandler {
    /// Opens `url` if it is one this handler understands, returning whether it
    /// did. Returning `false` passes the link outward and must leave no trace.
    fn on_link(&self, url: &str) -> bool;
}

aimer_tls::thread_local! {
    static HANDLERS: RefCell<Vec<(u64, Rc<dyn LinkHandler>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The latest link no handler has taken yet.
    static WAITING: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Keeps a handler registered for as long as it lives.
#[must_use = "a handler is unregistered the moment its guard is dropped"]
pub struct LinkGuard(u64);

impl Drop for LinkGuard {
    fn drop(&mut self) {
        // `try_with`: dropped while the thread shuts down, the registry is gone.
        let _ = HANDLERS.try_with(|handlers| handlers.borrow_mut().retain(|(id, _)| *id != self.0));
    }
}

/// Registers `handler` ahead of every handler registered before it.
pub fn register(handler: impl LinkHandler + 'static) -> LinkGuard {
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    HANDLERS.with(|handlers| handlers.borrow_mut().push((id, Rc::new(handler))));
    LinkGuard(id)
}

/// Offers the link that is waiting, if any, to `accept`, and removes it if
/// `accept` returns `true`.
///
/// This is how the link that *launched* the app is picked up: it arrived before
/// the widget tree existed, so nothing could open it then. The owner of a
/// handler claims it as a separate step once that handler is ready, rather than
/// having it pushed into a handler that is still being constructed.
pub fn claim_waiting_link(accept: impl FnOnce(&str) -> bool) -> bool {
    let url = WAITING.with(|waiting| waiting.borrow().clone());
    match url {
        Some(url) if accept(&url) => {
            WAITING.with(|waiting| *waiting.borrow_mut() = None);
            true
        }
        _ => false,
    }
}

/// Offers `url` to the handlers, innermost first, and returns whether one
/// opened it. A link nobody opened is kept for the next handler to register.
pub fn deliver_link(url: &str) -> bool {
    // A snapshot, so a handler may register or unregister while it is offered
    // the link.
    let handlers: Vec<_> =
        HANDLERS.with(|handlers| handlers.borrow().iter().rev().map(|(_, handler)| handler.clone()).collect());
    let opened = handlers.iter().any(|handler| handler.on_link(url));
    // Whatever was waiting is older than this, and the user's latest intent is
    // the one that counts, whether it was served now or is still waiting.
    WAITING.with(|waiting| *waiting.borrow_mut() = (!opened).then(|| url.to_owned()));
    opened
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    type Log = Rc<RefCell<Vec<String>>>;

    struct Probe {
        name: &'static str,
        log: Log,
        accepts: bool,
    }

    impl LinkHandler for Probe {
        fn on_link(&self, url: &str) -> bool {
            self.log.borrow_mut().push(format!("{} {url}", self.name));
            self.accepts
        }
    }

    fn probe(name: &'static str, log: &Log, accepts: bool) -> Probe {
        Probe { name, log: log.clone(), accepts }
    }

    #[test]
    fn the_innermost_handler_is_asked_first_and_the_first_yes_wins() {
        let log = Log::default();
        let _outer = register(probe("outer", &log, true));
        let _inner = register(probe("inner", &log, true));
        assert!(deliver_link("app://a"));
        assert_eq!(*log.borrow(), ["inner app://a"]);
    }

    #[test]
    fn a_handler_that_does_not_recognise_the_link_passes_it_outward() {
        let log = Log::default();
        let _outer = register(probe("outer", &log, true));
        let _inner = register(probe("inner", &log, false));
        assert!(deliver_link("app://a"));
        assert_eq!(*log.borrow(), ["inner app://a", "outer app://a"]);
    }

    #[test]
    fn a_link_nobody_opened_waits_to_be_claimed_once() {
        // The app was launched by the link, so it lands before any navigator exists.
        assert!(!deliver_link("app://cold"), "nobody to take it yet");
        let mut seen = None;
        assert!(claim_waiting_link(|url| {
            seen = Some(url.to_owned());
            true
        }));
        assert_eq!(seen.as_deref(), Some("app://cold"));
        assert!(!claim_waiting_link(|_| true), "it was claimed");
    }

    #[test]
    fn a_claim_that_declines_leaves_the_link_waiting_for_someone_who_understands_it() {
        deliver_link("app://stubborn");
        assert!(!claim_waiting_link(|_| false));
        assert!(claim_waiting_link(|url| url == "app://stubborn"));
    }

    #[test]
    fn only_the_latest_waiting_link_is_kept() {
        deliver_link("app://first");
        deliver_link("app://second");
        let mut seen = Vec::new();
        claim_waiting_link(|url| {
            seen.push(url.to_owned());
            true
        });
        assert_eq!(seen, ["app://second"], "a newer link supersedes one nobody opened");
    }

    #[test]
    fn a_link_that_is_opened_retires_an_older_one_still_waiting() {
        deliver_link("app://old");
        let _handler = register(probe("h", &Log::default(), true));
        assert!(deliver_link("app://new"));
        assert!(!claim_waiting_link(|_| true), "the user's latest intent was served");
    }

    #[test]
    fn registering_does_not_replay_a_waiting_link_into_the_handler() {
        // Replaying inside `register` would call the handler while its owner is
        // still being constructed; claiming is the owner's explicit step.
        deliver_link("app://waiting");
        let log = Log::default();
        let _handler = register(probe("h", &log, true));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn dropping_the_guard_unregisters() {
        let log = Log::default();
        let guard = register(probe("gone", &log, true));
        drop(guard);
        assert!(!deliver_link("app://x"));
        assert!(log.borrow().is_empty());
        // Leave nothing behind for the next test on this thread.
        let _sink = register(probe("sink", &Log::default(), true));
    }

    #[test]
    fn a_handler_may_unregister_itself_while_it_is_being_offered_a_link() {
        let slot: Rc<RefCell<Option<LinkGuard>>> = Rc::default();
        struct OneShot(Rc<RefCell<Option<LinkGuard>>>);
        impl LinkHandler for OneShot {
            fn on_link(&self, _: &str) -> bool {
                self.0.borrow_mut().take();
                true
            }
        }
        *slot.borrow_mut() = Some(register(OneShot(slot.clone())));
        assert!(deliver_link("app://once"));
        assert!(!deliver_link("app://twice"));
        let _sink = register(probe("sink", &Log::default(), true));
    }
}
