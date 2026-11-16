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

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    note_set();
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
                let cursor = cursor.into();
                note_requested(&cursor);
                window.set_cursor(cursor);
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

aimer_tls::thread_local! {
    /// Who last claimed the cursor with [`claim`], or `0` for nobody.
    static CLAIMANT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The last number [`claim`] handed out on this thread.
    static LAST_CLAIM: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Records that `claimant` set the cursor, and returns the ticket that proves it.
///
/// A region that shows an icon while the pointer is over it must give the default
/// back when the pointer leaves, but only if nobody has set the cursor since: the
/// pointer may have moved straight onto a neighbour that asked for its own icon
/// before the first one was told it left, and resetting then would wipe the
/// neighbour's icon out. Call this right after setting the icon, keep the ticket,
/// and hand it to [`release`] on leaving.
///
/// Any cursor request made after the claim — through [`set_cursor`], or by a
/// window handle — voids it.
///
/// # Examples
///
/// ```
/// use aimer_utils::cursor::{claim, release};
///
/// let mine = claim();
/// assert!(release(mine), "nobody set the cursor in between");
/// assert!(!release(mine), "and a ticket is spent once released");
/// ```
#[inline]
pub fn claim() -> u64 {
    let ticket = LAST_CLAIM.with(|last| {
        last.set(last.get() + 1);
        last.get()
    });
    CLAIMANT.with(|claimant| claimant.set(ticket));
    ticket
}

/// Gives a [`claim`] up, and reports whether it was still the latest: only then
/// is the caller the one who should restore the default cursor.
///
/// A `0` ticket, and a ticket that was voided or already released, report `false`.
#[inline]
pub fn release(ticket: u64) -> bool {
    CLAIMANT.with(|claimant| {
        let holds = ticket != 0 && claimant.get() == ticket;
        if holds {
            claimant.set(0);
        }
        holds
    })
}

/// Whether `ticket` is still the latest claim: nobody has claimed or set the
/// cursor since.
#[inline]
pub fn is_current(ticket: u64) -> bool {
    ticket != 0 && CLAIMANT.with(|claimant| claimant.get() == ticket)
}

/// One widget's [`claim`] on the cursor, kept for as long as the widget has an
/// icon up.
///
/// Take it right after setting the icon; on the pointer leaving, [`release`] it
/// and restore the default cursor only if that answers `true`. The pointer may
/// have moved straight onto a neighbour that set its own icon first, and the
/// default must not be put over that.
///
/// [`release`]: CursorClaim::release
///
/// # Examples
///
/// ```
/// use aimer_utils::cursor::{CursorClaim, CursorIcon, set_cursor, reset_cursor};
///
/// let claim = CursorClaim::new();
/// set_cursor(CursorIcon::Text);
/// claim.take();
///
/// // ... the pointer leaves ...
/// if claim.release() {
///     reset_cursor();
/// }
/// ```
#[derive(Debug, Default)]
pub struct CursorClaim {
    ticket: Cell<u64>,
}

impl CursorClaim {
    /// A claim that holds nothing.
    #[inline]
    pub const fn new() -> Self {
        Self { ticket: Cell::new(0) }
    }

    /// Claims the cursor. Call it right after setting the icon.
    #[inline]
    pub fn take(&self) {
        self.ticket.set(claim());
    }

    /// Whether this claim is still the latest.
    #[inline]
    pub fn holds(&self) -> bool {
        is_current(self.ticket.get())
    }

    /// Gives the claim up, and answers whether it was still the latest, which is
    /// when the default cursor is this widget's to restore.
    #[inline]
    pub fn release(&self) -> bool {
        release(self.ticket.replace(0))
    }

    /// Takes over what `other` holds, for an element that replaces another which
    /// has an icon up.
    #[inline]
    pub fn inherit(&self, other: &CursorClaim) {
        self.ticket.set(other.ticket.get());
    }
}

/// Voids the latest [`claim`]: somebody else has set the cursor. Called by the
/// window handles and by [`set_cursor`]; widgets have no reason to.
#[doc(hidden)]
#[inline]
pub fn note_set() {
    CLAIMANT.with(|claimant| claimant.set(0));
}

/// The icon the application last asked its platform window to show.
static REQUESTED_ICON: AtomicUsize = AtomicUsize::new(CursorIcon::Default as usize);

/// A built-in icon as a number, for storing it in an atomic.
///
/// The inverse of [`icon_from_id`].
#[doc(hidden)]
#[inline]
pub const fn icon_id(icon: CursorIcon) -> usize {
    icon as usize
}

/// The built-in icon [`icon_id`] numbered, or [`CursorIcon::Default`] for a
/// number that names none.
#[doc(hidden)]
#[inline]
pub fn icon_from_id(id: usize) -> CursorIcon {
    match id {
        0 => CursorIcon::Default,
        1 => CursorIcon::ContextMenu,
        2 => CursorIcon::Help,
        3 => CursorIcon::Pointer,
        4 => CursorIcon::Progress,
        5 => CursorIcon::Wait,
        6 => CursorIcon::Cell,
        7 => CursorIcon::Crosshair,
        8 => CursorIcon::Text,
        9 => CursorIcon::VerticalText,
        10 => CursorIcon::Alias,
        11 => CursorIcon::Copy,
        12 => CursorIcon::Move,
        13 => CursorIcon::NoDrop,
        14 => CursorIcon::NotAllowed,
        15 => CursorIcon::Grab,
        16 => CursorIcon::Grabbing,
        17 => CursorIcon::EResize,
        18 => CursorIcon::NResize,
        19 => CursorIcon::NeResize,
        20 => CursorIcon::NwResize,
        21 => CursorIcon::SResize,
        22 => CursorIcon::SeResize,
        23 => CursorIcon::SwResize,
        24 => CursorIcon::WResize,
        25 => CursorIcon::EwResize,
        26 => CursorIcon::NsResize,
        27 => CursorIcon::NeswResize,
        28 => CursorIcon::NwseResize,
        29 => CursorIcon::ColResize,
        30 => CursorIcon::RowResize,
        31 => CursorIcon::AllScroll,
        32 => CursorIcon::ZoomIn,
        33 => CursorIcon::ZoomOut,
        34 => CursorIcon::DndAsk,
        35 => CursorIcon::AllResize,
        _ => CursorIcon::Default,
    }
}


/// Remembers that a platform window was just asked to show `cursor`.
///
/// Called by whatever hands a cursor to the platform, so that
/// [`requested_icon`] can answer for tools that draw a cursor of their own — a
/// driven app's virtual pointer follows it. A custom image has no icon name and
/// leaves the record as it was. Not called for the per-thread handler, which
/// belongs to one headless application and must not leak into another's.
#[doc(hidden)]
#[inline]
pub fn note_requested(cursor: &Cursor) {
    if let Cursor::Icon(icon) = cursor {
        REQUESTED_ICON.store(icon_id(*icon), Ordering::Relaxed);
    }
}

/// The built-in icon the application last asked its platform window to show,
/// [`CursorIcon::Default`] until it asks for one.
///
/// This is what the real mouse cursor looks like, which is exactly what a
/// virtual one should copy. Cheap enough to read every frame.
///
/// # Examples
///
/// ```
/// use aimer_utils::cursor::{CursorIcon, requested_icon};
///
/// // Nothing has been asked of a window yet in this process.
/// assert_eq!(requested_icon(), CursorIcon::Default);
/// ```
#[inline]
pub fn requested_icon() -> CursorIcon {
    icon_from_id(REQUESTED_ICON.load(Ordering::Relaxed))
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

    #[test]
    fn every_icon_round_trips_through_its_id() {
        for icon in [
            CursorIcon::Default,
            CursorIcon::Pointer,
            CursorIcon::Text,
            CursorIcon::NwseResize,
            CursorIcon::AllResize,
        ] {
            assert_eq!(icon_from_id(icon_id(icon)), icon);
        }
        assert_eq!(icon_from_id(usize::MAX), CursorIcon::Default, "an unknown id is the default");
    }

    #[test]
    fn the_last_icon_given_to_a_window_is_remembered_and_a_custom_cursor_is_not_an_icon() {
        note_requested(&Cursor::Icon(CursorIcon::EwResize));
        assert_eq!(requested_icon(), CursorIcon::EwResize);
        // `Cursor::Custom` cannot be built without an event loop, so the
        // remaining case is a later icon replacing the earlier one.
        note_requested(&Cursor::Icon(CursorIcon::Text));
        assert_eq!(requested_icon(), CursorIcon::Text);
        note_requested(&Cursor::Icon(CursorIcon::Default));
    }

    #[test]
    fn a_request_answered_by_the_thread_handler_is_not_a_window_request() {
        note_requested(&Cursor::Icon(CursorIcon::Default));
        let previous = set_thread_cursor_handler(|_| {});
        assert!(set_cursor(CursorIcon::Grab));
        restore_thread_cursor_handler(previous);
        assert_eq!(requested_icon(), CursorIcon::Default);
    }

    #[test]
    fn a_claim_is_released_once_and_only_while_it_is_the_latest() {
        let first = claim();
        let second = claim();
        assert!(!release(first), "a later claim took the cursor over");
        assert!(release(second));
        assert!(!release(second), "a ticket is spent once released");
        assert!(!release(0), "no ticket is never the owner");
    }

    #[test]
    fn any_cursor_request_voids_the_claim() {
        let previous = set_thread_cursor_handler(|_| {});
        let ticket = claim();
        assert!(set_cursor(CursorIcon::Text));
        restore_thread_cursor_handler(previous);
        assert!(!release(ticket), "somebody else set the cursor after the claim");
    }

    #[test]
    fn claims_do_not_cross_threads() {
        let ticket = claim();
        std::thread::spawn(|| {
            let _ = claim();
        })
        .join()
        .expect("the other thread finished");
        assert!(release(ticket), "another application's claim is not ours to lose");
    }

    #[test]
    fn a_cursor_claim_is_held_until_somebody_else_sets_the_cursor() {
        let mine = CursorClaim::new();
        assert!(!mine.holds(), "nothing is held before it is taken");
        mine.take();
        assert!(mine.holds());
        let theirs = CursorClaim::new();
        theirs.take();
        assert!(!mine.release(), "the neighbour took the cursor over");
        assert!(theirs.release());
        assert!(!theirs.release(), "released once");
    }

    #[test]
    fn a_replacement_inherits_the_claim_it_replaces() {
        let old = CursorClaim::new();
        old.take();
        let new = CursorClaim::new();
        new.inherit(&old);
        assert!(new.holds());
        assert!(new.release());
    }
}
