//! Application-owned heap storage for UI allocations.

use std::alloc::Layout;
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;

use allocator_api2::alloc::{AllocError, Allocator};
use dlmalloc::Dlmalloc;

use crate::page_backend::{MemoryBudget, PageBackend, growth_granularity};
use crate::pool::{self, EMPTY, UNPOOLED};

struct UiMemoryInner {
    rubick_cache: UiBlockCache,
    heap: RefCell<Dlmalloc<PageBackend>>,
    budget: Arc<MemoryBudget>,
}

struct UiBlockCache {
    heads: [Cell<*mut u8>; pool::CLASS_SIZES.len()],
    counts: [Cell<usize>; pool::CLASS_SIZES.len()],
}

impl UiBlockCache {
    const fn new() -> Self {
        Self {
            heads: [const { Cell::new(std::ptr::null_mut()) }; pool::CLASS_SIZES.len()],
            counts: [const { Cell::new(0) }; pool::CLASS_SIZES.len()],
        }
    }

    #[inline(always)]
    fn pop(&self, class: u8) -> Option<NonNull<u8>> {
        let index = class as usize;
        let head = NonNull::new(self.heads[index].get())?;
        // SAFETY: every cached block is at least two words long and stores the
        // next pointer in its first word, written by `push`.
        let next = unsafe { head.as_ptr().cast::<*mut u8>().read() };
        self.heads[index].set(next);
        self.counts[index].set(self.counts[index].get() - 1);
        Some(head)
    }

    #[inline(always)]
    fn push(&self, class: u8, block: NonNull<u8>) -> bool {
        let index = class as usize;
        let count = self.counts[index].get();
        if count >= pool::class_capacity(pool::CLASS_SIZES[index]) {
            return false;
        }
        // SAFETY: every pooled class is large and aligned enough for a pointer;
        // the value in the block has already been dropped.
        unsafe {
            block
                .as_ptr()
                .cast::<*mut u8>()
                .write(self.heads[index].get())
        };
        self.heads[index].set(block.as_ptr());
        self.counts[index].set(count + 1);
        true
    }
}

/// Owns one bounded UI heap and the system pages it acquires.
///
/// The heap starts without acquired pages and grows in regions of up to
/// 2 MiB, rounded to the operating system's page size. Cloning a [`UiMemory`]
/// keeps the same heap alive. On WebAssembly, committed linear memory cannot
/// shrink, so `committed_bytes` reports the pool's high-water usage.
#[derive(Clone)]
pub struct UiMemory {
    inner: Rc<UiMemoryInner>,
}

/// A cloneable allocator handle for [`UiMemory`].
///
/// The handle implements `allocator_api2::alloc::Allocator`, so it can back
/// `allocator_api2::boxed::Box` and `allocator_api2::vec::Vec`. It is confined
/// to the UI thread, like Rubick's type-erased owners.
#[derive(Clone)]
pub struct UiAllocator {
    inner: Rc<UiMemoryInner>,
}

thread_local! {
    static ACTIVE_UI_MEMORY: Cell<*const UiMemoryInner> = const { Cell::new(std::ptr::null()) };
}

impl UiMemory {
    /// Creates an empty UI heap whose acquired page regions cannot exceed
    /// `max_bytes`.
    ///
    /// A zero limit creates a valid but non-allocating pool. Pages are
    /// requested lazily, on the first non-zero allocation. On Unix, this limit
    /// caps mapped bytes; the OS may back anonymous mappings lazily, so it does
    /// not cap the process's resident memory.
    pub fn new(max_bytes: usize) -> Self {
        let backend = PageBackend::new(max_bytes);
        let budget = backend.budget();
        let granularity = growth_granularity(max_bytes, backend.page_size());
        let mut heap = Dlmalloc::new_with_allocator(backend);
        debug_assert!(heap.set_granularity(granularity));

        Self {
            inner: Rc::new(UiMemoryInner {
                rubick_cache: UiBlockCache::new(),
                heap: RefCell::new(heap),
                budget,
            }),
        }
    }

    /// Returns a cloneable allocator for this heap.
    #[inline]
    pub fn allocator(&self) -> UiAllocator {
        UiAllocator {
            inner: Rc::clone(&self.inner),
        }
    }

