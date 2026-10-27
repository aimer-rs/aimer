use std::cell::{Cell, RefCell};

use smallvec::SmallVec;

aimer_tls::thread_local! {
    static NEXT_TRAP: Cell<u64> = const { Cell::new(0) };
    static TRAP_GENERATION: Cell<u64> = const { Cell::new(0) };
    static TRAPS: RefCell<SmallVec<[FocusTrapId; 4]>> = const { RefCell::new(SmallVec::new_const()) };
}

/// The focus traps of one window, kept with the window while another is active.
///
/// A trap confines keyboard focus to one region of one window. With several
/// windows on a thread, a dialog trapping focus in one must not trap it in the
/// others, so each window holds its own list and puts it on the thread while it
/// is the active one.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct WindowFocusTraps(SmallVec<[FocusTrapId; 4]>);

impl WindowFocusTraps {
    /// Exchanges these traps with the thread's. Calling it twice puts both back.
    pub fn swap_with_thread(&mut self) {
        let any = TRAPS.with(|traps| {
            let mut traps = traps.borrow_mut();
            std::mem::swap(&mut *traps, &mut self.0);
            !traps.is_empty() || !self.0.is_empty()
        });
        // Managers decide whether they are confined from the generation, so a
        // change of the active list has to be visible as one.
        if any {
            advance_trap_generation();
        }
    }
}

/// Identifies one focus trap for as long as it is held.
///
/// Identities are never reused, so a trap that has been released can never be
/// mistaken for the trap that replaced it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FocusTrapId(u64);

/// Confines keyboard focus to one region of the application while it is held.
///
/// A trap exists because a modal is a *mode*: while a dialog is presented, the
/// text field behind it must stop receiving text, `Tab` must not walk into the
/// application underneath, and closing the dialog must give focus back to
/// whatever had it before. Aimer presents overlays through their own dispatch
/// root — the content of a modal is not part of the element tree it covers — so
/// confinement cannot be expressed as a subtree restriction. It is expressed
/// here instead, as a thread-local stack of regions: the innermost held trap is
/// the [active](active_focus_trap) one, and a [`crate::FocusManager`] that
/// belongs to any other region [suspends](crate::FocusManager::set_trap)
/// itself, remembering its owner so it can be restored.
///
/// The trap is released when the guard is dropped, which is what makes it safe:
/// an overlay that is torn down — dismissed, animated out, or dropped by a
/// panic while unwinding — cannot leave focus confined to a region that no
/// longer exists.
///
/// Traps may be released out of order. A dialog that opens a second dialog and
/// is then itself dismissed first leaves the second one trapping, exactly as it
/// still appears on screen.
///
/// # Examples
///
/// ```
/// use aimer_focus::{FocusTrap, active_focus_trap};
///
/// assert_eq!(active_focus_trap(), None);
///
/// let dialog = FocusTrap::acquire();
/// assert_eq!(active_focus_trap(), Some(dialog.id()));
///
/// // A nested overlay takes over confinement while it is held.
/// let nested = FocusTrap::acquire();
/// assert_eq!(active_focus_trap(), Some(nested.id()));
/// drop(nested);
///
/// assert_eq!(active_focus_trap(), Some(dialog.id()));
/// drop(dialog);
/// assert_eq!(active_focus_trap(), None);
/// ```
#[derive(Debug)]
pub struct FocusTrap {
    id: FocusTrapId,
}

impl FocusTrap {
    /// Confines focus to a new region until the returned guard is dropped.
    #[inline]
    pub fn acquire() -> Self {
        let id = FocusTrapId(NEXT_TRAP.with(|next| {
            let id = next
                .get()
                .checked_add(1)
                .expect("exhausted all focus trap identities");
            next.set(id);
            id
        }));
        TRAPS.with(|traps| traps.borrow_mut().push(id));
        advance_trap_generation();
        Self { id }
    }

    /// Returns the region this trap confines focus to.
    #[inline]
    pub const fn id(&self) -> FocusTrapId {
        self.id
    }
}

impl Drop for FocusTrap {
    #[inline]
    fn drop(&mut self) {
        TRAPS.with(|traps| traps.borrow_mut().retain(|held| *held != self.id));
        advance_trap_generation();
    }
}

/// Returns the innermost held focus trap, if focus is confined at all.
#[inline]
pub fn active_focus_trap() -> Option<FocusTrapId> {
    TRAPS.with(|traps| traps.borrow().last().copied())
}

/// Returns how often confinement has changed on this thread.
///
/// The counter only ever grows, so a focus manager can tell whether a trap was
/// acquired or released since it last resolved ownership by comparing a single
/// integer, without looking at the stack.
#[inline]
pub fn focus_trap_generation() -> u64 {
    TRAP_GENERATION.with(Cell::get)
}

#[inline]
fn advance_trap_generation() {
    TRAP_GENERATION.with(|generation| generation.set(generation.get().wrapping_add(1)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trap_is_active_until_one_is_acquired() {
        assert_eq!(active_focus_trap(), None);

        let trap = FocusTrap::acquire();
        assert_eq!(active_focus_trap(), Some(trap.id()));

        drop(trap);
        assert_eq!(active_focus_trap(), None);
    }

    #[test]
    fn the_innermost_trap_confines_focus() {
        let outer = FocusTrap::acquire();
        let inner = FocusTrap::acquire();

        assert_eq!(active_focus_trap(), Some(inner.id()));
        assert_ne!(outer.id(), inner.id());

        drop(inner);
        assert_eq!(active_focus_trap(), Some(outer.id()));
    }

    #[test]
    fn a_trap_released_out_of_order_leaves_the_others_confining() {
        let outer = FocusTrap::acquire();
        let inner = FocusTrap::acquire();

        drop(outer);

        assert_eq!(active_focus_trap(), Some(inner.id()));
    }

    #[test]
    fn every_change_of_confinement_advances_the_generation() {
        let before = focus_trap_generation();

        let trap = FocusTrap::acquire();
        let acquired = focus_trap_generation();
        assert_ne!(acquired, before);

        drop(trap);
        assert_ne!(focus_trap_generation(), acquired);
    }

    #[test]
    fn a_trap_held_by_one_window_does_not_confine_another() {
        let (mut first, mut second) = (WindowFocusTraps::default(), WindowFocusTraps::default());

        first.swap_with_thread();
        let dialog = FocusTrap::acquire();
        let held = active_focus_trap();
        first.swap_with_thread();

        second.swap_with_thread();
        assert_eq!(held, Some(dialog.id()));
        assert_eq!(active_focus_trap(), None, "the other window was trapped");
        second.swap_with_thread();

        first.swap_with_thread();
        assert_eq!(active_focus_trap(), Some(dialog.id()));
        drop(dialog);
        assert_eq!(active_focus_trap(), None);
        first.swap_with_thread();
    }

    #[test]
    fn changing_the_active_window_is_visible_as_a_new_generation() {
        let mut window = WindowFocusTraps::default();
        window.swap_with_thread();
        let _dialog = FocusTrap::acquire();
        window.swap_with_thread();

        let before = focus_trap_generation();
        window.swap_with_thread();

        assert_ne!(focus_trap_generation(), before);
        window.swap_with_thread();
    }
}
