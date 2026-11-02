//! The "hot thread" fast path: one thread skips std's TLS lookup entirely.
//!
//! Every key access starts by finding this thread's [`Table`], which costs one
//! `std::thread_local!` lookup (on Apple targets a call through
//! `__tlv_get_addr`). A thread that does most of the work, such as the UI
//! thread, can register itself with [`hot`]. While the returned [`HotGuard`]
//! lives, the table's address sits in a process-wide cell, and an access only
//! has to prove that it runs on that thread.
//!
//! The proof is the thread pointer register (`tpidr_el0`, `fs`, `gs`), read
//! with one instruction: it is unique to every live thread, needs no memory
//! access and, unlike a thread-local, no lookup. If it equals the registered
//! owner's, the cell's table is the caller's own table. Any other thread, and
//! every thread while no owner is registered, takes the ordinary path, so
//! behaviour never changes, only speed.
//!
//! # Soundness
//!
//! * `OWNER` is claimed by compare-exchange, so at most one thread owns the
//!   fast path at a time; later claimants get an inactive guard.
//! * `HOT_TABLE` is only read after the reader's own thread pointer matched
//!   `OWNER`, which only the owner can satisfy, and only written by the thread
//!   that holds `OWNER` (or just claimed it). The acquire on the claim and the
//!   release on the clear order consecutive owners, so it is never raced.
//! * The table is a `const` std thread-local without a destructor, so its
//!   address is stable for the thread's whole life. The owner is cleared by the
//!   guard's drop and, as a backstop for a leaked guard, by the thread's
//!   teardown, before the thread (and with it the table) can go away.
//! * A thread pointer value is only compared while its thread is alive, and
//!   the owner is cleared before that thread dies, so a recycled value cannot
//!   match a stale owner.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{TABLE, Table, load_relaxed};

/// Whether this target can read its thread pointer.
pub(super) const SUPPORTED: bool = cfg!(any(
    all(
        target_arch = "aarch64",
        any(target_os = "linux", target_os = "android", target_vendor = "apple")
    ),
    all(
        target_arch = "x86_64",
        any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            windows
        )
    ),
));

/// The thread pointer of the thread that owns the fast path, or `0`.
static OWNER: AtomicUsize = AtomicUsize::new(0);

/// A plain cell shared between threads; see the module docs for why access is
/// never concurrent.
struct HotTable(UnsafeCell<*const Table>);

// SAFETY: access is serialised through `OWNER`, see the module docs.
unsafe impl Sync for HotTable {}

static HOT_TABLE: HotTable = HotTable(UnsafeCell::new(std::ptr::null()));

/// A value unique to the calling thread for as long as it lives.
#[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "android")))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reading the thread register has no side effects.
    unsafe {
        std::arch::asm!(
            "mrs {}, tpidr_el0",
            out(reg) tp,
            options(nomem, nostack, preserves_flags),
        );
    }
    tp
}

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reading the thread register has no side effects.
    unsafe {
        std::arch::asm!(
            "mrs {}, tpidrro_el0",
            out(reg) tp,
            options(nomem, nostack, preserves_flags),
        );
    }
    // The low bits carry the CPU number and change as the thread migrates.
    tp & !7
}

#[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "android")))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: `fs:0` holds the thread control block's self pointer.
    unsafe {
        std::arch::asm!(
            "mov {}, fs:0",
            out(reg) tp,
            options(nostack, preserves_flags, readonly),
        );
    }
    tp
}

#[cfg(all(target_arch = "x86_64", target_vendor = "apple"))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: `gs:0` holds the thread's self pointer.
    unsafe {
        std::arch::asm!(
            "mov {}, gs:0",
            out(reg) tp,
            options(nostack, preserves_flags, readonly),
        );
    }
    tp
}

#[cfg(all(target_arch = "x86_64", windows))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: `gs:[0x30]` is the thread environment block's self pointer.
    unsafe {
        std::arch::asm!(
            "mov {}, gs:[0x30]",
            out(reg) tp,
            options(nostack, preserves_flags, readonly),
        );
    }
    tp
}

#[cfg(not(any(
    all(
        target_arch = "aarch64",
        any(target_os = "linux", target_os = "android", target_vendor = "apple")
    ),
    all(
        target_arch = "x86_64",
        any(
            target_os = "linux",
            target_os = "android",
            target_vendor = "apple",
            windows
        )
    ),
)))]
#[inline(always)]
pub(super) fn thread_pointer() -> usize {
    0
}

/// The current thread's table if it owns the fast path, otherwise null.
#[inline(always)]
pub(super) fn current() -> *const Table {
    if SUPPORTED {
        // Checking for "no owner" first spares every thread, while nobody owns
        // the fast path, the thread register read.
        let owner = super::load_relaxed(&OWNER);
        if owner != 0 && owner == thread_pointer() {
            // SAFETY: only the owner thread passes the check above, and only
            // the owner writes the cell, see the module docs.
            return unsafe { *HOT_TABLE.0.get() };
        }
    }
    std::ptr::null()
}

/// Releases the fast path if the calling thread owns it.
///
/// The per-key caches of this thread are cleared first, so no key can hand
/// out a pointer for a thread that no longer owns the fast path.
pub(super) fn release_thread(table: &Table) {
    if !SUPPORTED {
        return;
    }
    let tp = thread_pointer();
    if tp != 0 && OWNER.load(Ordering::Relaxed) == tp {
        table.clear_hot();
        // SAFETY: this thread owns the cell. Clearing it first keeps a reader
        // from ever seeing an owner without a table.
        unsafe { *HOT_TABLE.0.get() = std::ptr::null() };
        OWNER.store(0, Ordering::Release);
    }
}

