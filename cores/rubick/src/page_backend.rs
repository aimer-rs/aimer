//! Page acquisition for the application-owned Rubick heap.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const DEFAULT_GROWTH: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct MemoryBudget {
    limit: usize,
    committed: AtomicUsize,
}

impl MemoryBudget {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            committed: AtomicUsize::new(0),
        }
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn committed(&self) -> usize {
        self.committed.load(Ordering::Relaxed)
    }

    fn reserve(&self, amount: usize) -> bool {
        let mut current = self.committed.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(amount) else {
                return false;
            };
            if next > self.limit {
                return false;
            }
            match self.committed.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    fn release(&self, amount: usize) {
        let previous = self.committed.fetch_sub(amount, Ordering::Relaxed);
        debug_assert!(
            previous >= amount,
            "released more UI memory than was committed"
        );
    }
}

struct Region {
    base: *mut u8,
    reserved_size: usize,
    #[cfg(not(target_arch = "wasm32"))]
    committed_size: usize,
}

// Region pointers are opaque mapping addresses. The mutex serializes their
// registry operations, and the backend never dereferences them from another
// thread; allocation access itself remains confined to the UI thread.
unsafe impl Send for Region {}

#[cfg(target_arch = "wasm32")]
#[repr(C)]
struct FreeRegion {
    pages: usize,
    next: *mut FreeRegion,
}

#[cfg(target_arch = "wasm32")]
aimer_tls::thread_local! {
    /// Linear-memory ranges released by a dropped UiMemory heap. WebAssembly
    /// cannot shrink memory, so this list lets later app pools reuse those
    /// ranges without asking the host for the same memory again.
    static WASM_FREE_REGIONS: std::cell::Cell<*mut FreeRegion> = const {
        std::cell::Cell::new(std::ptr::null_mut())
    };
}

pub(crate) struct PageBackend {
    budget: Arc<MemoryBudget>,
    page_size: usize,
    regions: Mutex<Vec<Region>>,
}

