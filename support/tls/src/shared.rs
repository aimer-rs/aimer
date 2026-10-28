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
use std::cell::{Cell, RefCell, UnsafeCell};
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

/// A `Cell` whose accessors are `inline(always)`.
///
/// `Cell::get` and friends are plain `#[inline]`, which a debug build turns
/// into real calls; the lookup path reads these fields on every access.
struct Field<T: Copy>(UnsafeCell<T>);

impl<T: Copy> Field<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    #[inline(always)]
    fn get(&self) -> T {
        // SAFETY: the table is thread-confined and no reference into the cell
        // is ever handed out, so this read cannot overlap a write.
        unsafe { *self.0.get() }
    }

    #[inline(always)]
    fn set(&self, value: T) {
        // SAFETY: as in `get`.
        unsafe { *self.0.get() = value };
    }

    #[inline(always)]
    fn replace(&self, value: T) -> T {
        let old = self.get();
        self.set(value);
        old
    }
}

/// Reads an atomic with relaxed ordering.
///
/// A relaxed load is a plain aligned load on these targets, but
/// `AtomicUsize::load` is several non-inlined calls in a debug build, which
/// dominated the lookup. The asm block is that same single load.
///
/// Optimised builds use the plain atomic load instead: an asm block is opaque
/// to LLVM, which can then neither merge the loads of several keys nor hoist
/// them out of a loop, while it handles a relaxed atomic load fine.
#[inline(always)]
fn load_relaxed(atomic: &AtomicUsize) -> usize {
    #[cfg(all(debug_assertions, target_arch = "aarch64"))]
    {
        let value: usize;
        // SAFETY: `atomic` is a valid, aligned `usize`; a word-sized aligned
        // load cannot tear, and the block reads memory only.
        unsafe {
            std::arch::asm!(
                "ldr {value}, [{ptr}]",
                value = out(reg) value,
                ptr = in(reg) atomic as *const AtomicUsize,
                options(nostack, preserves_flags, readonly),
            );
        }
        value
    }
    #[cfg(all(debug_assertions, target_arch = "x86_64"))]
    {
        let value: usize;
        // SAFETY: as above.
        unsafe {
            std::arch::asm!(
                "mov {value}, qword ptr [{ptr}]",
                value = out(reg) value,
                ptr = in(reg) atomic as *const AtomicUsize,
                options(nostack, preserves_flags, readonly),
            );
        }
        value
    }
    #[cfg(not(all(debug_assertions, any(target_arch = "aarch64", target_arch = "x86_64"))))]
    {
        atomic.load(Ordering::Relaxed)
    }
}

/// How many slots are mirrored inline in [`Table::fast`].
const FAST: usize = 128;

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
    /// The key's [`hot::HotCache`] if this thread cached the value there.
    hot: *const hot::HotCache,
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
    /// Mirror of the live value pointers of the first [`FAST`] slots, kept
    /// inline so a hit costs one indexed load instead of reading `len` and
    /// `base` first. `slots` stays the source of truth for everything else.
    fast: [Field<*mut ()>; FAST],
    base: Field<*mut Slot>,
    len: Field<usize>,
    /// Whether this thread's [`GUARD`] has been touched, i.e. whether its
    /// destructor is registered and will run the teardown.
    registered: Field<bool>,
    /// Set once teardown has freed the buffer; nothing can be stored after.
    finished: Field<bool>,
}

impl Table {
    const fn new() -> Self {
        Self {
            fast: [const { Field::new(ptr::null_mut()) }; FAST],
            base: Field::new(ptr::null_mut()),
            len: Field::new(0),
            registered: Field::new(false),
            finished: Field::new(false),
        }
    }