/// Whether the calling thread owns the fast path.
#[inline(always)]
pub(super) fn owns_current_thread() -> bool {
    if !SUPPORTED {
        return false;
    }
    let tp = thread_pointer();
    tp != 0 && load_relaxed(&OWNER) == tp
}

/// Makes the calling thread the owner if nobody is, as soon as it creates
/// its first thread-local value.
///
/// The first thread to use thread-locals is in practice the one that does
/// most of the work. Without this, a program has to remember to call [`hot`]
/// on the right thread. The thread's teardown releases it.
///
/// Compiled out of this crate's own unit tests: they run on many threads in
/// one process and assert on who owns the fast path.
pub(super) fn auto_claim(table: &Table) {
    if !SUPPORTED || cfg!(test) {
        return;
    }
    let tp = thread_pointer();
    if tp == 0 {
        return;
    }
    if OWNER
        .compare_exchange(0, tp, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        // SAFETY: the claim above made this thread the only writer.
        unsafe { *HOT_TABLE.0.get() = std::ptr::from_ref(table) };
    }
}

/// One key's cached value pointer for the thread that owns the fast path.
///
/// Only the owner thread ever writes it, `ptr` before `tp`, and it clears it
/// before it stops owning (see [`release_thread`]). A thread reads `ptr` only
/// after `tp` equals its own thread pointer, which only the owner can satisfy
/// (a thread pointer is unique among live threads and a dead owner's caches
/// were cleared by its teardown), so the pair is never read torn.
pub struct HotCache {
    tp: AtomicUsize,
    ptr: AtomicUsize,
}

impl HotCache {
    pub(super) const fn new() -> Self {
        Self {
            tp: AtomicUsize::new(0),
            ptr: AtomicUsize::new(0),
        }
    }

    /// The calling thread's cached value address, or null.
    #[inline(always)]
    pub(super) fn get(&self) -> usize {
        if SUPPORTED && load_relaxed(&self.tp) == thread_pointer() {
            return load_relaxed(&self.ptr);
        }
        0
    }

    /// Caches `ptr` for the calling thread, which must own the fast path.
    pub(super) fn set(&self, ptr: usize) {
        self.ptr.store(ptr, Ordering::Relaxed);
        self.tp.store(thread_pointer(), Ordering::Relaxed);
    }

    pub(super) fn clear(&self) {
        self.tp.store(0, Ordering::Relaxed);
        self.ptr.store(0, Ordering::Relaxed);
    }
}

fn try_claim() -> bool {
    if !SUPPORTED {
        return false;
    }
    let tp = thread_pointer();
    if tp == 0 {
        return false;
    }
    // `TABLE` has no destructor, so it can always be reached.
    TABLE.with(|table| {
        // The backstop for a leaked guard is the thread's teardown, which
        // only exists once the guard thread-local is registered.
        if !table.register() {
            return false;
        }
        // Already the owner, through `auto_claim`.
        if load_relaxed(&OWNER) == tp {
            return false;
        }
        if OWNER
            .compare_exchange(0, tp, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return false;
        }
        // SAFETY: the claim above made this thread the only writer.
        unsafe { *HOT_TABLE.0.get() = std::ptr::from_ref(table) };
        true
    })
}

/// Keeps the calling thread on the fast path while it lives.
///
/// Created by [`hot`]. It cannot leave its thread.
#[must_use = "the fast path ends as soon as the guard is dropped"]
pub struct HotGuard {
    active: bool,
    _not_send: PhantomData<*const ()>,
}

impl HotGuard {
    /// Whether this guard put its thread on the fast path.
    ///
    /// `false` when the target has no supported thread register, when another
    /// thread or an outer guard on this thread already owns the fast path, or
    /// when the thread is exiting. The program is correct either way.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for HotGuard {
    fn drop(&mut self) {
        if self.active {
            // `TABLE` has no destructor, so it can always be reached.
            TABLE.with(release_thread);
        }
    }
}

/// Puts the calling thread on the fast path until the guard is dropped.
///
/// While one thread holds an active guard, all of its thread-local accesses,
/// through [`LocalKey`](crate::LocalKey) and [`scope`](crate::scope) alike,
/// skip the platform's TLS lookup. Other threads are unaffected. Only one
/// thread can hold the fast path at a time; the rest get an inactive guard,
/// as does a nested call on the owning thread. Call it once around the work of
/// the thread that touches thread-locals the most, such as the UI thread's
/// event loop.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
///
/// aimer_tls::thread_local! {
///     static FRAMES: Cell<u32> = const { Cell::new(0) };
/// }
///
/// let _hot = aimer_tls::hot();
/// FRAMES.set(FRAMES.get() + 1);
/// assert_eq!(FRAMES.get(), 1);
/// ```
pub fn hot() -> HotGuard {
    HotGuard {
        active: try_claim(),
        _not_send: PhantomData,
    }
}

/// Whether this target can use the fast path at all.
///
/// When `false`, [`hot`] always returns an inactive guard.
#[inline]
pub const fn hot_supported() -> bool {
    SUPPORTED
}
