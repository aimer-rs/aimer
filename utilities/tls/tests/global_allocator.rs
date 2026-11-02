//! A `#[global_allocator]` may use `aimer_tls` for its own bookkeeping.
//!
//! The table and the values are allocated with `System`, so the first access to
//! a key from inside `alloc` must not re-enter the allocator. This is a
//! separate integration test because the allocator is process-wide.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

aimer_tls::thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every method forwards to `System` unchanged; the counter only
// observes the call.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn allocator_can_count_through_aimer_tls() {
    let before = ALLOCATIONS.get();
    let held = Box::new([1u8, 2, 3]);
    assert!(ALLOCATIONS.get() > before);
    assert_eq!(held.len(), 3);

    // Other threads start with their own, fresh counter and table.
    std::thread::spawn(|| {
        let _ = Box::new(0u64);
        assert!(ALLOCATIONS.get() > 0);
    })
    .join()
    .unwrap();
}
