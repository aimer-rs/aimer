//! The platform's "go back" request, and who gets to answer it.
//!
//! Android's Back key and gesture, a desktop's back shortcut and a browser's
//! back button all ask the same thing of the app: undo the last step. What that
//! means belongs to whatever the user is looking at, so the request is offered
//! to registered [`BackHandler`]s, **most recently registered first**. A dialog
//! that is open registers after the page beneath it and so answers first; a
//! nested navigator answers before the one around it, and passes the request
//! outward when it has nothing left to pop.
//!
//! A request nobody takes is left to the platform: on Android that means
//! leaving the app, which winit does not do on its own.
//!
//! Handlers live on the UI thread, which is also where back requests must be
//! delivered. The registry is therefore per thread and holds no locks.
//!
//! # Examples
//!
//! ```
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! use aimer_events::back::{BackHandler, register, request_back};
//!
//! struct Dialog(Rc<Cell<bool>>);
//! impl BackHandler for Dialog {
//!     fn on_back(&self) -> bool {
//!         self.0.set(false); // close it
//!         true
//!     }
//! }
//!
//! let open = Rc::new(Cell::new(true));
//! let registration = register(Dialog(open.clone()));
//! assert!(request_back());
//! assert!(!open.get());
//!
//! // Once the dialog is gone it no longer answers.
//! drop(registration);
//! assert!(!request_back());
//! ```

use std::cell::RefCell;
use std::rc::Rc;

/// Something that can answer a back request.
pub trait BackHandler {
    /// Takes a plain back request, returning whether it did anything.
    ///
    /// Returning `false` passes the request to the next handler outward and
    /// must leave no trace.
    fn on_back(&self) -> bool;

    /// Starts a back that a finger or the system drives, such as Android's
    /// predictive back, or `None` if this handler can only be asked plainly.
    ///
    /// Only a handler that returns a gesture owns the gesture. A handler that
    /// returns `None` is skipped, and the request goes outward.
    fn on_back_started(&self) -> Option<Box<dyn BackGesture>> {
        None
    }
}

/// A back in progress under the system's control.
pub trait BackGesture {
    /// How far the gesture has gone, from `0.0` to `1.0`.
    fn progress(&self, progress: f32);
    /// The gesture completed: go back.
    fn commit(self: Box<Self>);
    /// The gesture was abandoned: stay.
    fn cancel(self: Box<Self>);
}

