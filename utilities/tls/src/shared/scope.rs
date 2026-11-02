//! Batched access to several thread-locals through one table lookup.

use std::cell::{Cell, RefCell};

use super::{AccessError, LocalKey, Table, with_table};

/// A resolved handle to the current thread's thread-local table.
///
/// Every [`LocalKey`] access starts by looking up this thread's table, which on
/// most platforms is the expensive part. A scope does that lookup once, so
/// code touching several keys in a row pays for it once instead of once per
/// key. Create one with [`scope`].
///
/// A scope is tied to the thread it was created on and cannot leave the
/// closure given to [`scope`].
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
///
/// aimer_tls::thread_local! {
///     static WIDTH: Cell<u32> = const { Cell::new(4) };
///     static HEIGHT: Cell<u32> = const { Cell::new(3) };
/// }
///
/// let area = aimer_tls::scope(|t| t.get(&WIDTH) * t.get(&HEIGHT));
/// assert_eq!(area, 12);
/// ```
pub struct Scope<'t> {
    table: &'t Table,
}

/// Resolves the current thread's table once and runs `f` with it.
///
/// Keys read through the [`Scope`] see exactly the storage the keys' own
/// methods use, and scopes nest freely with each other and with plain
/// [`LocalKey`] calls, including from inside a thread-local initialiser.
#[inline(always)]
pub fn scope<R>(f: impl FnOnce(&Scope<'_>) -> R) -> R {
    with_table(|table| f(&Scope { table }))
}

impl Scope<'_> {
    /// Returns this thread's value of `key`, creating it on first use.
    ///
    /// The reference lasts as long as the borrow of the scope; the value is
    /// only destroyed when the thread exits.
    ///
    /// # Panics
    ///
    /// Panics if the value is being or has been destroyed.
    #[inline(always)]
    pub fn local<T: 'static>(&self, key: &'static LocalKey<T>) -> &T {
        let cached = key.hot_cached();
        if cached != 0 {
            // SAFETY: see `LocalKey::local_ref`.
            return unsafe { &*(cached as *const T) };
        }
        // SAFETY: see `try_local`.
        unsafe { &*key.ptr_in(self.table) }
    }

    /// Like [`local`](Self::local), but returns [`AccessError`] instead of
    /// panicking when the value is being or has been destroyed.
    #[inline(always)]
    pub fn try_local<T: 'static>(&self, key: &'static LocalKey<T>) -> Result<&T, AccessError> {
        let ptr = key.value_in(self.table)?;
        // SAFETY: the value is heap-allocated, owned by this thread's table,
        // and freed only by that value's own teardown. Teardown runs on this
        // thread and never while this scope's caller is running, except for
        // values other than the ones it can still hand out here.
        Ok(unsafe { &*ptr })
    }

    /// Runs `f` with this thread's value of `key`.
    ///
    /// # Panics
    ///
    /// Panics if the value is being or has been destroyed.
    #[inline(always)]
    pub fn with<T: 'static, R>(&self, key: &'static LocalKey<T>, f: impl FnOnce(&T) -> R) -> R {
        f(self.local(key))
    }

    /// Returns a copy of a `Cell` thread-local.
    #[inline(always)]
    pub fn get<T: Copy + 'static>(&self, key: &'static LocalKey<Cell<T>>) -> T {
        self.local(key).get()
    }

    /// Replaces a `Cell` thread-local. Does not run the initialiser.
    #[inline(always)]
    pub fn set<T: 'static>(&self, key: &'static LocalKey<Cell<T>>, value: T) {
        let cached = key.hot_cached();
        if cached != 0 {
            // SAFETY: see `LocalKey::local_ref`.
            unsafe { &*(cached as *const Cell<T>) }.set(value);
            return;
        }
        let found = key.lookup_in(self.table);
        if found as usize == 0 {
            key.store_fresh(self.table, Cell::new(value));
        } else {
            // SAFETY: see `try_local`.
            unsafe { &*found }.set(value);
        }
    }

    /// Updates a `Cell` thread-local in place with `f`.
    #[inline(always)]
    pub fn update<T: Copy + 'static>(
        &self,
        key: &'static LocalKey<Cell<T>>,
        f: impl FnOnce(T) -> T,
    ) {
        self.local(key).update(f);
    }

    /// Takes a `Cell` thread-local, leaving `Default::default()` behind.
    #[inline(always)]
    pub fn take<T: Default + 'static>(&self, key: &'static LocalKey<Cell<T>>) -> T {
        self.local(key).take()
    }

    /// Replaces a `Cell` thread-local, returning the old value.
    #[inline(always)]
    pub fn replace<T: 'static>(&self, key: &'static LocalKey<Cell<T>>, value: T) -> T {
        self.local(key).replace(value)
    }

    /// Borrows a `RefCell` thread-local immutably for the duration of `f`.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently mutably borrowed.
    #[inline(always)]
    pub fn with_borrow<T: 'static, R>(
        &self,
        key: &'static LocalKey<RefCell<T>>,
        f: impl FnOnce(&T) -> R,
    ) -> R {
        f(&self.local(key).borrow())
    }

    /// Borrows a `RefCell` thread-local mutably for the duration of `f`.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline(always)]
    pub fn with_borrow_mut<T: 'static, R>(
        &self,
        key: &'static LocalKey<RefCell<T>>,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        f(&mut self.local(key).borrow_mut())
    }
}
