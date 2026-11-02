//! Compares `aimer_tls`'s shared table with `std::thread_local!`.
//!
//! criterion is not a workspace dependency, so this is a plain `harness =
//! false` bench: each case is timed over several batches and the median is
//! reported. Run it without extra flags:
//!
//! ```text
//! cargo bench -p aimer_tls --profile dev
//! ```

// The std side of the comparison is the whole point of this file.
#![allow(clippy::disallowed_macros)]

use std::cell::{Cell, RefCell};
use std::hint::black_box;
use aimer_time::Instant;

const BATCH: u32 = 200_000;
const ROUNDS: usize = 15;

std::thread_local! {
    static STD_CELL: Cell<u64> = const { Cell::new(0) };
    static STD_LAZY: Cell<u64> = Cell::new(std::env::args().count() as u64);
    static STD_LIST: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static STD_A: Cell<u64> = const { Cell::new(1) };
    static STD_B: Cell<u64> = const { Cell::new(2) };
    static STD_C: Cell<u64> = const { Cell::new(3) };
    static STD_D: Cell<u64> = const { Cell::new(4) };
}

aimer_tls::thread_local! {
    static TLS_CELL: Cell<u64> = const { Cell::new(0) };
    static TLS_LAZY: Cell<u64> = Cell::new(std::env::args().count() as u64);
    static TLS_LIST: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    static TLS_A: Cell<u64> = const { Cell::new(1) };
    static TLS_B: Cell<u64> = const { Cell::new(2) };
    static TLS_C: Cell<u64> = const { Cell::new(3) };
    static TLS_D: Cell<u64> = const { Cell::new(4) };
}

/// Median nanoseconds per call of `f` over `ROUNDS` batches.
fn measure(mut f: impl FnMut()) -> f64 {
    for _ in 0..BATCH {
        f();
    }
    let mut samples = [0.0f64; ROUNDS];
    for sample in &mut samples {
        let start = Instant::now();
        for _ in 0..BATCH {
            f();
        }
        *sample = start.elapsed().as_nanos() as f64 / f64::from(BATCH);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[ROUNDS / 2]
}

fn report(name: &str, std_ns: f64, tls_ns: f64) {
    println!(
        "{name:<28} std {std_ns:>8.2} ns   aimer_tls {tls_ns:>8.2} ns   ratio {:>5.2}x",
        tls_ns / std_ns
    );
}

fn main() {
    println!("-- ordinary path");
    run();

    // The hot guard lets this thread skip std's TLS lookup for aimer_tls only;
    // the std side is unchanged.
    let hot = aimer_tls::hot();
    println!(
        "-- hot thread (active: {}, supported: {})",
        hot.is_active(),
        aimer_tls::hot_supported()
    );
    run();
}

fn run() {
    let std_ns = measure(|| STD_CELL.set(black_box(STD_CELL.get()) + 1));
    let tls_ns = measure(|| TLS_CELL.set(black_box(TLS_CELL.get()) + 1));
    report("cell get+set (const)", std_ns, tls_ns);

    let std_ns = measure(|| {
        black_box(STD_LAZY.get());
    });
    let tls_ns = measure(|| {
        black_box(TLS_LAZY.get());
    });
    report("cell get (lazy init)", std_ns, tls_ns);

    let std_ns = measure(|| {
        STD_LIST.with_borrow_mut(|list| {
            list.push(black_box(1));
            list.pop();
        })
    });
    let tls_ns = measure(|| {
        TLS_LIST.with_borrow_mut(|list| {
            list.push(black_box(1));
            list.pop();
        })
    });
    report("refcell push+pop", std_ns, tls_ns);

    let std_ns = measure(|| {
        black_box(STD_A.get() + STD_B.get() + STD_C.get() + STD_D.get());
    });
    let tls_ns = measure(|| {
        black_box(TLS_A.get() + TLS_B.get() + TLS_C.get() + TLS_D.get());
    });
    report("four keys get", std_ns, tls_ns);

    let std_ns = measure(|| {
        black_box(STD_A.with(|a| STD_B.with(|b| a.get() + b.get())));
    });
    let tls_ns = measure(|| {
        black_box(TLS_A.with(|a| TLS_B.with(|b| a.get() + b.get())));
    });
    report("nested with", std_ns, tls_ns);

    let std_ns = measure(|| {
        black_box(STD_CELL.try_with(|c| c.get()).ok());
    });
    let tls_ns = measure(|| {
        black_box(TLS_CELL.try_with(|c| c.get()).ok());
    });
    report("try_with", std_ns, tls_ns);

    // The scope resolves the thread's table once for all four keys, which is
    // what std cannot do: it pays one TLS lookup per key.
    let std_ns = measure(|| {
        black_box(STD_A.get() + STD_B.get() + STD_C.get() + STD_D.get());
    });
    let tls_ns = measure(|| {
        black_box(aimer_tls::scope(|t| {
            t.get(&TLS_A) + t.get(&TLS_B) + t.get(&TLS_C) + t.get(&TLS_D)
        }));
    });
    report("four keys via scope", std_ns, tls_ns);

    let std_ns = measure(|| {
        STD_CELL.set(black_box(STD_CELL.get()) + STD_A.get());
    });
    let tls_ns = measure(|| {
        aimer_tls::scope(|t| t.set(&TLS_CELL, black_box(t.get(&TLS_CELL)) + t.get(&TLS_A)));
    });
    report("two keys read+write scope", std_ns, tls_ns);
}
