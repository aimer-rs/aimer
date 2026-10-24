//! Closing a menu from inside it.
//!
//! A menu is presented through the modal host, which hands back a
//! [`ModalHandle`] — but its rows are built *before* that handle exists, and a
//! row that runs `Copy` has to close the menu it was chosen from. The handle is
//! therefore shared through this slot: the content captures it while the menu is
//! being described, and [`crate::ContextMenu::show`] fills it in the moment the
//! host answers.
//!
//! # Examples
//!
//! ```
//! use aimer_ctxmenu::ContextMenuDismiss;
//!
//! // A slot no menu has claimed yet closes nothing, and says so.
//! let dismiss = ContextMenuDismiss::new();
//! assert!(!dismiss.dismiss());
//! ```

#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use aimer_modal::ModalHandle;

/// A shared handle that closes the menu it was given to, once that menu is
/// open.
///
/// Cloning is cheap and every clone sees the same menu, which is what lets a
/// custom child hold one while the menu is still being built.
#[derive(Clone, Default)]
pub struct ContextMenuDismiss {
    handle: Rc<RefCell<Option<ModalHandle>>>,
    /// Whether closing was ever asked for, shared by every clone.
    ///
    /// Only tests read it: they drive the content without a modal host, where a
    /// real handle cannot exist, so the ask itself is the observable effect.
    #[cfg(test)]
    asked: Rc<Cell<bool>>,
}

impl ContextMenuDismiss {
    /// Creates a slot that no menu has claimed yet.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Closes the menu, reporting whether there was one left to close.
    ///
    /// Repeated calls are harmless, which is what lets every dismissal path
    /// call it without asking first.
    pub fn dismiss(&self) -> bool {
        #[cfg(test)]
        self.asked.set(true);
        let handle = self.handle.borrow().clone();
        handle.is_some_and(|handle| handle.dismiss())
    }

    /// Whether a menu has claimed this slot.
    #[inline]
    pub fn is_claimed(&self) -> bool {
        self.handle.borrow().is_some()
    }

    /// Hands the presented menu's handle to every clone of this slot.
    pub(crate) fn claim(&self, handle: ModalHandle) {
        *self.handle.borrow_mut() = Some(handle);
    }

    /// Whether closing was ever asked for.
    #[cfg(test)]
    pub(crate) fn was_asked_to_dismiss(&self) -> bool {
        self.asked.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unclaimed_slot_closes_nothing() {
        let dismiss = ContextMenuDismiss::new();

        assert!(!dismiss.is_claimed());
        assert!(!dismiss.dismiss());
    }
}