    /// Returns the page bytes currently acquired by this pool.
    ///
    /// On Unix this counts mapped bytes, whose physical backing may be lazy.
    /// On WebAssembly this reports the pool's high-water linear-memory usage.
    #[inline]
    pub fn committed_bytes(&self) -> usize {
        self.inner.budget.committed()
    }

    /// Returns the maximum page bytes this pool may acquire.
    #[inline]
    pub fn limit_bytes(&self) -> usize {
        self.inner.budget.limit()
    }
}

impl Default for UiMemory {
    fn default() -> Self {
        Self::new(usize::MAX)
    }
}

impl UiAllocator {
    /// Runs `f` with this allocator active for Rubick heap allocations on the
    /// current thread.
    ///
    /// Scopes nest: when `f` returns or unwinds, the allocator that was active
    /// before this call is restored. Heap-backed Rubick values retain an
    /// allocator handle and can be dropped after the scope ends.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        let _scope = self.enter_scope();
        f()
    }

    /// Returns the page bytes currently acquired by this pool.
    ///
    /// On Unix this counts mapped bytes, whose physical backing may be lazy.
    /// On WebAssembly this reports the pool's high-water linear-memory usage.
    #[inline]
    pub fn committed_bytes(&self) -> usize {
        self.inner.budget.committed()
    }

    /// Returns the maximum page bytes this pool may acquire.
    #[inline]
    pub fn limit_bytes(&self) -> usize {
        self.inner.budget.limit()
    }

    /// Returns the allocator active on the current thread, if a build or an
    /// explicit [`UiAllocator::scope`] is running.
    #[inline]
    pub fn current() -> Option<Self> {
        let inner = Self::current_inner()?;
        // SAFETY: The active scope owns a strong allocator handle while the
        // thread-local pointer is set, so the Rc allocation is live here.
        Some(unsafe { Self::clone_from_inner(inner) })
    }

    /// Returns the allocator active on the current thread, if a build or an
    /// explicit [`UiAllocator::scope`] is running.
    #[inline]
    pub unsafe fn current_unchecked() -> Self {
        // SAFETY: The active scope owns a strong allocator handle while the
        // thread-local pointer is set, so the Rc allocation is live here.
        let inner = unsafe { Self::current_inner().unwrap_unchecked() };
        // SAFETY: The active scope owns a strong allocator handle while the
        // thread-local pointer is set, so the Rc allocation is live here.
        unsafe { Self::clone_from_inner(inner) }
    }

    #[inline(always)]
    fn current_inner() -> Option<NonNull<UiMemoryInner>> {
        ACTIVE_UI_MEMORY
            .try_with(|inner| NonNull::new(inner.get() as *mut UiMemoryInner))
            .ok()
            .flatten()
    }

    pub(crate) fn enter_scope(&self) -> UiAllocatorScope {
        let active = self.clone();
        let previous_inner =
            ACTIVE_UI_MEMORY.with(|inner| inner.replace(Rc::as_ptr(&active.inner)));
        UiAllocatorScope {
            _active: active,
            previous_inner,
        }
    }

    /// Clones a heap handle from the scope's non-owning thread-local pointer.
    ///
    /// # Safety
    ///
    /// `inner` must point to an `Rc` allocation kept alive by the active scope.
    #[inline(always)]
    unsafe fn clone_from_inner(inner: NonNull<UiMemoryInner>) -> Self {
        // SAFETY: The caller guarantees the active scope owns a strong reference.
        unsafe { Rc::increment_strong_count(inner.as_ptr()) };
        // SAFETY: The increment above provides this new Rc owner.
        let inner = unsafe { Rc::from_raw(inner.as_ptr()) };
        Self { inner }
    }

    /// Allocates storage for a Rubick block, using the per-heap size-class
    /// cache before asking dlmalloc for another block.
    #[inline(always)]
    pub(crate) fn allocate_rubick(
        &self,
        class: u8,
        layout: Layout,
    ) -> Result<NonNull<u8>, AllocError> {
        debug_assert_ne!(class, EMPTY, "Rubick blocks always include a header");
        if class != UNPOOLED {
            if let Some(block) = self.inner.rubick_cache.pop(class) {
                return Ok(block);
            }
        }

        let layout = if class == UNPOOLED {
            layout
        } else {
            pool::class_layout(class)
        };
        self.allocate_from_heap(layout)
    }

    /// Returns a Rubick block to this heap's cache, or to dlmalloc when its
    /// class cache has reached its retention limit.
    ///
    /// # Safety
    ///
    /// `block` must be a live allocation from [`UiAllocator::allocate_rubick`]
    /// with the same `class` and `layout`, and its payload must be destroyed.
    #[inline(always)]
    pub(crate) unsafe fn deallocate_rubick(&self, block: NonNull<u8>, class: u8, layout: Layout) {
        debug_assert_ne!(class, EMPTY, "Rubick blocks always include a header");
        if class != UNPOOLED && self.inner.rubick_cache.push(class, block) {
            return;
        }

        let layout = if class == UNPOOLED {
            layout
        } else {
            pool::class_layout(class)
        };
        let mut heap = self.inner.heap.borrow_mut();
        // SAFETY: pooled blocks were allocated with their class layout;
        // unpooled blocks retain their exact allocation layout.
        unsafe { heap.free(block.as_ptr(), layout.size(), layout.align()) };
    }
}

