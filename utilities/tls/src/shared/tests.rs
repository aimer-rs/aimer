use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static INITS: AtomicUsize = AtomicUsize::new(0);

crate::thread_local! {
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
    crate::thread_local! {
        static A: Cell<u32> = const { Cell::new(10) };
        static B: Cell<u32> = Cell::new(A.get() + 1);
    }
    assert_eq!(B.get(), 11);
}

#[test]
fn with_may_nest_other_keys_while_the_table_grows() {
    crate::thread_local! {
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
    crate::thread_local! {
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
    crate::thread_local! {
        static SHARED: Rc<Cell<u32>> = Rc::new(Cell::new(3));
    }
    SHARED.with(|rc| rc.set(4));
    assert_eq!(SHARED.with(|rc| rc.get()), 4);
}

#[test]
fn teardown_keeps_live_keys_readable_and_refuses_destroyed_ones() {
    // Bits recorded by the destructors below.
    const SECOND_SAW_FIRST_ALIVE: usize = 1;
    const SECOND_SAW_UNTOUCHED_WORKS: usize = 2;
    const SECOND_SAW_ITSELF_REFUSED: usize = 4;
    const FIRST_SAW_SECOND_REFUSED: usize = 8;
    const FIRST_SAW_ITSELF_REFUSED: usize = 16;
    static SEEN: AtomicUsize = AtomicUsize::new(0);

    struct Probe(usize);
    impl Drop for Probe {
        fn drop(&mut self) {
            let mut bits = 0;
            if self.0 == 2 {
                if FIRST.try_with(|_| ()).is_ok() {
                    bits |= SECOND_SAW_FIRST_ALIVE;
                }
                if UNTOUCHED.try_with(Cell::get) == Ok(5) {
                    bits |= SECOND_SAW_UNTOUCHED_WORKS;
                }
                if SECOND.try_with(|_| ()).is_err() {
                    bits |= SECOND_SAW_ITSELF_REFUSED;
                }
            } else {
                if SECOND.try_with(|_| ()).is_err() {
                    bits |= FIRST_SAW_SECOND_REFUSED;
                }
                if FIRST.try_with(|_| ()).is_err() {
                    bits |= FIRST_SAW_ITSELF_REFUSED;
                }
            }
            SEEN.fetch_or(bits, Ordering::SeqCst);
        }
    }
    crate::thread_local! {
        static FIRST: RefCell<Option<Probe>> = const { RefCell::new(None) };
        static SECOND: RefCell<Option<Probe>> = const { RefCell::new(None) };
        static UNTOUCHED: Cell<u32> = const { Cell::new(5) };
    }

    std::thread::spawn(|| {
        // Slots are dropped newest first, so `SECOND` goes before `FIRST`.
        FIRST.set(Some(Probe(1)));
        SECOND.set(Some(Probe(2)));
    })
    .join()
    .unwrap();
    assert_eq!(SEEN.load(Ordering::SeqCst), 31);
}

#[test]
fn a_key_first_touched_during_teardown_is_still_dropped() {
    struct Late(Arc<AtomicUsize>);
    impl Drop for Late {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    struct Starter(Arc<AtomicUsize>);
    impl Drop for Starter {
        fn drop(&mut self) {
            LATE.set(Some(Late(self.0.clone())));
        }
    }
    crate::thread_local! {
        static LATE: RefCell<Option<Late>> = const { RefCell::new(None) };
        static STARTER: RefCell<Option<Starter>> = const { RefCell::new(None) };
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let for_thread = drops.clone();
    std::thread::spawn(move || STARTER.set(Some(Starter(for_thread))))
        .join()
        .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn more_statics_than_an_android_process_has_tls_keys() {
    // With std's keyed implementation this many statics would abort on
    // Android; here they all share the table's single key. Declared in
    // batches because each static in one block costs a macro recursion.
    macro_rules! batch {
        ($touch:ident; $($name:ident),*) => {
            crate::thread_local! {
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

#[test]
fn update_applies_the_closure_in_place() {
    crate::thread_local! {
        static N: Cell<u32> = const { Cell::new(2) };
    }
    N.update(|n| n * 5);
    assert_eq!(N.get(), 10);
}

#[test]
fn debug_and_access_error_match_std_output() {
    crate::thread_local! {
        static K: Cell<u32> = const { Cell::new(0) };
    }
    assert_eq!(format!("{K:?}"), "LocalKey { .. }");
    assert_eq!(format!("{:?}", super::AccessError), "AccessError");
    assert_eq!(super::AccessError.to_string(), "already destroyed");
}

#[test]
fn set_does_not_run_the_initialiser() {
    static CELL_INITS: AtomicUsize = AtomicUsize::new(0);
    static REF_CELL_INITS: AtomicUsize = AtomicUsize::new(0);
    crate::thread_local! {
        static C: Cell<u32> = {
            CELL_INITS.fetch_add(1, Ordering::SeqCst);
            Cell::new(1)
        };
        static R: RefCell<u32> = {
            REF_CELL_INITS.fetch_add(1, Ordering::SeqCst);
            RefCell::new(1)
        };
    }
    C.set(5);
    R.set(6);
    assert_eq!(C.get(), 5);
    assert_eq!(R.with_borrow(|v| *v), 6);
    // A second `set` replaces the stored value.
    C.set(7);
    R.set(8);
    assert_eq!((C.get(), R.with_borrow(|v| *v)), (7, 8));
    assert_eq!(CELL_INITS.load(Ordering::SeqCst), 0);
    assert_eq!(REF_CELL_INITS.load(Ordering::SeqCst), 0);
}

#[test]
fn zero_sized_values_work() {
    crate::thread_local! {
        static UNIT: () = ();
    }
    UNIT.with(|unit| assert_eq!(*unit, ()));
}

#[test]
fn reentrant_initialisation_keeps_the_first_value() {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    crate::thread_local! {
        static SELF_INIT: Cell<u32> = {
            // The inner access initialises the key before the outer finishes.
            if RUNS.fetch_add(1, Ordering::SeqCst) == 0 {
                SELF_INIT.set(100);
            }
            Cell::new(1)
        };
    }
    assert_eq!(SELF_INIT.get(), 100);
    assert_eq!(RUNS.load(Ordering::SeqCst), 1);
}

#[test]
fn trailing_semicolon_is_optional() {
    crate::thread_local! {
        static NO_SEMI: Cell<u32> = const { Cell::new(3) }
    }
    crate::thread_local! {
        static ONE: Cell<u32> = const { Cell::new(1) };
        #[allow(dead_code)]
        static TWO: Cell<u32> = const { Cell::new(2) }
    }
    assert_eq!(NO_SEMI.get() + ONE.get(), 4);
}

/// Helper for [`destructor_panic_aborts_the_process`]: only does something
/// when the parent test re-runs this binary with the marker variable set.
#[test]
fn abort_child() {
    if std::env::var_os("AIMER_TLS_ABORT_CHILD").is_none() {
        return;
    }
    struct Bomb;
    impl Drop for Bomb {
        fn drop(&mut self) {
            panic!("destructor panic");
        }
    }
    crate::thread_local! {
        static HOLDER: RefCell<Option<Bomb>> = const { RefCell::new(None) };
    }
    let _ = std::thread::spawn(|| HOLDER.set(Some(Bomb))).join();
    // Only reached if the panic escaped the thread's teardown.
    std::process::exit(0);
}

#[cfg(all(unix, not(any(target_os = "android", target_os = "ios"))))]
#[test]
fn destructor_panic_aborts_the_process() {
    use std::os::unix::process::ExitStatusExt;

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "shared::tests::abort_child", "--test-threads=1"])
        .env("AIMER_TLS_ABORT_CHILD", "1")
        .output()
        .unwrap()
        .status;
    // The exact signal is platform specific (SIGABRT, SIGTRAP, ...); the
    // point is that the process died instead of exiting normally.
    assert!(status.signal().is_some(), "expected an abort, got {status:?}");
}

mod scope {
    use super::*;
    use crate::scope;

    static SCOPE_INITS: AtomicUsize = AtomicUsize::new(0);

    crate::thread_local! {
        static A: Cell<u32> = const { Cell::new(1) };
        static B: Cell<u32> = const { Cell::new(2) };
        static C: Cell<u32> = const { Cell::new(3) };
        static NAMES: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        static LAZY_ONCE: Cell<u32> = {
            SCOPE_INITS.fetch_add(1, Ordering::SeqCst);
            Cell::new(40)
        };
        static UNSET: Cell<u32> = {
            panic!("`set` must not run the initialiser");
        };
    }

    #[test]
    fn reads_and_writes_several_keys_through_one_lookup() {
        let sum = scope(|t| {
            t.set(&A, 10);
            t.update(&B, |b| b + 5);
            assert_eq!(t.replace(&C, 9), 3);
            t.get(&A) + t.get(&B) + t.get(&C)
        });
        assert_eq!(sum, 10 + 7 + 9);
        // The scope sees the same storage as the keys' own methods.
        assert_eq!((A.get(), B.get(), C.get()), (10, 7, 9));
        assert_eq!(scope(|t| t.take(&C)), 9);
        assert_eq!(C.get(), 0);
    }

    #[test]
    fn local_and_borrow_helpers_work_on_ref_cells() {
        scope(|t| {
            t.with_borrow_mut(&NAMES, |names| names.push(1));
            t.with_borrow_mut(&NAMES, |names| names.push(2));
            assert_eq!(t.with_borrow(&NAMES, |names| names.clone()), vec![1, 2]);
            assert_eq!(t.local(&NAMES).borrow().len(), 2);
            assert_eq!(t.try_local(&NAMES).unwrap().borrow().len(), 2);
            assert_eq!(t.local(&NAMES).take(), vec![1, 2]);
            t.with_borrow_mut(&NAMES, |names| names.push(7));
            assert_eq!(t.local(&NAMES).replace(vec![8]), vec![7]);
        });
        assert_eq!(NAMES.with_borrow(Clone::clone), vec![8]);
    }

    #[test]
    fn lazy_initialiser_runs_once_and_set_skips_it() {
        assert_eq!(scope(|t| t.get(&LAZY_ONCE) + t.get(&LAZY_ONCE)), 80);
        assert_eq!(SCOPE_INITS.load(Ordering::SeqCst), 1);

        // `UNSET`'s initialiser panics, so `set` has to store without it.
        scope(|t| t.set(&UNSET, 5));
        assert_eq!(UNSET.get(), 5);
    }

    #[test]
    fn scopes_and_plain_keys_nest() {
        let sum = scope(|outer| {
            let inner = scope(|t| t.get(&A));
            outer.get(&B) + inner + C.get() + A.with(|a| a.get())
        });
        assert_eq!(sum, 2 + 1 + 3 + 1);
    }

    #[test]
    fn initialiser_may_use_the_scope_it_is_started_from() {
        crate::thread_local! {
            static SEED: Cell<u32> = const { Cell::new(20) };
            static DERIVED: Cell<u32> = scope(|t| Cell::new(t.get(&SEED) + 1));
        }
        assert_eq!(scope(|t| t.get(&DERIVED) + t.get(&SEED)), 41);
    }

    #[test]
    fn references_follow_the_scope_and_stay_valid_while_the_table_grows() {
        scope(|t| {
            let first = t.local(&A);
            // Touch many fresh keys so the table reallocates under `first`.
            macro_rules! touch {
                ($($name:ident),*) => {$(
                    crate::thread_local! { static $name: Cell<u32> = const { Cell::new(0) }; }
                    t.set(&$name, 1);
                )*};
            }
            touch!(K0, K1, K2, K3, K4, K5, K6, K7, K8, K9, K10, K11, K12, K13, K14, K15, K16);
            first.set(first.get() + 1);
            assert_eq!(t.get(&A), first.get());
        });
    }

    #[test]
    fn a_scope_in_a_destructor_refuses_only_destroyed_keys() {
        const LIVE_READABLE: usize = 1;
        const SELF_REFUSED: usize = 2;
        static SEEN: AtomicUsize = AtomicUsize::new(0);

        struct Probe;
        impl Drop for Probe {
            fn drop(&mut self) {
                scope(|t| {
                    let mut seen = 0;
                    if t.try_local(&LIVE_PEER).is_ok() {
                        seen |= LIVE_READABLE;
                    }
                    if t.try_local(&DYING).is_err() {
                        seen |= SELF_REFUSED;
                    }
                    SEEN.store(seen, Ordering::SeqCst);
                });
            }
        }
        crate::thread_local! {
            static LIVE_PEER: Cell<u32> = const { Cell::new(0) };
            static DYING: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }

        std::thread::spawn(|| {
            // `LIVE_PEER` is first, so it is destroyed after `DYING`.
            LIVE_PEER.set(1);
            DYING.set(Some(Probe));
        })
        .join()
        .unwrap();
        assert_eq!(SEEN.load(Ordering::SeqCst), LIVE_READABLE | SELF_REFUSED);
    }

    #[test]
    fn scopes_are_per_thread() {
        scope(|t| t.set(&A, 100));
        std::thread::spawn(|| assert_eq!(scope(|t| t.get(&A)), 1))
            .join()
            .unwrap();
        assert_eq!(scope(|t| t.get(&A)), 100);
    }
}

mod hot {
    use super::*;
    use crate::{hot, hot_supported, scope};
    use std::sync::Mutex;

    /// The fast path has one process-wide owner, so tests that look at who
    /// holds it must not overlap.
    static SERIAL: Mutex<()> = Mutex::new(());

    crate::thread_local! {
        static A: Cell<u32> = const { Cell::new(1) };
        static LAZY: Cell<u32> = Cell::new(A.get() + 10);
        static LIST: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn behaves_like_the_ordinary_path() {
        let _serial = serial();
        let guard = hot();
        assert_eq!(guard.is_active(), hot_supported());

        A.set(5);
        assert_eq!(A.get(), 5);
        assert_eq!(LAZY.get(), 15);
        LIST.with_borrow_mut(|list| list.push(1));
        assert_eq!(LIST.try_with(|l| l.borrow().len()), Ok(1));
        assert_eq!(scope(|t| t.get(&A) + t.get(&LAZY)), 20);

        // Values written while hot are the ones the ordinary path reads.
        drop(guard);
        assert_eq!(A.get(), 5);
        assert_eq!(LIST.take(), vec![1]);
    }

    #[test]
    fn only_one_thread_owns_the_fast_path_and_others_stay_correct() {
        let _serial = serial();
        let guard = hot();
        A.set(7);
        let others_guard_active = std::thread::spawn(|| {
            let other = hot();
            // A fresh thread has its own value, owner or not.
            assert_eq!(A.get(), 1);
            A.set(9);
            assert_eq!(A.get(), 9);
            other.is_active()
        })
        .join()
        .unwrap();
        assert_eq!(A.get(), 7);
        if hot_supported() {
            assert!(guard.is_active());
            assert!(!others_guard_active);
        }
    }

    #[test]
    fn nested_guards_release_only_at_the_outer_one() {
        let _serial = serial();
        let outer = hot();
        let inner = hot();
        assert!(!inner.is_active());
        drop(inner);
        // The inner guard must not have taken the fast path away.
        A.set(3);
        assert_eq!(A.get(), 3);
        if hot_supported() {
            assert!(outer.is_active());
        }
        drop(outer);
        assert_eq!(A.get(), 3);
        // Released: another thread can claim it now.
        let claimed = std::thread::spawn(|| hot().is_active()).join().unwrap();
        assert_eq!(claimed, hot_supported());
    }

    #[test]
    fn a_leaked_guard_is_released_when_its_thread_exits() {
        if !hot_supported() {
            return;
        }
        let _serial = serial();
        std::thread::spawn(|| {
            let guard = hot();
            assert!(guard.is_active());
            A.set(2);
            std::mem::forget(guard);
        })
        .join()
        .unwrap();
        let guard = hot();
        assert!(guard.is_active(), "the dead thread still owns the fast path");
        assert_eq!(A.get(), A.with(Cell::get));
    }

    #[test]
    fn destructors_run_while_hot() {
        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                // Touches another key during teardown, with the guard leaked.
                A.set(A.get() + 1);
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        crate::thread_local! {
            static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) };
        }

        let _serial = serial();
        let drops = Arc::new(AtomicUsize::new(0));
        let for_thread = drops.clone();
        std::thread::spawn(move || {
            std::mem::forget(hot());
            PROBE.set(Some(Probe(for_thread)));
        })
        .join()
        .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn each_thread_can_take_its_turn() {
        let _serial = serial();
        for round in 0..4u32 {
            std::thread::spawn(move || {
                let guard = hot();
                assert_eq!(guard.is_active(), hot_supported());
                A.set(round);
                assert_eq!(A.get(), round);
            })
            .join()
            .unwrap();
        }
    }
}
