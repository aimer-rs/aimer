//! Heap-mode payload blocks for both app-owned and thread-local allocators.
//!
//! A retained widget or element tree is rebuilt many times per second and
//! most of its heap payloads share a handful of layouts. Routing those blocks
//! through the global allocator pays for a lock-free-but-not-free `malloc`
//! search on every node, every frame. Outside a UI scope, this module recycles
//! blocks in per-size-class free lists. During an app build, it uses the app's
//! allocator and records the owning handle in each block header.
//!
//! The pool is sound because [`Rubick`](crate::Rubick) is neither `Send` nor
//! `Sync`, so a payload is always freed on the thread that allocated it. The
//! free list is intrusive: a cached block stores the next pointer in its own
//! first word, so the pool itself needs no auxiliary allocation.

use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::cell::Cell;
use std::ptr::NonNull;

use crate::UiAllocator;

/// Alignment guaranteed by every pooled block.
///
/// Requests with a stricter alignment bypass the pool and use their exact
/// layout, which keeps recycling correct without over-aligning every class.
const CLASS_ALIGNMENT: usize = 16;

/// Metadata at the start of each Rubick heap block. The header is aligned to
/// the largest pooled alignment, making the payload offset stable for pooled
/// layouts while keeping origin information out of every Rubick owner.
#[repr(C, align(16))]
struct HeapHeader {
    origin: HeapOrigin,
}

enum HeapOrigin {
    ThreadLocal,
    Ui(UiAllocator),
}

/// Returns the block layout and payload offset needed to store `layout` after
/// a provenance header. `None` means the combined allocation would overflow.
pub(crate) const fn heap_layout(layout: Layout) -> Option<(Layout, usize)> {
    match Layout::new::<HeapHeader>().extend(layout) {
        Ok((combined, offset)) => Some((combined.pad_to_align(), offset)),
        Err(_) => None,
    }
}

/// Block sizes served by the pool, in ascending order.
///
/// The smallest class is at least two words so an intrusive next pointer
/// always fits, and the largest bounds how much memory one thread can retain.
pub(crate) const CLASS_SIZES: [usize; 6] = [16, 32, 64, 128, 256, 512];

/// Bytes a single class may keep cached before blocks are returned to the
/// global allocator.
///
/// A cached block is one that the program has already freed, so the budget
/// bounds the gap between peak and current usage rather than total usage. It
/// has to cover a whole tree rebuild to be useful: a frame that destroys every
/// node and immediately recreates it must find the freed blocks still in the
/// list. Half a megabyte per class covers several thousand nodes while keeping
/// the worst case for a thread that touches every class in the low megabytes.
const CLASS_BUDGET: usize = 512 * 1024;

/// Returns how many blocks of `size` bytes the pool may retain.
pub(crate) const fn class_capacity(size: usize) -> usize {
    let blocks = CLASS_BUDGET / size;
    if blocks < 8 { 8 } else { blocks }
}

/// The class of a layout the pool cannot serve, which is freed with its exact
/// layout instead.
pub(crate) const UNPOOLED: u8 = u8::MAX;

/// The class of a zero-sized payload, which needs no memory at all.
pub(crate) const EMPTY: u8 = u8::MAX - 1;

/// Returns the class that will serve `layout` for the whole program run.
///
/// Callers evaluate this once per concrete type, in a constant, and store the
/// result in that type's operation table. Allocation and deallocation then
/// never search the class table, which matters most in unoptimized builds
/// where such a search is a real loop.
#[inline]
pub(crate) const fn class_of(layout: Layout) -> u8 {
    if layout.size() == 0 {
        return EMPTY;
    }
    if layout.align() > CLASS_ALIGNMENT {
        return UNPOOLED;
    }
    let mut index = 0;
    while index < CLASS_SIZES.len() {
        if layout.size() <= CLASS_SIZES[index] {
            return index as u8;
        }
        index += 1;
    }
    UNPOOLED
}

/// Returns the layout every block of `class` is allocated and freed with.
#[inline(always)]
pub(crate) const fn class_layout(class: u8) -> Layout {
    // SAFETY: Every class size is a non-zero multiple of `CLASS_ALIGNMENT`,
    // which is a power of two, so the pair is a valid layout.
    unsafe { Layout::from_size_align_unchecked(CLASS_SIZES[class as usize], CLASS_ALIGNMENT) }
}

/// Returns `true` when a block allocated for `current` can be reused for a
/// payload of `candidate` without touching any allocator.
///
/// Reuse requires both classes to be the same pooled class, because a block
/// must be freed with the layout it was allocated with. Unpooled payloads
/// additionally need identical layouts.
#[inline]
pub(crate) const fn reuses(current: u8, candidate: u8, exact: bool) -> bool {
    if current != candidate {
        return false;
    }
    current != EMPTY && (current != UNPOOLED || exact)
}

