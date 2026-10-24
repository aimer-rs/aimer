use std::cell::{Cell, UnsafeCell};

/// Interior mutability for values confined to Aimer's UI thread.
pub(crate) struct LocalCell<T> {
    value: UnsafeCell<T>,
    borrowed: Cell<bool>,
}

struct BorrowGuard<'a>(&'a Cell<bool>);

impl Drop for BorrowGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl<T> LocalCell<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            value: UnsafeCell::new(value),
            borrowed: Cell::new(false),
        }
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let _guard = self.borrow();
        // Safety: the borrow guard rejects overlapping access, and `LocalCell`
        // is not `Sync`, so the value remains confined to one thread.
        unsafe { f(&*self.value.get()) }
    }

    pub(crate) fn with_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let _guard = self.borrow();
        // Safety: the borrow guard rejects overlapping access, and `LocalCell`
        // is not `Sync`, so the mutable reference is unique.
        unsafe { f(&mut *self.value.get()) }
    }

    fn borrow(&self) -> BorrowGuard<'_> {
        assert!(
            !self.borrowed.replace(true),
            "LocalCell is already borrowed"
        );
        BorrowGuard(&self.borrowed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_is_visible_to_later_access() {
        let cell = LocalCell::new(1);
        cell.with_mut(|value| *value = 2);

        assert_eq!(cell.with(|value| *value), 2);
    }

    #[test]
    #[should_panic(expected = "LocalCell is already borrowed")]
    fn reentrant_access_is_rejected() {
        let cell = LocalCell::new(1);
        cell.with(|_| cell.with(|value| *value));
    }
}
