//! Changing the mouse cursor of the application window.
//!
//! A widget that wants the pointer to look different while it is hovered —
//! a link showing [`CursorIcon::Pointer`], a text run showing
//! [`CursorIcon::Text`], a splitter showing [`CursorIcon::ColResize`] — calls
//! [`set_cursor`]. The call goes to the window the application registered with
//! [`aimer_events::window::set_window`], so a widget never has to carry a
//! window handle around just to change the cursor.
//!
//! An application running without a platform window (a test, a headless
//! render) installs a handler of its own with
//! [`set_thread_cursor_handler`] and observes the requests instead.

use std::cell::RefCell;
use std::rc::Rc;

#[doc(no_inline)]
pub use winit::window::{Cursor, CursorIcon, CustomCursor};

/// A handler that answers cursor requests made on the thread it was installed
/// on.
type CursorHandler = Rc<dyn Fn(Cursor)>;

aimer_tls::thread_local! {
    /// The handler that answers for this thread alone.
    ///
    /// The platform window is one per process, which is exactly right for an
    /// application that owns the screen. An application running without one is
    /// not that: several of them can be alive at once, each on its own thread,
    /// and each with its own idea of what the cursor should be. Installing per
    /// thread keeps a cursor request with the application that made it — the
    /// same reasoning, and the same shape, as
    /// [`aimer_events::window::set_thread_redraw_requester`].
    static THREAD_CURSOR_HANDLER: RefCell<Option<CursorHandler>> = const { RefCell::new(None) };
}