thread_local! {
    static HANDLERS: RefCell<Vec<(u64, Rc<dyn BackHandler>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Keeps a handler registered for as long as it lives.
#[must_use = "a handler is unregistered the moment its guard is dropped"]
pub struct BackGuard(u64);

impl Drop for BackGuard {
    fn drop(&mut self) {
        // `try_with`: a guard dropped while the thread is shutting down finds the
        // registry already gone, and there is nothing left to unregister from.
        let _ = HANDLERS.try_with(|handlers| handlers.borrow_mut().retain(|(id, _)| *id != self.0));
    }
}

/// Registers `handler` ahead of every handler registered before it.
pub fn register(handler: impl BackHandler + 'static) -> BackGuard {
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    HANDLERS.with(|handlers| handlers.borrow_mut().push((id, Rc::new(handler))));
    BackGuard(id)
}

/// A snapshot, innermost first, so a handler may register or unregister
/// (including itself) while it is being asked.
fn innermost_first() -> Vec<Rc<dyn BackHandler>> {
    HANDLERS.with(|handlers| handlers.borrow().iter().rev().map(|(_, handler)| handler.clone()).collect())
}

/// Offers a plain back request to the handlers, innermost first, and returns
/// whether one took it. When none did, the platform's default is the caller's.
pub fn request_back() -> bool {
    innermost_first().iter().any(|handler| handler.on_back())
}

/// Starts a gesture-driven back with the innermost handler that offers one.
pub fn start_back_gesture() -> Option<Box<dyn BackGesture>> {
    innermost_first().iter().find_map(|handler| handler.on_back_started())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// Records what it was asked, answers as told.
    struct Probe {
        name: &'static str,
        log: Rc<RefCell<Vec<String>>>,
        handles: bool,
        gesture: bool,
    }

    impl BackHandler for Probe {
        fn on_back(&self) -> bool {
            self.log.borrow_mut().push(format!("{} back", self.name));
            self.handles
        }

        fn on_back_started(&self) -> Option<Box<dyn BackGesture>> {
            self.log.borrow_mut().push(format!("{} started", self.name));
            self.gesture.then(|| {
                Box::new(ProbeGesture { name: self.name, log: self.log.clone() }) as Box<dyn BackGesture>
            })
        }
    }

    struct ProbeGesture {
        name: &'static str,
        log: Rc<RefCell<Vec<String>>>,
    }

    impl BackGesture for ProbeGesture {
        fn progress(&self, progress: f32) {
            self.log.borrow_mut().push(format!("{} progress {progress}", self.name));
        }
        fn commit(self: Box<Self>) {
            self.log.borrow_mut().push(format!("{} commit", self.name));
        }
        fn cancel(self: Box<Self>) {
            self.log.borrow_mut().push(format!("{} cancel", self.name));
        }
    }

    fn probe(name: &'static str, log: &Rc<RefCell<Vec<String>>>, handles: bool, gesture: bool) -> Probe {
        Probe { name, log: log.clone(), handles, gesture }
    }

    fn log() -> Rc<RefCell<Vec<String>>> {
        Rc::default()
    }

    #[test]
    fn nothing_registered_means_nothing_handled() {
        assert!(!request_back());
        assert!(start_back_gesture().is_none());
    }

    #[test]
    fn the_most_recently_registered_handler_is_asked_first_and_the_first_yes_wins() {
        let log = log();
        let _outer = register(probe("outer", &log, true, false));
        let _inner = register(probe("inner", &log, true, false));
        assert!(request_back());
        assert_eq!(*log.borrow(), ["inner back"], "the outer one is never asked");
    }

    #[test]
    fn a_handler_that_declines_passes_the_request_outward() {
        let log = log();
        let _outer = register(probe("outer", &log, true, false));
        let _inner = register(probe("inner", &log, false, false));
        assert!(request_back());
        assert_eq!(*log.borrow(), ["inner back", "outer back"]);
    }

    #[test]
    fn when_every_handler_declines_the_request_is_unhandled() {
        let log = log();
        let _a = register(probe("a", &log, false, false));
        let _b = register(probe("b", &log, false, false));
        assert!(!request_back(), "the platform default (leaving the app) is the caller's");
    }

    #[test]
    fn dropping_the_guard_unregisters() {
        let log = log();
        let _outer = register(probe("outer", &log, true, false));
        let inner = register(probe("inner", &log, true, false));
        drop(inner);
        assert!(request_back());
        assert_eq!(*log.borrow(), ["outer back"]);
    }

    #[test]
    fn a_gesture_belongs_to_the_innermost_handler_that_can_be_driven_by_one() {
        let log = log();
        let _outer = register(probe("outer", &log, true, true));
        let _inner = register(probe("inner", &log, true, false));
        let gesture = start_back_gesture().expect("the outer handler offers one");
        assert_eq!(*log.borrow(), ["inner started", "outer started"]);
        gesture.progress(0.5);
        gesture.commit();
        assert_eq!(log.borrow()[2..], ["outer progress 0.5", "outer commit"]);
    }

    #[test]
    fn a_handler_may_unregister_itself_while_it_is_being_asked() {
        let log = log();
        let slot: Rc<RefCell<Option<BackGuard>>> = Rc::default();
        struct OneShot(Rc<RefCell<Option<BackGuard>>>, Rc<Cell<u32>>);
        impl BackHandler for OneShot {
            fn on_back(&self) -> bool {
                self.1.set(self.1.get() + 1);
                self.0.borrow_mut().take(); // drops its own registration mid-call
                true
            }
        }
        let calls = Rc::new(Cell::new(0));
        *slot.borrow_mut() = Some(register(OneShot(slot.clone(), calls.clone())));
        let _other = register(probe("other", &log, false, false));
        assert!(request_back());
        assert_eq!(calls.get(), 1);
        assert!(!request_back(), "it is gone now");
    }
}
