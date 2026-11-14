//! Files the app is asked to open: dropped on its icon, chosen with "Open With",
//! double-clicked in a file manager, or named on its command line.
//!
//! The platform hands the app the paths; what opening one means belongs to the
//! app. So the paths are offered to registered [`FileHandler`]s, **most recently
//! registered first** (the same order as [`crate::link`] and [`crate::back`]),
//! and the first to take them keeps them. A text editor registers one that opens
//! a tab per path.
//!
//! The file that *launches* the app arrives before the widget tree, and so any
//! handler, exists. Paths nobody could take are therefore kept, in the order they
//! arrived, for a handler's owner to [claim](claim_waiting_files) once it is
//! ready.
//!
//! This is not the drag of a file *over a window* — that is
//! [`ElementEvent::DroppedFile`](crate::element::ElementEvent::DroppedFile), which
//! is positional and goes to the widget under the pointer. These are requests to
//! the application as a whole, and have no position.
//!
//! Handlers live on the UI thread, which is also where paths must be delivered,
//! so the registry is per thread and holds no locks.
//!
//! # Examples
//!
//! ```
//! use std::cell::RefCell;
//! use std::path::PathBuf;
//! use std::rc::Rc;
//!
//! use aimer_events::file_open::{claim_waiting_files, deliver_files, register};
//!
//! // The app was launched by a file: it arrives before anything can take it.
//! assert!(!deliver_files(&[PathBuf::from("/work/main.rs")]));
//!
//! // Whatever builds the first screen claims it...
//! let mut launch = Vec::new();
//! claim_waiting_files(|paths| {
//!     launch.extend_from_slice(paths);
//!     true
//! });
//! assert_eq!(launch, [PathBuf::from("/work/main.rs")]);
//!
//! // ...and later files go to the registered handlers.
//! let opened = Rc::new(RefCell::new(Vec::new()));
//! let sink = opened.clone();
//! let _registration = register(move |paths: &[PathBuf]| {
//!     sink.borrow_mut().extend_from_slice(paths);
//!     true
//! });
//! assert!(deliver_files(&[PathBuf::from("/work/lib.rs")]));
//! assert_eq!(*opened.borrow(), [PathBuf::from("/work/lib.rs")]);
//! ```

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// The most paths kept for a handler that has not registered yet. A person
/// cannot open more than this before the first screen exists; the oldest go.
pub const MAX_WAITING: usize = 64;

/// Something that can open files.
pub trait FileHandler {
    /// Opens `paths` if this handler can, returning whether it did. Returning
    /// `false` passes the paths outward and must leave no trace.
    ///
    /// The paths are as the platform gave them: they may name a directory, and may
    /// no longer exist by the time they are read.
    fn on_open_files(&self, paths: &[PathBuf]) -> bool;
}

/// A closure is a handler.
impl<F: Fn(&[PathBuf]) -> bool> FileHandler for F {
    fn on_open_files(&self, paths: &[PathBuf]) -> bool {
        self(paths)
    }
}

aimer_tls::thread_local! {
    static HANDLERS: RefCell<Vec<(u64, Rc<dyn FileHandler>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The paths no handler has taken yet, oldest first.
    static WAITING: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Keeps a handler registered for as long as it lives.
#[must_use = "a handler is unregistered the moment its guard is dropped"]
pub struct FileGuard(u64);

impl Drop for FileGuard {
    fn drop(&mut self) {
        // `try_with`: dropped while the thread shuts down, the registry is gone.
        let _ = HANDLERS.try_with(|handlers| handlers.borrow_mut().retain(|(id, _)| *id != self.0));
    }
}

/// Registers `handler` ahead of every handler registered before it.
pub fn register(handler: impl FileHandler + 'static) -> FileGuard {
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    HANDLERS.with(|handlers| handlers.borrow_mut().push((id, Rc::new(handler))));
    FileGuard(id)
}

/// Offers the paths that are waiting, if any, to `accept`, and removes them if
/// `accept` returns `true`.
///
/// This is how the file that *launched* the app is picked up: it arrived before
/// the widget tree existed, so nothing could open it then. The owner of a handler
/// claims it as a separate step once that handler is ready, rather than having it
/// pushed into a handler that is still being constructed.
pub fn claim_waiting_files(accept: impl FnOnce(&[PathBuf]) -> bool) -> bool {
    let paths = WAITING.with(|waiting| waiting.borrow().clone());
    if paths.is_empty() || !accept(&paths) {
        return false;
    }
    // Whatever arrived while `accept` ran is not what it was shown.
    WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        let taken = paths.len().min(waiting.len());
        waiting.drain(..taken);
    });
    true
}

