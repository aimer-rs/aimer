//! The shared per-thread table behind [`crate::thread_local!`] on Android.
//!
//! Every [`LocalKey`] owns a process-wide slot index. Each thread owns one
//! [`Table`], the only std `thread_local!` in this crate, so the only OS TLS
//! key, mapping slot indices to lazily created, heap-allocated values.
//!
//! Values are boxed so their address stays fixed while the table's `Vec` grows
//! and while other slots are initialised from inside a `with` closure.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Source of slot indices. Index `0` is never handed out, so a key's `0` means
/// "not assigned yet".
static NEXT_SLOT: AtomicUsize = AtomicUsize::new(1);

/// Error returned by [`LocalKey::try_with`] when the thread's table is already
/// gone because the thread is exiting.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub struct AccessError;

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("already destroyed")
    }
}

impl std::error::Error for AccessError {}

/// A type-erased heap value together with the function that frees it.
struct Slot {
    ptr: *mut (),
    drop: unsafe fn(*mut ()),
}

/// # Safety
///
/// `ptr` must come from `Box::<T>::into_raw` and must not be used afterwards.
unsafe fn drop_boxed<T>(ptr: *mut ()) {
    drop(unsafe { Box::from_raw(ptr.cast::<T>()) });
}

/// All shared thread-local values of one thread.
struct Table {
    slots: RefCell<Vec<Option<Slot>>>,
}

impl Table {
    const fn new() -> Self {
        Self {
            slots: RefCell::new(Vec::new()),
        }
    }

    fn get(&self, index: usize) -> Option<*mut ()> {
        self.slots
            .borrow()
            .get(index)
            .and_then(|slot| slot.as_ref())
            .map(|slot| slot.ptr)
    }

    /// Stores `slot` unless the index is already filled; returns the pointer
    /// that ends up owning the index.
    ///
    /// Initialisers may touch other keys, so a re-entrant initialisation of the
    /// same key can reach here with the slot already filled. The first value
    /// wins and the loser is handed back to the caller to drop.
    fn insert(&self, index: usize, slot: Slot) -> Result<*mut (), Slot> {
        let mut slots = self.slots.borrow_mut();
        if slots.len() <= index {
            slots.resize_with(index + 1, || None);
        }
        if slots[index].is_some() {
            return Err(slot);
        }
        let ptr = slot.ptr;
        slots[index] = Some(slot);
        Ok(ptr)
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        // Drop one value at a time with no borrow held, newest first, so a
        // destructor never observes a half-borrowed table.
        loop {
            let slot = self
                .slots
                .get_mut()
                .iter_mut()
                .rev()
                .find_map(|slot| slot.take());
            match slot {
                Some(slot) => unsafe { (slot.drop)(slot.ptr) },
                None => break,
            }
        }
    }
}

std::thread_local! {
    /// The single OS TLS key every shared thread-local lives behind.
    static TABLE: Table = const { Table::new() };
}

/// A thread-local value stored in the shared per-thread table.
///
/// Created by [`crate::thread_local!`]; it mirrors the parts of
/// [`std::thread::LocalKey`] the framework uses.
pub struct LocalKey<T: 'static> {
    slot: AtomicUsize,
    init: fn() -> T,
}