/// Ask the window to show `cursor`.
///
/// Accepts anything that converts into a [`Cursor`]: a [`CursorIcon`] for one
/// of the platform's built-in shapes, or a [`CustomCursor`] built from an image
/// through the event loop.
///
/// Returns `true` when the request reached a window (or the handler installed
/// for this thread) and `false` when there is nothing to show a cursor on yet —
/// before the window exists, or on a platform that has no pointer at all. The
/// return value is a report, not an error: a widget updating the cursor on
/// hover can safely ignore it.
///
/// The cursor is window state, not element state: it stays as it was set until
/// somebody sets it again. Whoever changes it while hovered is responsible for
/// calling [`reset_cursor`] on the way out.
///
/// # Examples
///
/// Showing the hand cursor over a link:
///
/// ```
/// use aimer_utils::cursor::{CursorIcon, set_cursor};
///
/// # let hovering_a_link = true;
/// if hovering_a_link {
///     set_cursor(CursorIcon::Pointer);
/// }
/// ```
///
/// Observing the requests without a window, which is how the tests do it:
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_utils::cursor::{
///     Cursor, CursorIcon, restore_thread_cursor_handler, set_cursor, set_thread_cursor_handler,
/// };
///
/// let requested = Rc::new(Cell::new(None));
/// let recorded = requested.clone();
/// let previous = set_thread_cursor_handler(move |cursor| recorded.set(Some(cursor)));
///
/// assert!(set_cursor(CursorIcon::Text));
///
/// restore_thread_cursor_handler(previous);
/// assert_eq!(requested.take(), Some(Cursor::Icon(CursorIcon::Text)));
/// ```
#[inline]
pub fn set_cursor(cursor: impl Into<Cursor>) -> bool {
    #[cfg(aimer_portable_guest)]
    {
        let _ = cursor;
        return false;
    }

    #[cfg(not(aimer_portable_guest))]
    {
        // An application without a platform window answers for itself, and says so
        // per thread, so its request never reaches a window belonging to somebody
        // else.
        if let Some(handler) = thread_cursor_handler() {
            handler(cursor.into());
            return true;
        }

        match aimer_events::window::get_window() {
            Some(window) => {
                window.set_cursor(cursor.into());
                true
            }
            None => false,
        }
    }
}

/// Ask the window to show the platform's default cursor again.
///
/// Sugar for `set_cursor(CursorIcon::Default)`, and the counterpart of the
/// [`set_cursor`] call a widget makes when the pointer enters it.
///
/// # Examples
///
/// ```
/// use aimer_utils::cursor::reset_cursor;
///
/// # let pointer_left_the_widget = true;
/// if pointer_left_the_widget {
///     reset_cursor();
/// }
/// ```
#[inline]
pub fn reset_cursor() -> bool {
    set_cursor(CursorIcon::Default)
}

/// Install a cursor handler for the current thread, replacing any previous one,
/// and hand back the one it replaced.
///
/// Takes precedence over the global window, so an application that has no
/// platform window — a test, a headless render — still sees the cursor requests
/// its widgets make. Give the returned handler to
/// [`restore_thread_cursor_handler`] once that application is gone, so a
/// dropped application stops receiving them.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use std::cell::RefCell;
///
/// use aimer_utils::cursor::{
///     CursorIcon, restore_thread_cursor_handler, set_cursor, set_thread_cursor_handler,
/// };
///
/// let log = Rc::new(RefCell::new(Vec::new()));
/// let recorded = log.clone();
/// let previous = set_thread_cursor_handler(move |cursor| recorded.borrow_mut().push(cursor));
///
/// set_cursor(CursorIcon::Grab);
/// set_cursor(CursorIcon::Grabbing);
///
/// restore_thread_cursor_handler(previous);
/// assert_eq!(log.borrow().len(), 2);
/// ```
#[inline]
pub fn set_thread_cursor_handler<F>(handler: F) -> Option<Rc<dyn Fn(Cursor)>>
where
    F: Fn(Cursor) + 'static,
{
    THREAD_CURSOR_HANDLER.with(|slot| slot.borrow_mut().replace(Rc::new(handler)))
}

/// Put back the handler that [`set_thread_cursor_handler`] replaced.
///
/// Passing `None` leaves the thread with no handler, so cursor requests go back
/// to the global window.
#[inline]
pub fn restore_thread_cursor_handler(previous: Option<Rc<dyn Fn(Cursor)>>) {
    THREAD_CURSOR_HANDLER.with(|slot| *slot.borrow_mut() = previous);
}

/// The handler installed for this thread, if any.
#[inline]
fn thread_cursor_handler() -> Option<CursorHandler> {
    THREAD_CURSOR_HANDLER.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn set_cursor_reports_failure_when_nothing_can_answer() {
        assert!(!set_cursor(CursorIcon::Pointer));
    }

    #[test]
    fn set_cursor_reaches_the_handler_installed_for_this_thread() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let recorded = seen.clone();
        let previous = set_thread_cursor_handler(move |cursor| {
            recorded.borrow_mut().push(cursor);
        });

        assert!(set_cursor(CursorIcon::Text));
        assert!(set_cursor(Cursor::Icon(CursorIcon::Grab)));

        restore_thread_cursor_handler(previous);

        assert_eq!(
            *seen.borrow(),
            vec![
                Cursor::Icon(CursorIcon::Text),
                Cursor::Icon(CursorIcon::Grab)
            ]
        );
    }

    #[test]
    fn restoring_the_handler_puts_the_previous_one_back() {
        let outer_hits = Rc::new(Cell::new(0));
        let outer = outer_hits.clone();
        let root = set_thread_cursor_handler(move |_| outer.set(outer.get() + 1));

        let inner_hits = Rc::new(Cell::new(0));
        let inner = inner_hits.clone();
        let replaced = set_thread_cursor_handler(move |_| inner.set(inner.get() + 1));

        assert!(set_cursor(CursorIcon::Default));
        restore_thread_cursor_handler(replaced);
        assert!(set_cursor(CursorIcon::Default));
        restore_thread_cursor_handler(root);

        assert_eq!(inner_hits.get(), 1);
        assert_eq!(outer_hits.get(), 1);
    }

    #[test]
    fn reset_cursor_asks_for_the_platform_default() {
        let seen = Rc::new(RefCell::new(None));
        let recorded = seen.clone();
        let previous = set_thread_cursor_handler(move |cursor| {
            *recorded.borrow_mut() = Some(cursor);
        });

        assert!(reset_cursor());

        restore_thread_cursor_handler(previous);

        assert_eq!(*seen.borrow(), Some(Cursor::Icon(CursorIcon::Default)));
    }

    #[cfg(aimer_portable_guest)]
    #[test]
    fn portable_cursor_requests_do_not_reach_winit() {
        assert!(!set_cursor(CursorIcon::Pointer));
        assert!(!reset_cursor());
    }
}
