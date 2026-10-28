//! The shared per-thread table behind [`crate::thread_local!`].
//!
//! Every [`LocalKey`] owns a process-wide slot index. Each thread owns one
//! [`Table`] mapping slot indices to lazily created, heap-allocated values.
//!
//! The table itself has no destructor, so it can always be read, even while the
//! thread exits. A separate [`Guard`], the only thread-local here with a
//! destructor and therefore the only one that needs an OS destructor
//! registration, runs the teardown: it drops the values one at a time and marks
//! each as destroyed just before its own destructor runs. As with std, a
//! destructor can therefore keep using every other key, and only a key that is
//! already being or has been destroyed reports [`AccessError`].
//!
//! Values are heap-allocated so their address stays fixed while the table grows
//! and while other slots are initialised from inside a `with` closure. Both the
//! values and the table are allocated with [`System`], never the global
//! allocator, so a `#[global_allocator]` may itself use `aimer_tls` (as std's
//! own thread-locals allow).

use std::alloc::{GlobalAlloc, Layout, System, handle_alloc_error};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Source of slot indices. Index `0` is never handed out, so a key's `0` means
/// "not assigned yet".
static NEXT_SLOT: AtomicUsize = AtomicUsize::new(1);

/// Error returned by [`LocalKey::try_with`] when the value is already destroyed
/// or is being destroyed because the thread is exiting.
#[derive(Clone, Copy, Eq, PartialEq, Debug)]
#[non_exhaustive]
pub struct AccessError;

impl fmt::Display for AccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("already destroyed")
    }
}

impl std::error::Error for AccessError {}

#[cold]
#[inline(never)]
#[track_caller]
fn access_panic() -> ! {
    panic!("cannot access a Thread Local Storage value during or after destruction: AccessError")
}

/// Frees a value stored in a [`Slot`].
type DropFn = unsafe fn(*mut ());

/// A type-erased heap value together with the function that frees it. A null
/// `ptr` marks a slot with no live value. All-zero bytes are a valid fresh
/// `Slot`, which lets the table's buffer come from `alloc_zeroed`.
struct Slot {
    ptr: *mut (),
    drop: Option<DropFn>,
    /// Set just before the value's destructor runs; the key then reports
    /// `AccessError` for the rest of the thread's life.
    destroyed: bool,
}

/// Moves `value` to a fresh `System` allocation and returns its address.
///
/// A zero-sized value is stored as a dangling but non-null pointer.
fn alloc_value<T>(value: T) -> *mut () {
    let layout = Layout::new::<T>();
    let ptr = if layout.size() == 0 {
        NonNull::<T>::dangling().as_ptr()
    } else {
        // SAFETY: `layout` has a non-zero size.
        let ptr = unsafe { System.alloc(layout) }.cast::<T>();
        if ptr.is_null() {
            handle_alloc_error(layout);
        }
        ptr
    };
    // SAFETY: `ptr` is valid, aligned and uninitialised for a `T`.
    unsafe { ptr.write(value) };
    ptr.cast()
}

/// # Safety
///
/// `ptr` must come from [`alloc_value::<T>`] and must not be used afterwards.
unsafe fn drop_value<T>(ptr: *mut ()) {
    let ptr = ptr.cast::<T>();
    unsafe { ptr::drop_in_place(ptr) };
    let layout = Layout::new::<T>();
    if layout.size() != 0 {
        unsafe { System.dealloc(ptr.cast(), layout) };
    }
}

/// Why [`Table::insert`] did not store a value.
enum Refused {
    /// The index already holds a live value, which wins.
    Present(*mut ()),
    /// The key is destroyed, or the whole table is gone.
    Destroyed,
}

/// All shared thread-local values of one thread.
///
/// `base` points at a `System`-allocated buffer of `len` slots (`len` doubles
/// as its capacity). The buffer is only replaced by [`Table::grow`] and freed by
/// [`Table::teardown`]; neither runs foreign code while a slot reference is
/// held, so nothing is aliased. Everything goes through raw pointers and
/// `Cell`s, which also keeps the lookup cheap in debug builds, where
/// `Vec::get` is not inlined.
struct Table {
    base: Cell<*mut Slot>,
    len: Cell<usize>,
    /// Whether this thread's [`GUARD`] has been touched, i.e. whether its
    /// destructor is registered and will run the teardown.
    registered: Cell<bool>,
    /// Set once teardown has freed the buffer; nothing can be stored after.
    finished: Cell<bool>,
}

impl Table {
    const fn new() -> Self {
        Self {
            base: Cell::new(ptr::null_mut()),
            len: Cell::new(0),
            registered: Cell::new(false),
            finished: Cell::new(false),
        }
    }