/// Restores the allocator scope that was active before a build started.
pub(crate) struct UiAllocatorScope {
    // Keeps the heap alive while the thread-local pointer refers to it.
    _active: UiAllocator,
    // The parent scope owns this heap through its `_active` handle.
    previous_inner: *const UiMemoryInner,
}

impl Drop for UiAllocatorScope {
    fn drop(&mut self) {
        let previous_inner = self.previous_inner;
        let _ = ACTIVE_UI_MEMORY.try_with(|inner| {
            inner.set(previous_inner);
        });
    }
}

// SAFETY: Cloned handles share one stable heap, and every block remains backed
// by that heap for as long as its allocator handle is retained.
unsafe impl Allocator for UiAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if layout.size() == 0 {
            // SAFETY: layout alignment is a nonzero power of two, so this
            // non-null address is aligned and no bytes are accessed.
            let dangling = unsafe { NonNull::new_unchecked(layout.align() as *mut u8) };
            return Ok(NonNull::slice_from_raw_parts(dangling, 0));
        }

        let pointer = self.allocate_from_heap(layout)?;
        Ok(NonNull::slice_from_raw_parts(pointer, layout.size()))
    }

    unsafe fn deallocate(&self, pointer: NonNull<u8>, layout: Layout) {
        if layout.size() == 0 {
            return;
        }
        let mut heap = self.inner.heap.borrow_mut();
        // SAFETY: the Allocator contract requires this pointer and layout to
        // describe a live allocation from this same heap.
        unsafe { heap.free(pointer.as_ptr(), layout.size(), layout.align()) };
    }
}

impl UiAllocator {
    #[inline(always)]
    fn allocate_from_heap(&self, layout: Layout) -> Result<NonNull<u8>, AllocError> {
        let Ok(mut heap) = self.inner.heap.try_borrow_mut() else {
            return Err(AllocError);
        };
        // SAFETY: layout is valid and the mutable borrow serializes dlmalloc
        // access to this UI-thread heap.
        NonNull::new(unsafe { heap.malloc(layout.size(), layout.align()) }).ok_or(AllocError)
    }
}

#[cfg(test)]
mod tests {
    use super::UiMemory;
    use crate::pool::class_layout;

    #[test]
    fn rubick_reuses_cached_blocks_without_borrowing_the_heap() {
        let memory = UiMemory::new(2 * 1024 * 1024);
        let allocator = memory.allocator();
        let layout = class_layout(0);
        let first = allocator.allocate_rubick(0, layout).unwrap();
        // SAFETY: `first` is a live Rubick block from this allocator, and its
        // contents are not initialized in this focused cache test.
        unsafe { allocator.deallocate_rubick(first, 0, layout) };

        // A cache hit should not enter dlmalloc at all. Holding its RefCell
        // borrow makes that property observable without timing assumptions.
        let _heap = memory.inner.heap.borrow_mut();
        let reused = allocator.allocate_rubick(0, layout).unwrap();
        assert_eq!(reused, first);
        drop(_heap);

        // SAFETY: the reused block is still live and has the same class/layout.
        unsafe { allocator.deallocate_rubick(reused, 0, layout) };
    }
}