/// Offers `paths` to the handlers, innermost first, and returns whether one
/// opened them. Paths nobody opened are kept for the next handler to claim.
/// An empty list is nothing to open and is ignored.
pub fn deliver_files(paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
        return false;
    }
    // A snapshot, so a handler may register or unregister while it is offered
    // the paths.
    let handlers: Vec<_> =
        HANDLERS.with(|handlers| handlers.borrow().iter().rev().map(|(_, handler)| handler.clone()).collect());
    let opened = handlers.iter().any(|handler| handler.on_open_files(paths));
    if !opened {
        WAITING.with(|waiting| {
            let mut waiting = waiting.borrow_mut();
            waiting.extend_from_slice(paths);
            let excess = waiting.len().saturating_sub(MAX_WAITING);
            waiting.drain(..excess);
        });
    }
    opened
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use super::*;

    type Log = Rc<RefCell<Vec<String>>>;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn probe(name: &'static str, log: &Log, accepts: bool) -> impl Fn(&[PathBuf]) -> bool + 'static {
        let log = log.clone();
        move |paths: &[PathBuf]| {
            let shown: Vec<_> = paths.iter().map(|p| p.display().to_string()).collect();
            log.borrow_mut().push(format!("{name} {}", shown.join(",")));
            accepts
        }
    }

    #[test]
    fn the_innermost_handler_is_asked_first_and_the_first_yes_wins() {
        let log = Log::default();
        let _outer = register(probe("outer", &log, true));
        let _inner = register(probe("inner", &log, true));
        assert!(deliver_files(&paths(&["/a.rs"])));
        assert_eq!(*log.borrow(), ["inner /a.rs"]);
    }

    #[test]
    fn a_handler_that_cannot_open_the_files_passes_them_outward() {
        let log = Log::default();
        let _outer = register(probe("outer", &log, true));
        let _inner = register(probe("inner", &log, false));
        assert!(deliver_files(&paths(&["/a.rs", "/b.rs"])));
        assert_eq!(*log.borrow(), ["inner /a.rs,/b.rs", "outer /a.rs,/b.rs"], "all the paths, in one call");
    }

    #[test]
    fn files_nobody_opened_wait_in_arrival_order_to_be_claimed_once() {
        // The app was launched by the files, so they land before any handler exists.
        assert!(!deliver_files(&paths(&["/one.rs"])));
        assert!(!deliver_files(&paths(&["/two.rs", "/three.rs"])));
        let mut seen = Vec::new();
        assert!(claim_waiting_files(|paths| {
            seen.extend_from_slice(paths);
            true
        }));
        assert_eq!(seen, paths(&["/one.rs", "/two.rs", "/three.rs"]));
        assert!(!claim_waiting_files(|_| true), "they were claimed");
    }

    #[test]
    fn a_claim_that_declines_leaves_the_files_waiting() {
        deliver_files(&paths(&["/stubborn.rs"]));
        assert!(!claim_waiting_files(|_| false));
        assert!(claim_waiting_files(|paths| paths == [Path::new("/stubborn.rs")]));
    }

    #[test]
    fn nothing_is_waiting_when_nothing_arrived() {
        assert!(!claim_waiting_files(|_| panic!("there is nothing to offer")));
        assert!(!deliver_files(&[]), "an empty list is not a request");
        assert!(!claim_waiting_files(|_| panic!("an empty list leaves nothing behind")));
    }

    #[test]
    fn a_flood_keeps_the_newest_files_and_never_grows_without_bound() {
        for n in 0..(MAX_WAITING + 10) {
            deliver_files(&[PathBuf::from(format!("/f{n}.rs"))]);
        }
        let mut seen = Vec::new();
        claim_waiting_files(|paths| {
            seen.extend_from_slice(paths);
            true
        });
        assert_eq!(seen.len(), MAX_WAITING);
        assert_eq!(seen.last(), Some(&PathBuf::from(format!("/f{}.rs", MAX_WAITING + 9))));
        assert_eq!(seen.first(), Some(&PathBuf::from("/f10.rs")), "the oldest ten were dropped");
    }

    #[test]
    fn registering_does_not_replay_waiting_files_into_the_handler() {
        // Replaying inside `register` would call the handler while its owner is
        // still being constructed; claiming is the owner's explicit step.
        deliver_files(&paths(&["/waiting.rs"]));
        let log = Log::default();
        let _handler = register(probe("h", &log, true));
        assert!(log.borrow().is_empty());
        let _ = claim_waiting_files(|_| true);
    }

    #[test]
    fn dropping_the_guard_unregisters() {
        let log = Log::default();
        let guard = register(probe("gone", &log, true));
        drop(guard);
        assert!(!deliver_files(&paths(&["/x.rs"])));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn a_handler_may_unregister_itself_while_it_is_being_offered_files() {
        let slot: Rc<RefCell<Option<FileGuard>>> = Rc::default();
        let inner = slot.clone();
        *slot.borrow_mut() = Some(register(move |_: &[PathBuf]| {
            inner.borrow_mut().take();
            true
        }));
        assert!(deliver_files(&paths(&["/once.rs"])));
        assert!(!deliver_files(&paths(&["/twice.rs"])));
    }
}