    /// The live value stored at `index`, or null if there is none.
    #[inline(always)]
    fn get(&self, index: usize) -> *mut () {
        if index < FAST {
            // SAFETY: `index < FAST`.
            return unsafe { self.fast.get_unchecked(index) }.get();
        }
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

    /// Makes sure this thread's teardown is registered with the OS, which is
    /// what will eventually free everything stored here. Returns `false` if
    /// the thread is already past that point.
    #[inline]
    fn register(&self) -> bool {
        if self.registered.get() {
            return true;
        }
        // Touching the guard registers its destructor.
        if GUARD.try_with(|_| ()).is_err() {
            return false;
        }
        self.registered.set(true);
        hot::auto_claim(self);
        true
    }

    /// Records that `cache` holds a pointer to the value at `index`, so
    /// [`clear_hot`](Self::clear_hot) can find it.
    fn note_hot(&self, index: usize, cache: *const hot::HotCache) {
        if index < self.len.get() {
            // SAFETY: `index < len`.
            unsafe { (*self.base.get().add(index)).hot = cache };
        }
    }

    /// Empties every per-key cache this thread filled.
    fn clear_hot(&self) {
        for index in 0..self.len.get() {
            // SAFETY: `index < len`; the reference ends with this iteration.
            let slot = unsafe { &mut *self.base.get().add(index) };
            if !slot.hot.is_null() {
                // SAFETY: a key is `'static`, so its cache outlives the table.
                unsafe { &*slot.hot }.clear();
                slot.hot = ptr::null();
            }
        }
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
        if !self.register() {
            return Err(Refused::Destroyed);
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
        if index < FAST {
            self.fast[index].set(ptr);
        }
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
                if index < FAST {
                    self.fast[index].set(ptr::null_mut());
                }
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
        // A leaked `HotGuard` must not outlive the table it points at.
        TABLE.with(|table| {
            hot::release_thread(table);
            table.teardown();
        });
    }
}

/// Runs `f` with this thread's table, skipping std's TLS lookup when the
/// thread owns the [`hot`] fast path.
#[inline(always)]
fn with_table<R>(f: impl FnOnce(&Table) -> R) -> R {
    // SAFETY: `table_ptr` points at this thread's table, which outlives `f`.
    f(unsafe { &*table_ptr() })
}

/// This thread's table. It is a `const` thread-local without a destructor, so
/// the address stays valid for the thread's whole life, teardown included.
#[inline(always)]
fn table_ptr() -> *const Table {
    // The hot path pays off in a debug build, where it skips non-inlined std
    // calls. An optimised build is better off without it: the owner check
    // splits every lookup into two branches, so LLVM can no longer merge the
    // table address of several keys read in a row into one TLS lookup, which
    // is exactly what it does for plain `std::thread_local!` keys.
    if cfg!(debug_assertions) {
        let owned = hot::current();
        if owned as usize != 0 {
            // `current` only returns the calling thread's own table, which
            // stays alive until the owner is released, see the `hot` module
            // docs.
            return owned;
        }
    }
    // `try_with` rather than `with`, which adds an `expect` call that a debug
    // build does not inline. A const thread-local without a destructor never
    // reports an error.
    //
    // The closure is kept on purpose: it inlines `from_ref`, where passing the
    // function itself adds a `call_once` shim call in a debug build.
    #[allow(clippy::redundant_closure)]
    match TABLE.try_with(|table| ptr::from_ref(table)) {
        Ok(table) => table,
        Err(_) => access_panic(),
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
    hot: hot::HotCache,
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
            hot: hot::HotCache::new(),
        }
    }

    #[inline(always)]
    fn index(&self) -> usize {
        let current = load_relaxed(&self.slot);
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

    /// Address of this thread's value in `table`, creating it on first
    /// access.
    ///
    /// The address stays valid until the value's destructor starts. Taking the
    /// table as a parameter is what lets a [`Scope`] resolve it once for many
    /// keys.
    #[inline(always)]
    fn value_in(&'static self, table: &Table) -> Result<*const T, AccessError> {
        let index = self.index();
        let ptr = table.get(index);
        if ptr as usize != 0 {
            return Ok(ptr.cast::<T>());
        }
        self.initialise(table, index)
    }

    /// Like [`value_in`](Self::value_in) but panics instead of returning an
    /// error. The hit path is a bare pointer test with no `Result` to build or
    /// unwrap, which matters in debug builds where those calls are not inlined.
    #[inline(always)]
    fn ptr_in(&'static self, table: &Table) -> *const T {
        let index = self.index();
        let ptr = table.get(index);
        if ptr as usize != 0 {
            return ptr.cast::<T>();
        }
        self.ptr_slow(table, index)
    }

    #[cold]
    #[inline(never)]
    fn ptr_slow(&'static self, table: &Table, index: usize) -> *const T {
        match self.initialise(table, index) {
            Ok(ptr) => ptr,
            Err(_) => access_panic(),
        }
    }

    #[cold]
    #[inline(never)]
    fn initialise(&'static self, table: &Table, index: usize) -> Result<*const T, AccessError> {
        if table.refuses(index) {
            return Err(AccessError);
        }
        // The initialiser may itself use thread-locals, so it runs with the
        // table untouched.
        self.store(table, index, (self.init)())
    }

    /// Moves `value` into the table, unless a value got there first.
    fn store(&'static self, table: &Table, index: usize, value: T) -> Result<*const T, AccessError> {
        let raw = alloc_value(value);
        match table.insert(index, raw, drop_value::<T>) {
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

    /// This thread's live value in `table`, or null if it has none. Never runs
    /// the initialiser, which is what lets `set` skip it, as std does.
    #[inline(always)]
    fn lookup_in(&self, table: &Table) -> *const T {
        table.get(self.index()).cast::<T>()
    }

    /// Stores `fresh` as this thread's value, for a key whose
    /// [`lookup_in`](Self::lookup_in) found nothing.
    ///
    /// # Panics
    ///
    /// Panics if the key is being or has been destroyed.
    #[cold]
    #[inline(never)]
    fn store_fresh(&'static self, table: &Table, fresh: T) {
        let index = self.index();
        if self.store(table, index, fresh).is_err() {
            access_panic();
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
        f(self.local_ref())
    }

    /// This thread's value, creating it on first use, without a closure layer:
    /// in a debug build every closure is a real call, so the helpers below read
    /// the value directly instead of going through [`with`](Self::with).
    ///
    /// The lifetime is unbounded, so callers must not let the reference escape
    /// their own body: the value is freed by this thread's teardown, which
    /// cannot start while one of them is running.
    ///
    /// # Panics
    ///
    /// Panics if the value is being or has been destroyed.
    #[inline(always)]
    fn local_ref<'a>(&'static self) -> &'a T {
        let cached = self.hot.get();
        if cached != 0 {
            // SAFETY: only the owner thread gets a non-zero cache, and the
            // cache is cleared before the value can be destroyed.
            return unsafe { &*(cached as *const T) };
        }
        // SAFETY: the value is heap-allocated and owned by this thread's
        // table, see `try_with`.
        unsafe { &*self.resolve() }
    }

    /// The owner thread's cached address of the value, or `0`.
    #[inline(always)]
    fn hot_cached(&self) -> usize {
        self.hot.get()
    }

    /// Looks the value up through the table, and caches it for the owner.
    #[inline(never)]
    fn resolve(&'static self) -> *const T {
        // SAFETY: the table outlives this call, see `table_ptr`.
        let table = unsafe { &*table_ptr() };
        let ptr = self.ptr_in(table);
        self.cache(table, ptr);
        ptr
    }

    /// Caches `ptr`, this thread's value of the key, if the thread owns the
    /// fast path.
    #[inline(always)]
    fn cache(&'static self, table: &Table, ptr: *const T) {
        if hot::owns_current_thread() {
            table.note_hot(self.index(), &self.hot);
            self.hot.set(ptr as usize);
        }
    }

    /// Like [`with`](Self::with), but returns [`AccessError`] instead of
    /// panicking when the value is being or has been destroyed.
    #[inline(always)]
    pub fn try_with<F, R>(&'static self, f: F) -> Result<R, AccessError>
    where
        F: FnOnce(&T) -> R,
    {
        let cached = self.hot.get();
        if cached != 0 {
            // SAFETY: see `local_ref`.
            return Ok(f(unsafe { &*(cached as *const T) }));
        }
        // SAFETY: the table outlives this call, see `table_ptr`.
        let table = unsafe { &*table_ptr() };
        let index = self.index();
        let ptr = table.get(index);
        // SAFETY (both arms): the value is heap-allocated, owned by this
        // thread's table, and freed only by that value's own teardown, which
        // cannot start while this thread is inside `f`.
        if ptr as usize != 0 {
            self.cache(table, ptr.cast::<T>());
            return Ok(f(unsafe { &*ptr.cast::<T>() }));
        }
        // A `match` rather than `?`: the `Try` calls are not inlined in debug
        // builds.
        match self.initialise(table, index) {
            Ok(ptr) => {
                self.cache(table, ptr);
                Ok(f(unsafe { &*ptr }))
            }
            Err(error) => Err(error),
        }
    }
}

impl<T: 'static> LocalKey<Cell<T>> {
    /// Replaces the value, dropping the old one. Does not run the initialiser.
    #[inline(always)]
    pub fn set(&'static self, value: T) {
        let cached = self.hot.get();
        if cached != 0 {
            // SAFETY: see `local_ref`.
            unsafe { &*(cached as *const Cell<T>) }.set(value);
            return;
        }
        // SAFETY: the table outlives this call, see `table_ptr`.
        let table = unsafe { &*table_ptr() };
        let found = self.lookup_in(table);
        if found as usize == 0 {
            self.store_fresh(table, Cell::new(value));
        } else {
            // SAFETY: see `try_with`.
            unsafe { &*found }.set(value);
        }
    }

    /// Returns a copy of the value.
    #[inline(always)]
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.local_ref().get()
    }

    /// Updates the value in place with `f`.
    #[inline(always)]
    pub fn update(&'static self, f: impl FnOnce(T) -> T)
    where
        T: Copy,
    {
        self.local_ref().update(f);
    }

    /// Takes the value, leaving `Default::default()` behind.
    #[inline(always)]
    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.local_ref().take()
    }

    /// Replaces the value, returning the old one.
    #[inline(always)]
    pub fn replace(&'static self, value: T) -> T {
        self.local_ref().replace(value)
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
        f(&self.local_ref().borrow())
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
        f(&mut self.local_ref().borrow_mut())
    }

    /// Replaces the value, dropping the old one. Does not run the initialiser.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline(always)]
    pub fn set(&'static self, value: T) {
        let cached = self.hot.get();
        if cached != 0 {
            // SAFETY: see `local_ref`.
            *unsafe { &*(cached as *const RefCell<T>) }.borrow_mut() = value;
            return;
        }
        // SAFETY: the table outlives this call, see `table_ptr`.
        let table = unsafe { &*table_ptr() };
        let found = self.lookup_in(table);
        if found as usize == 0 {
            self.store_fresh(table, RefCell::new(value));
        } else {
            // SAFETY: see `try_with`.
            *unsafe { &*found }.borrow_mut() = value;
        }
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
        self.local_ref().take()
    }

    /// Replaces the value, returning the old one.
    ///
    /// # Panics
    ///
    /// Panics if the value is currently borrowed.
    #[inline(always)]
    pub fn replace(&'static self, value: T) -> T {
        self.local_ref().replace(value)
    }
}

mod hot;
mod scope;

pub use hot::{HotGuard, hot, hot_supported};
pub use scope::{Scope, scope};

#[cfg(test)]
mod tests;