impl PageBackend {
    pub(crate) fn new(limit: usize) -> Self {
        let page_size = os::page_size();
        Self {
            budget: Arc::new(MemoryBudget::new(limit)),
            page_size,
            regions: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn budget(&self) -> Arc<MemoryBudget> {
        Arc::clone(&self.budget)
    }

    pub(crate) fn page_size(&self) -> usize {
        self.page_size
    }

    fn rounded_size(&self, size: usize) -> Option<usize> {
        size.checked_add(self.page_size - 1)
            .map(|size| size / self.page_size * self.page_size)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn allocate_native(&self, size: usize) -> (*mut u8, usize, u32) {
        let Ok(mut regions) = self.regions.lock() else {
            return (std::ptr::null_mut(), 0, 0);
        };
        if regions.try_reserve(1).is_err() || !self.budget.reserve(size) {
            return (std::ptr::null_mut(), 0, 0);
        }

        // SAFETY: `size` is nonzero and page rounded; the OS returns a fresh
        // read/write region or a null pointer.
        let base = unsafe { os::allocate(size) };
        if base.is_null() {
            self.budget.release(size);
            return (base, 0, 0);
        }

        regions.push(Region {
            base,
            reserved_size: size,
            #[cfg(not(target_arch = "wasm32"))]
            committed_size: size,
        });
        (base, size, 0)
    }

    #[cfg(target_arch = "wasm32")]
    fn allocate_wasm(&self, size: usize) -> (*mut u8, usize, u32) {
        let pages = size / self.page_size;
        let Some(grown_size) = pages.checked_mul(self.page_size) else {
            return (std::ptr::null_mut(), 0, 0);
        };
        if !self.budget.reserve(grown_size) {
            return (std::ptr::null_mut(), 0, 0);
        }

        let Ok(mut regions) = self.regions.lock() else {
            self.budget.release(grown_size);
            return (std::ptr::null_mut(), 0, 0);
        };
        if regions.try_reserve(1).is_err() {
            self.budget.release(grown_size);
            return (std::ptr::null_mut(), 0, 0);
        }

        if let Some(base) = take_wasm_region(pages, self.page_size) {
            regions.push(Region {
                base,
                reserved_size: grown_size,
            });
            return (base, grown_size, 0);
        }

        // `memory.grow` gives this allocator the new, disjoint tail of linear
        // memory. WebAssembly pointers remain linear-memory offsets if the
        // host moves the backing bytes while growing memory.
        let previous_pages = core::arch::wasm32::memory_grow::<0>(pages);
        if previous_pages == usize::MAX {
            self.budget.release(grown_size);
            return (std::ptr::null_mut(), 0, 0);
        }

        let Some(previous_bytes) = previous_pages.checked_mul(self.page_size) else {
            self.budget.release(grown_size);
            return (std::ptr::null_mut(), 0, 0);
        };
        let base = previous_bytes as *mut u8;
        regions.push(Region {
            base,
            reserved_size: grown_size,
        });
        let mut available = grown_size;
        if previous_bytes.wrapping_add(grown_size) == 0 {
            available = available.saturating_sub(16);
        }
        (base, available, 0)
    }
}

// SAFETY: Every returned segment is a page-aligned OS mapping or a unique
// linear-memory tail. The backend tracks native mappings until they are
// released or the allocator is dropped. Wasm segments are never returned to
// the host because linear memory cannot shrink.
unsafe impl dlmalloc::Allocator for PageBackend {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let Some(size) = self.rounded_size(size) else {
            return (std::ptr::null_mut(), 0, 0);
        };
        if size == 0 {
            return (std::ptr::null_mut(), 0, 0);
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.allocate_wasm(size)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.allocate_native(size)
        }
    }

    fn remap(&self, _ptr: *mut u8, _oldsize: usize, _newsize: usize, _can_move: bool) -> *mut u8 {
        std::ptr::null_mut()
    }

    fn free_part(&self, ptr: *mut u8, oldsize: usize, newsize: usize) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (ptr, oldsize, newsize);
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if newsize > oldsize {
                return false;
            }
            if newsize == oldsize {
                return true;
            }
            let Ok(mut regions) = self.regions.lock() else {
                return false;
            };
            let Some(region) = regions.iter_mut().find(|region| region.base == ptr) else {
                return false;
            };
            if region.committed_size != oldsize {
                return false;
            }

            // SAFETY: dlmalloc supplies the old segment base and page-aligned
            // sizes for the tail it has stopped using.
            if !unsafe { os::release_part(ptr, oldsize, newsize) } {
                return false;
            }
            let released = oldsize - newsize;
            region.committed_size = newsize;
            #[cfg(unix)]
            {
                if newsize == 0 {
                    // The complete mmap is gone. Clear its identity so a
                    // later mapping at the same address matches the new
                    // region record instead of this released tombstone.
                    region.base = std::ptr::null_mut();
                    region.reserved_size = 0;
                } else {
                    region.reserved_size = newsize;
                }
            }
            self.budget.release(released);
            true
        }
    }

    fn free(&self, ptr: *mut u8, _size: usize) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ptr;
            false
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let Ok(mut regions) = self.regions.lock() else {
                return false;
            };
            let Some(region) = regions.iter_mut().find(|region| region.base == ptr) else {
                return false;
            };
            // SAFETY: this is the original base of a region returned by
            // `allocate_native`; Windows releases the complete reservation.
            if !unsafe { os::release(region.base, region.reserved_size) } {
                return false;
            }
            self.budget.release(region.committed_size);
            region.base = std::ptr::null_mut();
            region.reserved_size = 0;
            #[cfg(not(target_arch = "wasm32"))]
            {
                region.committed_size = 0;
            }
            true
        }
    }

    fn can_release_part(&self, _flags: u32) -> bool {
        cfg!(not(target_arch = "wasm32"))
    }

    fn allocates_zeros(&self) -> bool {
        // Native regions are freshly mapped and zero-filled. Wasm regions can
        // come from this backend's free list, so they may contain old bytes.
        cfg!(not(target_arch = "wasm32"))
    }

    fn page_size(&self) -> usize {
        self.page_size
    }
}

