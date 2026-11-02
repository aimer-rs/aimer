//! The first thread that creates a thread-local owns the fast path without
//! calling `hot()`; every other thread stays correct. This is an integration
//! test because the crate's own unit tests disable automatic ownership.

use std::cell::{Cell, RefCell};

aimer_tls::thread_local! {
    static COUNT: Cell<u32> = const { Cell::new(1) };
    static LIST: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn first_thread_owns_the_fast_path_and_others_stay_correct() {
    // The first use creates this thread's table and claims the fast path.
    COUNT.set(5);
    LIST.with_borrow_mut(|list| list.push(1));
    assert_eq!(COUNT.get(), 5);
    assert_eq!(COUNT.try_with(Cell::get), Ok(5));
    assert_eq!(aimer_tls::scope(|t| t.get(&COUNT)), 5);

    if aimer_tls::hot_supported() {
        // The owner cannot be claimed twice.
        assert!(!aimer_tls::hot().is_active());
    }

    // Another thread must not see the owner's cached value.
    std::thread::spawn(|| {
        assert_eq!(COUNT.get(), 1);
        COUNT.set(9);
        assert_eq!(COUNT.get(), 9);
        LIST.with_borrow(|list| assert!(list.is_empty()));
    })
    .join()
    .unwrap();

    assert_eq!(COUNT.get(), 5);
    assert_eq!(LIST.take(), vec![1]);
}

#[test]
fn a_thread_that_exits_releases_the_fast_path_for_the_next_one() {
    // Whichever thread of this process owns it, a fresh thread that exits
    // must leave every later thread with correct values.
    for round in 0..8u32 {
        std::thread::spawn(move || {
            assert_eq!(COUNT.get(), 1);
            COUNT.set(round);
            assert_eq!(COUNT.get(), round);
        })
        .join()
        .unwrap();
    }
}