struct Pool {
    heads: [Cell<*mut u8>; CLASS_SIZES.len()],
    counts: [Cell<usize>; CLASS_SIZES.len()],
}

impl Pool {
    const fn new() -> Self {
        Self {
            heads: [const { Cell::new(std::ptr::null_mut()) }; CLASS_SIZES.len()],
            counts: [const { Cell::new(0) }; CLASS_SIZES.len()],
        }
    }

    /// Removes and returns a cached block of `index`, or null when the class
    /// is empty.
    #[inline(always)]
    fn pop(&self, index: usize) -> *mut u8 {
        let head = self.heads[index].get();
        if head.is_null() {
            return head;
        }
        // SAFETY: A cached block is at least two words long and stores the
        // next pointer in its first word, written by `push`.
        let next = unsafe { head.cast::<*mut u8>().read() };
        self.heads[index].set(next);
        self.counts[index].set(self.counts[index].get() - 1);
        head
    }

    /// Caches `block` for `index` and reports whether it was accepted.
    #[inline(always)]
    fn push(&self, index: usize, block: NonNull<u8>) -> bool {
        let count = self.counts[index].get();
        if count >= class_capacity(CLASS_SIZES[index]) {
            return false;
        }
        // SAFETY: The block belongs to this class, so it is at least two words
        // long, and its payload is already dropped, so overwriting the first
        // word destroys nothing.
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

impl Drop for Pool {
    fn drop(&mut self) {
        for index in 0..CLASS_SIZES.len() {
            let mut current = self.heads[index].get();
            while let Some(block) = NonNull::new(current) {
                // SAFETY: Cached blocks form an intrusive list and were all
                // allocated with this class layout.
                unsafe {
                    current = block.as_ptr().cast::<*mut u8>().read();
                    dealloc(block.as_ptr(), class_layout(index as u8));
                }
            }
            self.heads[index].set(std::ptr::null_mut());
            self.counts[index].set(0);
        }
    }
}

thread_local! {
    static POOL: Pool = const { Pool::new() };
}

/// Returns a non-null, correctly aligned address for a zero-sized payload.
#[inline]
const fn dangling(align: usize) -> NonNull<u8> {
    // SAFETY: An alignment is always a non-zero power of two.
    unsafe { NonNull::new_unchecked(align as *mut u8) }
}

#[inline]
fn global(layout: Layout) -> NonNull<u8> {
    // SAFETY: The layout has a non-zero size, checked by every caller.
    let pointer = unsafe { alloc(layout) };
    match NonNull::new(pointer) {
        Some(pointer) => pointer,
        None => handle_alloc_error(layout),
    }
}

/// Allocates storage for one value of `class` and `layout`.
///
/// `class` must be [`class_of`] applied to `layout`; the caller keeps it in a
/// constant so that neither this function nor its counterpart has to derive
/// it. The block is recycled from this thread's free list whenever the class
/// is pooled, so a steady-state tree rebuild performs no global allocation.
#[inline(always)]
pub(crate) fn allocate(class: u8, layout: Layout) -> NonNull<u8> {
    if class == EMPTY {
        return dangling(layout.align());
    }
    if class == UNPOOLED {
        return global(layout);
    }
    let recycled = POOL
        .try_with(|pool| pool.pop(class as usize))
        .unwrap_or(std::ptr::null_mut());
    match NonNull::new(recycled) {
        Some(block) => block,
        None => global(class_layout(class)),
    }
}

/// Releases storage previously produced by [`allocate`] for the same class
/// and layout.
///
/// # Safety
///
/// `pointer` must come from [`allocate`] with an identical `class` and
/// `layout`, its payload must already be dropped, and it must not be used
/// afterwards.
#[inline(always)]
pub(crate) unsafe fn deallocate(pointer: NonNull<u8>, class: u8, layout: Layout) {
    if class == EMPTY {
        return;
    }
    if class == UNPOOLED {
        // SAFETY: Unpooled blocks are allocated with their exact layout.
        unsafe { dealloc(pointer.as_ptr(), layout) };
        return;
    }
    let cached = POOL
        .try_with(|pool| pool.push(class as usize, pointer))
        .unwrap_or(false);
    if !cached {
        // SAFETY: Pooled blocks are always allocated with their class layout,
        // so they must be freed with it.
        unsafe { dealloc(pointer.as_ptr(), class_layout(class)) };
    }
}

/// Allocates one Rubick block and records the allocator that owns it.
#[inline(always)]
pub(crate) fn allocate_rubick(class: u8, layout: Layout) -> NonNull<u8> {
    let (block, origin) = if let Some(allocator) = UiAllocator::current() {
        let block = match allocator.allocate_rubick(class, layout) {
            Ok(block) => block,
            Err(_) => handle_alloc_error(layout),
        };
        (block, HeapOrigin::Ui(allocator))
    } else {
        (allocate(class, layout), HeapOrigin::ThreadLocal)
    };

    // SAFETY: the block layout includes a correctly aligned header at its
    // start, and the header is initialized before the payload is written.
    unsafe {
        block
            .cast::<HeapHeader>()
            .as_ptr()
            .write(HeapHeader { origin })
    };
    block
}

/// Returns the payload address following a Rubick block's header.
#[inline(always)]
pub(crate) fn heap_data_pointer(block: *mut u8, offset: usize) -> *mut u8 {
    // SAFETY: `offset` comes from `heap_layout` for the same concrete payload
    // and lies within the allocated combined layout.
    unsafe { block.add(offset) }
}

/// Releases a Rubick block through the allocator recorded in its header.
///
/// # Safety
///
/// `block` must be a live result of [`allocate_rubick`] for `class` and
/// `layout`, and its payload must already have been destroyed or moved out.
#[inline(always)]
pub(crate) unsafe fn deallocate_rubick(block: NonNull<u8>, class: u8, layout: Layout) {
    // SAFETY: the caller guarantees this is a live Rubick block with an
    // initialized header.
    let header = unsafe { block.cast::<HeapHeader>().as_ptr().read() };
    match header.origin {
        HeapOrigin::ThreadLocal => {
            // SAFETY: the block was originally allocated from this thread pool
            // with the same class and combined layout.
            unsafe { deallocate(block, class, layout) };
        }
        HeapOrigin::Ui(allocator) => {
            // SAFETY: the header retains the allocator that produced the
            // allocation, including after the build scope or app has ended.
            unsafe { allocator.deallocate_rubick(block, class, layout) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_are_ascending_and_hold_an_intrusive_pointer() {
        assert!(CLASS_SIZES[0] >= 2 * size_of::<*mut u8>());
        for pair in CLASS_SIZES.windows(2) {
            assert!(pair[0] < pair[1]);
            assert_eq!(pair[0] % CLASS_ALIGNMENT, 0);
        }
    }

    #[test]
    fn class_selection_follows_size_and_alignment() {
        let layout = |size, align| Layout::from_size_align(size, align).unwrap();

        assert_eq!(class_of(layout(1, 1)), 0);
        assert_eq!(class_of(layout(16, 8)), 0);
        assert_eq!(class_of(layout(17, 8)), 1);
        assert_eq!(class_of(layout(512, 16)), 5);
        assert_eq!(class_of(layout(513, 8)), UNPOOLED);
        assert_eq!(class_of(layout(8, 32)), UNPOOLED);
        assert_eq!(class_of(layout(0, 8)), EMPTY);
    }

    #[test]
    fn reuse_requires_a_shared_class_or_an_identical_layout() {
        let layout = |size, align| Layout::from_size_align(size, align).unwrap();
        let class = |size, align| class_of(layout(size, align));

        assert!(reuses(class(40, 8), class(64, 8), false));
        assert!(!reuses(class(40, 8), class(65, 8), false));
        assert!(reuses(class(1024, 8), class(1024, 8), true));
        assert!(
            !reuses(class(1024, 8), class(2048, 8), false),
            "unpooled blocks need identical layouts"
        );
        assert!(!reuses(EMPTY, EMPTY, true), "nothing to reuse");
    }

    #[test]
    fn recycled_blocks_are_reused_and_stay_usable() {
        let layout = Layout::from_size_align(48, 8).unwrap();
        let class = class_of(layout);
        let first = allocate(class, layout);
        // SAFETY: The block is at least 48 bytes and currently unused.
        unsafe { first.as_ptr().write_bytes(0xAB, 48) };
        // SAFETY: The block came from `allocate` with this class.
        unsafe { deallocate(first, class, layout) };

        let second = allocate(class, layout);
        assert_eq!(first, second);
        // SAFETY: The block is live and at least 48 bytes.
        unsafe { second.as_ptr().write_bytes(0xCD, 48) };
        // SAFETY: The block came from `allocate` with this class.
        unsafe { deallocate(second, class, layout) };
    }

    #[test]
    fn zero_sized_requests_never_touch_the_allocator() {
        let layout = Layout::from_size_align(0, 32).unwrap();
        let class = class_of(layout);
        let pointer = allocate(class, layout);

        assert_eq!(class, EMPTY);
        assert_eq!(pointer.as_ptr() as usize % 32, 0);
        // SAFETY: A zero-sized deallocation is a no-op.
        unsafe { deallocate(pointer, class, layout) };
    }
}