impl Drop for PageBackend {
    fn drop(&mut self) {
        let regions = self
            .regions
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for region in regions {
            if !region.base.is_null() {
                #[cfg(target_arch = "wasm32")]
                return_wasm_region(region.base, region.reserved_size, self.page_size);
                #[cfg(not(target_arch = "wasm32"))]
                {
                    // SAFETY: every non-null entry is a still-live mapping
                    // returned by `allocate_native` and has not been released.
                    let _ = unsafe { os::release(region.base, region.reserved_size) };
                }
                region.base = std::ptr::null_mut();
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{PageBackend, os};
    use dlmalloc::Allocator as DlmallocAllocator;

    #[test]
    fn releasing_an_entire_unix_segment_clears_its_mapping_record() {
        let page_size = os::page_size();
        let backend = PageBackend::new(page_size * 2);
        let (base, size, _) = DlmallocAllocator::alloc(&backend, page_size);
        assert!(!base.is_null());
        assert_eq!(size, page_size);
        assert_eq!(backend.budget.committed(), page_size);

        assert!(DlmallocAllocator::free_part(&backend, base, size, 0));
        assert_eq!(backend.budget.committed(), 0);

        let regions = backend.regions.lock().unwrap();
        assert!(regions.iter().all(|region| region.base.is_null()));
    }
}

#[cfg(target_arch = "wasm32")]
fn take_wasm_region(pages: usize, page_size: usize) -> Option<*mut u8> {
    let requested_size = pages.checked_mul(page_size)?;
    WASM_FREE_REGIONS
        .try_with(|free_regions| {
            let mut previous = std::ptr::null_mut::<FreeRegion>();
            let mut current = free_regions.get();
            while !current.is_null() {
                // SAFETY: the free list contains only nodes written into live,
                // page-aligned linear-memory regions by `return_wasm_region`.
                let node = unsafe { &*current };
                let available_pages = node.pages;
                let next = node.next;
                if available_pages >= pages {
                    let base = current.cast::<u8>();
                    let remaining_pages = available_pages - pages;
                    let replacement = if remaining_pages == 0 {
                        next
                    } else {
                        // SAFETY: the remaining range starts after the allocated
                        // prefix and still has at least one whole page.
                        let remainder = unsafe { base.add(requested_size).cast::<FreeRegion>() };
                        unsafe {
                            remainder.write(FreeRegion {
                                pages: remaining_pages,
                                next,
                            });
                        }
                        remainder
                    };
                    if previous.is_null() {
                        free_regions.set(replacement);
                    } else {
                        // SAFETY: `previous` remains a live free-list node.
                        unsafe { (*previous).next = replacement };
                    }
                    return Some(base);
                }
                previous = current;
                current = next;
            }
            None
        })
        .ok()
        .flatten()
}

#[cfg(target_arch = "wasm32")]
fn return_wasm_region(base: *mut u8, size: usize, page_size: usize) {
    if base.is_null() || size < std::mem::size_of::<FreeRegion>() {
        return;
    }
    let pages = size / page_size;
    let _ = WASM_FREE_REGIONS.try_with(|free_regions| {
        let mut previous = std::ptr::null_mut::<FreeRegion>();
        let mut current = free_regions.get();
        while !current.is_null() && (current as usize) < base as usize {
            previous = current;
            // SAFETY: each current node points into a live free linear-memory
            // range previously returned by this function.
            current = unsafe { (*current).next };
        }

        let node = base.cast::<FreeRegion>();
        // SAFETY: the region is no longer owned by a heap and has enough
        // committed bytes for this intrusive free-list header.
        unsafe {
            node.write(FreeRegion {
                pages,
                next: current,
            })
        };
        if previous.is_null() {
            free_regions.set(node);
        } else {
            // SAFETY: `previous` is the still-live list node before `node`.
            unsafe { (*previous).next = node };
        }

        let node_end = (node as usize).checked_add(pages.saturating_mul(page_size));
        if !current.is_null() && node_end == Some(current as usize) {
            // SAFETY: adjacent free regions belong to the same linear memory;
            // joining their page counts makes the next allocation contiguous.
            unsafe {
                (*node).pages = (*node).pages.saturating_add((*current).pages);
                (*node).next = (*current).next;
            }
        }

        if !previous.is_null() {
            let previous_end = unsafe {
                (previous as usize).checked_add((*previous).pages.saturating_mul(page_size))
            };
            if previous_end == Some(node as usize) {
                // SAFETY: the preceding region ends exactly where this one
                // starts, so merge them into the preceding list node.
                unsafe {
                    (*previous).pages = (*previous).pages.saturating_add((*node).pages);
                    (*previous).next = (*node).next;
                }
            }
        }
    });
}

#[cfg(unix)]
mod os {
    pub(super) fn page_size() -> usize {
        // SAFETY: sysconf is a read-only query with no pointer arguments.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size > 0 && (page_size as usize).is_power_of_two() {
            page_size as usize
        } else {
            4096
        }
    }

    pub(super) unsafe fn allocate(size: usize) -> *mut u8 {
        // SAFETY: the caller requests a nonzero page-rounded anonymous region.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            std::ptr::null_mut()
        } else {
            ptr.cast()
        }
    }

    pub(super) unsafe fn release(ptr: *mut u8, size: usize) -> bool {
        // SAFETY: the pointer and size identify a region returned by mmap.
        unsafe { libc::munmap(ptr.cast(), size) == 0 }
    }

    pub(super) unsafe fn release_part(ptr: *mut u8, oldsize: usize, newsize: usize) -> bool {
        // SAFETY: the caller has validated the mapped segment and page-rounded
        // sizes; release only the unused tail.
        unsafe { libc::munmap(ptr.add(newsize).cast(), oldsize - newsize) == 0 }
    }
}

#[cfg(target_os = "windows")]
mod os {
    use std::mem::MaybeUninit;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_DECOMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc,
        VirtualFree,
    };
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