impl<T: 'static> LocalKey<T> {
    /// Creates a key whose per-thread value is built by `init` on first use.
    #[doc(hidden)]
    pub const fn new(init: fn() -> T) -> Self {
        Self {
            slot: AtomicUsize::new(0),
            init,
        }
    }

    fn index(&self) -> usize {
        let current = self.slot.load(Ordering::Relaxed);
        if current != 0 {
            return current;
        }
        let fresh = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
        match self
            .slot
            .compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => fresh,
            Err(winner) => winner,
        }
    }

    /// Address of this thread's value, creating it on first access.
    ///
    /// The address stays valid until the thread's table is destroyed.
    fn value(&'static self) -> Result<*const T, AccessError> {
        let index = self.index();
        if let Some(ptr) = TABLE.try_with(|table| table.get(index)).map_err(|_| AccessError)? {
            return Ok(ptr.cast::<T>());
        }

        // The initialiser may itself use thread-locals, so it runs with no
        // borrow of the table held.
        let created = Slot {
            ptr: Box::into_raw(Box::new((self.init)())).cast::<()>(),
            drop: drop_boxed::<T>,
        };
        match TABLE.try_with(|table| table.insert(index, created)) {
            Ok(Ok(ptr)) => Ok(ptr.cast::<T>()),
            Ok(Err(loser)) => {
                unsafe { (loser.drop)(loser.ptr) };
                // The winner is the value already in the table.
                TABLE
                    .try_with(|table| table.get(index))
                    .ok()
                    .flatten()
                    .map(|ptr| ptr.cast::<T>().cast_const())
                    .ok_or(AccessError)
            }
            Err(_) => Err(AccessError),
        }
    }

    /// Runs `f` with a reference to this thread's value.
    ///
    /// # Panics
    ///
    /// Panics if the thread's table has already been destroyed.
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.try_with(f).expect(
            "cannot access a Thread Local Storage value during or after destruction: AccessError",
        )
    }

    /// Like [`with`](Self::with), but returns [`AccessError`] instead of
    /// panicking when the thread is being torn down.
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let ptr = self.value()?;
        // SAFETY: the value is boxed, owned by this thread's table, and only
        // freed when the table is dropped at thread exit, which cannot happen
        // while this thread is inside `f`.
        Ok(f(unsafe { &*ptr }))
    }
}

impl<T: 'static> LocalKey<Cell<T>> {
    /// Replaces the value, dropping the old one.
    pub fn set(&'static self, value: T) {
        self.with(|cell| cell.set(value));
    }

    /// Returns a copy of the value.
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(Cell::get)
    }

    /// Takes the value, leaving `Default::default()` behind.
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(Cell::take)
    }

    /// Replaces the value, returning the old one.
    pub fn replace(&'static self, value: T) -> T {
        self.with(|cell| cell.replace(value))
    }
}