    /// The live value stored at `index`, or null if there is none.
    #[inline(always)]
    fn get(&self, index: usize) -> *mut () {
        if index < self.len.get() {
            // SAFETY: `base` holds `len` initialised slots and `index` is in
            // bounds. `ptr` is `Copy`, so no reference outlives this read.
            unsafe { (*self.base.get().add(index)).ptr }
        } else {
            ptr::null_mut()
        }
    }

    /// Whether `index` can no longer be given a value.
    fn refuses(&self, index: usize) -> bool {
        self.finished.get()
            // SAFETY: `index < len`, see `get`.
            || (index < self.len.get() && unsafe { (*self.base.get().add(index)).destroyed })
    }

    /// Stores `ptr` unless the index already holds a value or is destroyed.
    ///
    /// Initialisers may touch other keys, so a re-entrant initialisation of the
    /// same key can reach here with the slot already filled. The first value
    /// wins and the loser's pointer is handed back for the caller to free.
    fn insert(
        &self,
        index: usize,
        ptr: *mut (),
        drop: DropFn,
    ) -> Result<*mut (), Refused> {
        if self.refuses(index) {
            return Err(Refused::Destroyed);
        }
        if !self.registered.get() {
            // Touching the guard registers its destructor with the OS; that is
            // what will eventually free everything stored here.
            if GUARD.try_with(|_| ()).is_err() {
                return Err(Refused::Destroyed);
            }
            self.registered.set(true);
        }
        if index >= self.len.get() {
            self.grow(index + 1);
        }
        // SAFETY: `index < len` after `grow`.
        let slot = unsafe { &mut *self.base.get().add(index) };
        if !slot.ptr.is_null() {
            return Err(Refused::Present(slot.ptr));
        }
        slot.ptr = ptr;
        slot.drop = Some(drop);
        Ok(ptr)
    }

    /// Reallocates the buffer to hold at least `min_len` slots.
    #[cold]
    #[inline(never)]
    fn grow(&self, min_len: usize) {
        let old_len = self.len.get();
        let new_len = min_len.max(old_len * 2).max(8);
        let layout = Layout::array::<Slot>(new_len).expect("thread-local table too large");
        // SAFETY: the layout has a non-zero size; zeroed bytes are fresh slots.
        let fresh = unsafe { System.alloc_zeroed(layout) }.cast::<Slot>();
        if fresh.is_null() {
            handle_alloc_error(layout);
        }
        let old = self.base.get();
        if old_len != 0 {
            // SAFETY: `old` holds `old_len` slots from a previous `grow`, and
            // `fresh` is a distinct allocation at least as large.
            unsafe {
                ptr::copy_nonoverlapping(old, fresh, old_len);
                System.dealloc(old.cast(), Layout::array::<Slot>(old_len).unwrap());
            }
        }
        self.base.set(fresh);
        self.len.set(new_len);
    }

    /// Takes the next live value, newest slot first, and marks it destroyed.
    fn take_next(&self) -> Option<(*mut (), DropFn)> {
        for index in (0..self.len.get()).rev() {
            // SAFETY: `index < len`; the reference ends with this iteration.
            let slot = unsafe { &mut *self.base.get().add(index) };
            if let Some(drop) = slot.drop
                && !slot.ptr.is_null()
            {
                let ptr = slot.ptr;
                slot.ptr = ptr::null_mut();
                slot.drop = None;
                slot.destroyed = true;
                return Some((ptr, drop));
            }
        }
        None
    }

    /// Drops every value, then frees the buffer.
    ///
    /// A destructor runs with the table in a consistent state, so it may read
    /// other keys, even initialise new ones; those are picked up by the next
    /// `take_next`.
    fn teardown(&self) {
        while let Some((ptr, drop)) = self.take_next() {
            // SAFETY: `ptr` and `drop` were stored together by `insert`, and
            // `take_next` removed them, so the value is freed exactly once. A
            // panic unwinds into std's TLS destructor shim, which aborts the
            // process just as it does for std's own keys.
            unsafe { drop(ptr) };
        }
        self.finished.set(true);
        let base = self.base.replace(ptr::null_mut());
        let len = self.len.replace(0);
        if len != 0 {
            // SAFETY: `base` came from `grow` with this `len`.
            unsafe { System.dealloc(base.cast(), Layout::array::<Slot>(len).unwrap()) };
        }
    }
}

/// Runs [`Table::teardown`] when the thread exits.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        TABLE.with(Table::teardown);
    }
}

std::thread_local! {
    /// Every shared thread-local's storage. It has no destructor, so it needs
    /// no OS destructor registration and stays readable during teardown.
    static TABLE: Table = const { Table::new() };

    /// Its destructor tears [`TABLE`] down; the only one in this crate.
    static GUARD: Guard = const { Guard };
}

/// A thread-local value stored in the shared per-thread table.
///
/// Created by [`crate::thread_local!`]; it mirrors [`std::thread::LocalKey`].
pub struct LocalKey<T: 'static> {
    slot: AtomicUsize,
    init: fn() -> T,
}