    pub(super) fn page_size() -> usize {
        let mut info = MaybeUninit::<SYSTEM_INFO>::uninit();
        // SAFETY: GetSystemInfo initializes the provided SYSTEM_INFO value.
        let page_size = unsafe {
            GetSystemInfo(info.as_mut_ptr());
            info.assume_init().dwPageSize as usize
        };
        if page_size != 0 && page_size.is_power_of_two() {
            page_size
        } else {
            4096
        }
    }

    pub(super) unsafe fn allocate(size: usize) -> *mut u8 {
        // SAFETY: VirtualAlloc reserves and commits a fresh read/write region.
        unsafe {
            VirtualAlloc(
                std::ptr::null_mut(),
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
            .cast()
        }
    }

    pub(super) unsafe fn release(ptr: *mut u8, _size: usize) -> bool {
        // SAFETY: MEM_RELEASE requires the original VirtualAlloc base and zero
        // size, both tracked by PageBackend.
        unsafe { VirtualFree(ptr.cast(), 0, MEM_RELEASE) != 0 }
    }

    pub(super) unsafe fn release_part(ptr: *mut u8, oldsize: usize, newsize: usize) -> bool {
        // SAFETY: VirtualAlloc page boundaries and dlmalloc segment sizes are
        // page aligned; decommit only the unused tail.
        unsafe { VirtualFree(ptr.add(newsize).cast(), oldsize - newsize, MEM_DECOMMIT) != 0 }
    }
}

#[cfg(target_arch = "wasm32")]
mod os {
    pub(super) fn page_size() -> usize {
        64 * 1024
    }
}

#[cfg(not(any(unix, target_os = "windows", target_arch = "wasm32")))]
compile_error!("UiMemory page allocation is supported on Unix, Windows, and wasm32 targets");

pub(crate) fn growth_granularity(limit: usize, page_size: usize) -> usize {
    let cap = limit.min(DEFAULT_GROWTH);
    if cap == 0 {
        return DEFAULT_GROWTH;
    }
    let highest_power = 1usize << (usize::BITS - cap.leading_zeros() - 1);
    highest_power
        .max(page_size)
        .max(2 * std::mem::size_of::<usize>())
}