impl<T: 'static> LocalKey<RefCell<T>> {
    /// Borrows the value immutably for the duration of `f`.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently mutably borrowed.
    pub fn with_borrow<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        self.with(|cell| f(&cell.borrow()))
    }

    /// Borrows the value mutably for the duration of `f`.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    pub fn with_borrow_mut<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.with(|cell| f(&mut cell.borrow_mut()))
    }

    /// Replaces the value, dropping the old one.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    pub fn set(&'static self, value: T) {
        self.with(|cell| *cell.borrow_mut() = value);
    }

    /// Takes the value, leaving `Default::default()` behind.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(RefCell::take)
    }

    /// Replaces the value, returning the old one.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    pub fn replace(&'static self, value: T) -> T {
        self.with(|cell| cell.replace(value))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static INITS: AtomicUsize = AtomicUsize::new(0);

    crate::shared_thread_local! {
        static LAZY: Cell<u32> = {
            INITS.fetch_add(1, Ordering::SeqCst);
            Cell::new(7)
        };
        static CONST_CELL: Cell<u32> = const { Cell::new(1) };
        pub(crate) static LIST: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn value_is_created_lazily_once_per_thread() {
        assert_eq!(INITS.load(Ordering::SeqCst), 0);
        assert_eq!(LAZY.get(), 7);
        LAZY.set(9);
        assert_eq!(LAZY.get(), 9);
        assert_eq!(INITS.load(Ordering::SeqCst), 1);

        // A second thread gets its own, freshly initialised value.
        std::thread::spawn(|| {
            assert_eq!(LAZY.get(), 7);
        })
        .join()
        .unwrap();
        assert_eq!(INITS.load(Ordering::SeqCst), 2);
        assert_eq!(LAZY.get(), 9);
    }

    #[test]
    fn cell_and_ref_cell_helpers_match_std() {
        assert_eq!(CONST_CELL.replace(5), 1);
        assert_eq!(CONST_CELL.take(), 5);
        assert_eq!(CONST_CELL.get(), 0);

        LIST.with_borrow_mut(|list| list.push(1));
        LIST.with_borrow_mut(|list| list.push(2));
        assert_eq!(LIST.with_borrow(|list| list.clone()), vec![1, 2]);
        assert_eq!(LIST.replace(vec![3]), vec![1, 2]);
        assert_eq!(LIST.take(), vec![3]);
        LIST.set(vec![4]);
        assert_eq!(LIST.with_borrow(Vec::len), 1);
    }

    #[test]
    fn initialiser_may_use_other_keys() {
        crate::shared_thread_local! {
            static A: Cell<u32> = const { Cell::new(10) };
            static B: Cell<u32> = Cell::new(A.get() + 1);
        }
        assert_eq!(B.get(), 11);
    }

    #[test]
    fn with_may_nest_other_keys_while_the_table_grows() {
        crate::shared_thread_local! {
            static OUTER: Cell<u32> = const { Cell::new(1) };
            static INNER: Cell<u32> = const { Cell::new(2) };
        }
        let sum = OUTER.with(|outer| INNER.with(|inner| outer.get() + inner.get()));
        assert_eq!(sum, 3);
    }

    #[test]
    fn values_are_dropped_when_the_thread_exits() {
        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        crate::shared_thread_local! {
            static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let for_thread = drops.clone();
        std::thread::spawn(move || {
            PROBE.set(Some(Probe(for_thread)));
        })
        .join()
        .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rc_values_need_not_be_send() {
        crate::shared_thread_local! {
            static SHARED: Rc<Cell<u32>> = Rc::new(Cell::new(3));
        }
        SHARED.with(|rc| rc.set(4));
        assert_eq!(SHARED.with(|rc| rc.get()), 4);
    }

    #[test]
    fn access_during_thread_teardown_reports_an_error() {
        // 0 = destructor not run, 1 = access refused, 2 = access allowed.
        static OUTCOME: AtomicUsize = AtomicUsize::new(0);

        struct Late;
        impl Drop for Late {
            fn drop(&mut self) {
                // The table is being dropped: `try_with` must not panic.
                let seen = CONST_CELL.try_with(|cell| cell.get());
                OUTCOME.store(if seen.is_err() { 1 } else { 2 }, Ordering::SeqCst);
            }
        }
        crate::shared_thread_local! {
            static HOLDER: RefCell<Option<Late>> = const { RefCell::new(None) };
        }
        std::thread::spawn(|| {
            HOLDER.set(Some(Late));
        })
        .join()
        .unwrap();
        assert_eq!(OUTCOME.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn more_statics_than_an_android_process_has_tls_keys() {
        // With std's keyed implementation this many statics would abort on
        // Android; here they all share the table's single key. Declared in
        // batches because each static in one block costs a macro recursion.
        macro_rules! batch {
            ($touch:ident; $($name:ident),*) => {
                crate::shared_thread_local! {
                    $(static $name: Cell<usize> = const { Cell::new(0) };)*
                }
                fn $touch() -> usize {
                    let mut total = 0;
                    $($name.set($name.get() + 1); total += $name.get();)*
                    total
                }
            };
        }
        batch!(touch_a; A0, A1, A2, A3, A4, A5, A6, A7, A8, A9, A10, A11, A12, A13, A14, A15, A16, A17, A18, A19, A20, A21, A22, A23, A24, A25, A26, A27, A28, A29, A30, A31, A32, A33, A34, A35, A36, A37, A38, A39, A40, A41, A42, A43, A44, A45, A46, A47, A48, A49);
        batch!(touch_b; B0, B1, B2, B3, B4, B5, B6, B7, B8, B9, B10, B11, B12, B13, B14, B15, B16, B17, B18, B19, B20, B21, B22, B23, B24, B25, B26, B27, B28, B29, B30, B31, B32, B33, B34, B35, B36, B37, B38, B39, B40, B41, B42, B43, B44, B45, B46, B47, B48, B49);
        batch!(touch_c; C0, C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15, C16, C17, C18, C19, C20, C21, C22, C23, C24, C25, C26, C27, C28, C29, C30, C31, C32, C33, C34, C35, C36, C37, C38, C39, C40, C41, C42, C43, C44, C45, C46, C47, C48, C49);
        assert_eq!(touch_a() + touch_b() + touch_c(), 150);
    }
}
