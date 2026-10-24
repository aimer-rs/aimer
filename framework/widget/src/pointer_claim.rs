//! Which pointer gestures already mean something, so an ancestor cannot take
//! them for something else.
//!
//! A pointer capture answers *where* the events of a pointer go; it says nothing
//! about *what* they mean. Every gesture recognizer in the framework captures on
//! press — a button does it so a press that slides off it can still be
//! cancelled — so an enclosing [`Scrollable`](https://docs.rs/aimer_scroll) that
//! refused to drag whenever a descendant had captured would never scroll over a
//! button again.
//!
//! What such an ancestor actually needs to know is narrower: has a descendant
//! *already begun* a gesture of its own with this pointer, one that a scroll
//! would destroy? Dragging across text to select it is exactly that. A claim is
//! that answer, and nothing more:
//!
//! - a descendant that starts a drag of its own **claims** the pointer;
//! - an ancestor that arbitrates drags — a scrollable — leaves a claimed pointer
//!   alone;
//! - the claim is dropped when the gesture ends, and unconditionally when the
//!   pointer goes up or the interaction is cancelled, so a claim can never
//!   outlive the finger that made it and deadlock scrolling.
//!
//! Claims are held per thread, in a fixed-size table sized for ten
//! simultaneous contacts, so claiming, releasing and asking are allocation-free
//! constant-time operations on the pointer path.
//!
//! # Examples
//!
//! ```
//! use aimer_events::pointer::PointerSource;
//! use aimer_widget::{PointerKey, claim_pointer, is_pointer_claimed, release_pointer};
//!
//! let finger = PointerKey::new(PointerSource::Touch, 0);
//!
//! // A text that has started selecting takes the gesture for itself.
//! assert!(claim_pointer(finger));
//! assert!(is_pointer_claimed(finger));
//!
//! // Which an enclosing scrollable respects until the gesture is over.
//! assert!(release_pointer(finger));
//! assert!(!is_pointer_claimed(finger));
//! ```

use std::cell::RefCell;

use crate::components::event_element::PointerKey;

/// How many pointers may hold a claim at once.
///
/// Ten simultaneous contacts is the most any touch screen reports, and a claim
/// exists only while its pointer is down, so the table cannot grow past the
/// number of fingers on the glass.
const MAX_CLAIMED_POINTERS: usize = 10;

thread_local! {
    /// The pointers whose gesture is already spoken for.
    ///
    /// Per thread because pointers are delivered on the thread that owns the
    /// event loop and read by the widget tree on that same thread.
    static CLAIMED: RefCell<[Option<PointerKey>; MAX_CLAIMED_POINTERS]> =
        const { RefCell::new([None; MAX_CLAIMED_POINTERS]) };
}

/// Takes the gesture of `pointer` for the caller.
///
/// Returns whether the claim is now held. Claiming a pointer that is already
/// claimed succeeds and changes nothing, so a recognizer may claim on every
/// event of a gesture rather than tracking whether it did so already. `false`
/// means the table is full — more pointers than a screen can report are down —
/// in which case the caller simply does not get the courtesy of an ancestor
/// standing down.
pub fn claim_pointer(pointer: PointerKey) -> bool {
    CLAIMED.with(|claimed| {
        let mut claimed = claimed.borrow_mut();
        let mut free = None;
        for (index, slot) in claimed.iter().enumerate() {
            match slot {
                Some(held) if *held == pointer => return true,
                None if free.is_none() => free = Some(index),
                _ => {}
            }
        }
        match free {
            Some(index) => {
                claimed[index] = Some(pointer);
                true
            }
            None => false,
        }
    })
}

/// Gives the gesture of `pointer` back.
///
/// Returns whether a claim was actually held, which lets a caller that releases
/// defensively — on every pointer up, say — tell a real release from a no-op.
pub fn release_pointer(pointer: PointerKey) -> bool {
    CLAIMED.with(|claimed| {
        let mut claimed = claimed.borrow_mut();
        for slot in claimed.iter_mut() {
            if *slot == Some(pointer) {
                *slot = None;
                return true;
            }
        }
        false
    })
}

/// Reports whether a descendant has already begun a gesture with `pointer`.
///
/// This is the question an ancestor that arbitrates drags asks before starting
/// one of its own.
pub fn is_pointer_claimed(pointer: PointerKey) -> bool {
    CLAIMED.with(|claimed| claimed.borrow().contains(&Some(pointer)))
}

/// Drops every claim, as a cancelled interaction must.
///
/// Returns how many claims were held. Cancellation is delivered to the whole
/// tree at once and no gesture survives it, so the table is emptied rather than
/// pruned pointer by pointer.
pub fn release_all_pointers() -> usize {
    CLAIMED.with(|claimed| {
        let mut claimed = claimed.borrow_mut();
        let mut released = 0;
        for slot in claimed.iter_mut() {
            if slot.take().is_some() {
                released += 1;
            }
        }
        released
    })
}

/// How many pointers currently hold a claim.
#[inline]
pub fn claimed_pointer_count() -> usize {
    CLAIMED.with(|claimed| claimed.borrow().iter().filter(|slot| slot.is_some()).count())
}

#[cfg(test)]
mod tests {
    use aimer_events::pointer::PointerSource;

    use super::*;

    fn finger(id: u64) -> PointerKey {
        PointerKey::new(PointerSource::Touch, id)
    }

    #[test]
    fn an_unclaimed_pointer_is_free_for_an_ancestor_to_use() {
        assert!(!is_pointer_claimed(finger(0)));
        assert_eq!(claimed_pointer_count(), 0);
    }

    #[test]
    fn claiming_marks_only_that_pointer() {
        assert!(claim_pointer(finger(1)));

        assert!(is_pointer_claimed(finger(1)));
        assert!(!is_pointer_claimed(finger(2)));
        assert!(
            !is_pointer_claimed(PointerKey::new(PointerSource::Mouse, 1)),
            "a finger and a mouse with the same id are different pointers"
        );
    }

    #[test]
    fn claiming_twice_holds_one_claim() {
        assert!(claim_pointer(finger(3)));
        assert!(claim_pointer(finger(3)));

        assert_eq!(claimed_pointer_count(), 1);
        assert!(release_pointer(finger(3)));
        assert!(!is_pointer_claimed(finger(3)));
    }

    #[test]
    fn releasing_a_pointer_that_never_claimed_reports_nothing() {
        assert!(!release_pointer(finger(4)));
    }

    #[test]
    fn cancellation_drops_every_claim() {
        assert!(claim_pointer(finger(5)));
        assert!(claim_pointer(finger(6)));

        assert_eq!(release_all_pointers(), 2);
        assert_eq!(claimed_pointer_count(), 0);
        assert_eq!(release_all_pointers(), 0);
    }

    #[test]
    fn more_contacts_than_a_screen_reports_are_refused_without_losing_a_claim() {
        for id in 0..MAX_CLAIMED_POINTERS as u64 {
            assert!(claim_pointer(finger(id)));
        }

        assert!(!claim_pointer(finger(MAX_CLAIMED_POINTERS as u64)));
        assert!(is_pointer_claimed(finger(0)));
        assert_eq!(claimed_pointer_count(), MAX_CLAIMED_POINTERS);

        release_all_pointers();
    }

    #[test]
    fn a_released_slot_is_reused() {
        for id in 0..MAX_CLAIMED_POINTERS as u64 {
            assert!(claim_pointer(finger(id)));
        }
        assert!(release_pointer(finger(0)));

        assert!(claim_pointer(finger(MAX_CLAIMED_POINTERS as u64)));

        release_all_pointers();
    }
}