impl<T: 'static> fmt::Debug for LocalKey<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalKey").finish_non_exhaustive()
    }
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

    #[inline(always)]
    fn index(&self) -> usize {
        let current = self.slot.load(Ordering::Relaxed);
        if current != 0 {
            return current;
        }
        self.assign_index()
    }

    #[cold]
    #[inline(never)]
    fn assign_index(&self) -> usize {
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
    /// The address stays valid until the value's destructor starts.
    #[inline(always)]
    fn value(&'static self) -> Result<*const T, AccessError> {
        let index = self.index();
        let ptr = TABLE.with(|table| table.get(index));
        if !ptr.is_null() {
            return Ok(ptr.cast::<T>());
        }
        self.initialise(index)
    }

    #[cold]
    #[inline(never)]
    fn initialise(&'static self, index: usize) -> Result<*const T, AccessError> {
        if TABLE.with(|table| table.refuses(index)) {
            return Err(AccessError);
        }
        // The initialiser may itself use thread-locals, so it runs with the
        // table untouched.
        self.store(index, (self.init)())
    }

    /// Moves `value` into the table, unless a value got there first.
    fn store(&'static self, index: usize, value: T) -> Result<*const T, AccessError> {
        let raw = alloc_value(value);
        match TABLE.with(|table| table.insert(index, raw, drop_value::<T>)) {
            Ok(ptr) => Ok(ptr.cast::<T>()),
            Err(Refused::Present(winner)) => {
                // SAFETY: `raw` came from `alloc_value` just above and was not
                // stored.
                unsafe { drop_value::<T>(raw) };
                Ok(winner.cast::<T>())
            }
            Err(Refused::Destroyed) => {
                // SAFETY: as above.
                unsafe { drop_value::<T>(raw) };
                Err(AccessError)
            }
        }
    }

    /// Stores `fresh` if this thread has no value yet, without running the
    /// initialiser; otherwise calls `apply` with `fresh` and the live value.
    ///
    /// This is what lets `set` skip the initialiser, as std does.
    fn set_or_apply(&'static self, fresh: T, apply: impl FnOnce(T, &T)) {
        let index = self.index();
        let found = TABLE.with(|table| table.get(index));
        if found.is_null() {
            if self.store(index, fresh).is_err() {
                access_panic();
            }
        } else {
            // SAFETY: see `try_with`.
            apply(fresh, unsafe { &*found.cast::<T>() });
        }
    }

    /// Runs `f` with a reference to this thread's value.
    ///
    /// # Panics
    ///
    /// Panics if the value is being or has been destroyed.
    #[inline(always)]
    pub fn with<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        match self.value() {
            // SAFETY: see `try_with`.
            Ok(ptr) => f(unsafe { &*ptr }),
            Err(_) => access_panic(),
        }
    }

    /// Like [`with`](Self::with), but returns [`AccessError`] instead of
    /// panicking when the value is being or has been destroyed.
    #[inline(always)]
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let ptr = self.value()?;
        // SAFETY: the value is heap-allocated, owned by this thread's table,
        // and freed only by that value's own teardown, which cannot start
        // while this thread is inside `f`.
        Ok(f(unsafe { &*ptr }))
    }
}

impl<T: 'static> LocalKey<Cell<T>> {
    /// Replaces the value, dropping the old one. Does not run the initialiser.
    #[inline(always)]
    pub fn set(&'static self, value: T) {
        self.set_or_apply(Cell::new(value), |fresh, cell| cell.set(fresh.into_inner()));
    }

    /// Returns a copy of the value.
    #[inline(always)]
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(Cell::get)
    }

    /// Updates the value in place with `f`.
    #[inline(always)]
    pub fn update(&'static self, f: impl FnOnce(T) -> T)
    where
        T: Copy,
    {
        self.with(|cell| cell.update(f));
    }

    /// Takes the value, leaving `Default::default()` behind.
    #[inline(always)]
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(Cell::take)
    }

    /// Replaces the value, returning the old one.
    #[inline(always)]
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
    #[inline(always)]
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
    #[inline(always)]
    pub fn with_borrow_mut<F, R>(&'static self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        self.with(|cell| f(&mut cell.borrow_mut()))
    }

    /// Replaces the value, dropping the old one. Does not run the initialiser.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline(always)]
    pub fn set(&'static self, value: T) {
        self.set_or_apply(RefCell::new(value), |fresh, cell| {
            *cell.borrow_mut() = fresh.into_inner();
        });
    }

    /// Takes the value, leaving `Default::default()` behind.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline(always)]
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
    #[inline(always)]
    pub fn replace(&'static self, value: T) -> T {
        self.with(|cell| cell.replace(value))
    }
}

#[cfg(test)]
mod tests;
