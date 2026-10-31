//! Allocation and dispatch benchmark for [`aimer_rubick::Rubick`].
//!
//! Run it in both profiles and compare:
//!
//! ```text
//! cargo run -p aimer_rubick --example rubick_benchmark
//! cargo run -p aimer_rubick --example rubick_benchmark --release
//! ```
//!
//! Every scenario is repeated in several rounds and the best round is
//! reported, which keeps the numbers stable under a noisy scheduler. The
//! reported unit is nanoseconds per operation.

use std::hint::black_box;
use aimer_time::Instant;

use aimer_rubick::{ErasedFrom, INLINE_CAPACITY, Rubick, UiMemory};

const ROUNDS: usize = 5;

/// The capacity an erased widget owner uses in the framework.
type Wide = Rubick<dyn Node, 8>;

trait Node {
    fn value(&self) -> usize;
    fn bump(&mut self);
}

// SAFETY: The template is `null::<N>()` coerced to the target.
unsafe impl<N: Node + 'static> ErasedFrom<N> for dyn Node {
    const TEMPLATE: *const Self = std::ptr::null::<N>() as *const dyn Node;
}

macro_rules! node {
    ($name:ident, $bytes:expr) => {
        struct $name {
            counter: usize,
            payload: [u8; $bytes],
        }

        impl $name {
            #[inline]
            fn new(counter: usize) -> Self {
                Self {
                    counter,
                    payload: [0; $bytes],
                }
            }
        }

        impl Node for $name {
            #[inline]
            fn value(&self) -> usize {
                self.counter + usize::from(self.payload[0])
            }

            #[inline]
            fn bump(&mut self) {
                self.counter += 1;
            }
        }
    };
}

node!(Tiny, 8);
node!(Medium, 48);
node!(Large, 112);

fn project<U: Node + 'static>(value: &U) -> &(dyn Node + 'static) {
    value
}

fn project_mut<U: Node + 'static>(value: &mut U) -> &mut (dyn Node + 'static) {
    value
}

fn measure(name: &str, operations: usize, mut body: impl FnMut()) {
    let mut best = f64::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        body();
        let elapsed = start.elapsed().as_secs_f64();
        let per_operation = elapsed * 1e9 / operations as f64;
        if per_operation < best {
            best = per_operation;
        }
    }
    println!("{name:<52} {best:>10.2} ns/op");
}

fn report_layout() {
    println!("--- layout ---");
    println!(
        "{:<52} {:>4} bytes (align {})",
        "Rubick<usize>",
        size_of::<Rubick<usize>>(),
        align_of::<Rubick<usize>>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Rubick<dyn Node>",
        size_of::<Rubick<dyn Node>>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Box<dyn Node>",
        size_of::<Box<dyn Node>>()
    );
    println!("{:<52} {:>4} bytes", "INLINE_CAPACITY", INLINE_CAPACITY);
    println!(
        "{:<52} {:>4} / {} / {} bytes",
        "Tiny / Medium / Large",
        size_of::<Tiny>(),
        size_of::<Medium>(),
        size_of::<Large>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Rubick<dyn Node, 8> (widget capacity)",
        size_of::<Wide>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Rubick<dyn Node, 1> (element capacity)",
        size_of::<Rubick<dyn Node, 1>>()
    );
    let tiny: Rubick<dyn Node> = Rubick::erase(Tiny::new(0));
    let medium: Rubick<dyn Node> = Rubick::erase(Medium::new(0));
    let large: Rubick<dyn Node> = Rubick::erase(Large::new(0));
    println!(
        "{:<52} {} / {} / {}",
        "inline at 4 words? Tiny / Medium / Large",
        tiny.is_inline(),
        medium.is_inline(),
        large.is_inline()
    );
    let wide: Wide = Rubick::erase(Medium::new(0));
    println!("{:<52} {}", "inline at 8 words? Medium", wide.is_inline());
    println!();
}

fn bench_construction() {
    const COUNT: usize = 200_000;

    println!("--- construct + drop ---");
    measure("Rubick::new(usize)", COUNT, || {
        for index in 0..COUNT {
            black_box(Rubick::new(black_box(index)));
        }
    });
    measure("Box::new(usize)", COUNT, || {
        for index in 0..COUNT {
            black_box(Box::new(black_box(index)));
        }
    });
    measure("Rubick<dyn Node> from Tiny", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> =
                Rubick::new_projected(Tiny::new(black_box(index)), project, project_mut);
            black_box(owner);
        }
    });
    measure("Box<dyn Node> from Tiny", COUNT, || {
        for index in 0..COUNT {
            let owner: Box<dyn Node> = Box::new(Tiny::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick<dyn Node> from Medium", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> =
                Rubick::new_projected(Medium::new(black_box(index)), project, project_mut);
            black_box(owner);
        }
    });
    measure("Box<dyn Node> from Medium", COUNT, || {
        for index in 0..COUNT {
            let owner: Box<dyn Node> = Box::new(Medium::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick<dyn Node> from Large", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> =
                Rubick::new_projected(Large::new(black_box(index)), project, project_mut);
            black_box(owner);
        }
    });
    measure("Box<dyn Node> from Large", COUNT, || {
        for index in 0..COUNT {
            let owner: Box<dyn Node> = Box::new(Large::new(black_box(index)));
            black_box(owner);
        }
    });
    println!();

    println!("--- construct + drop, erased without adapters ---");
    measure("Rubick::erase(Tiny)", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> = Rubick::erase(Tiny::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick::erase(Medium)", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> = Rubick::erase(Medium::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick<_, 8>::erase(Medium), inline", COUNT, || {
        for index in 0..COUNT {
            let owner: Wide = Rubick::erase(Medium::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick::erase(Large), pooled block", COUNT, || {
        for index in 0..COUNT {
            let owner: Rubick<dyn Node> = Rubick::erase(Large::new(black_box(index)));
            black_box(owner);
        }
    });
    measure("Rubick::replace(Large), reused block", COUNT, || {
        let mut owner: Rubick<dyn Node> = Rubick::erase(Large::new(0));
        for index in 0..COUNT {
            owner.replace(Large::new(black_box(index)));
        }
        black_box(owner.value());
    });
    println!();
}

fn bench_dispatch() {
    const NODES: usize = 4_096;
    const PASSES: usize = 64;

    println!("--- dynamic dispatch ---");
    let mut owners: Vec<Rubick<dyn Node>> = Vec::with_capacity(NODES);
    let mut boxes: Vec<Box<dyn Node>> = Vec::with_capacity(NODES);
    for index in 0..NODES {
        match index % 3 {
            0 => {
                owners.push(Rubick::new_projected(
                    Tiny::new(index),
                    project,
                    project_mut,
                ));
                boxes.push(Box::new(Tiny::new(index)));
            }
            1 => {
                owners.push(Rubick::new_projected(
                    Medium::new(index),
                    project,
                    project_mut,
                ));
                boxes.push(Box::new(Medium::new(index)));
            }
            _ => {
                owners.push(Rubick::new_projected(
                    Large::new(index),
                    project,
                    project_mut,
                ));
                boxes.push(Box::new(Large::new(index)));
            }
        }
    }

    measure("Rubick<dyn Node>::value", NODES * PASSES, || {
        let mut total = 0_usize;
        for _ in 0..PASSES {
            for owner in &owners {
                total = total.wrapping_add(owner.value());
            }
        }
        black_box(total);
    });
    measure("Box<dyn Node>::value", NODES * PASSES, || {
        let mut total = 0_usize;
        for _ in 0..PASSES {
            for owner in &boxes {
                total = total.wrapping_add(owner.value());
            }
        }
        black_box(total);
    });
    measure("Rubick<dyn Node>::bump", NODES * PASSES, || {
        for _ in 0..PASSES {
            for owner in owners.iter_mut() {
                owner.bump();
            }
        }
    });
    measure("Box<dyn Node>::bump", NODES * PASSES, || {
        for _ in 0..PASSES {
            for owner in boxes.iter_mut() {
                owner.bump();
            }
        }
    });

    let mut erased: Vec<Rubick<dyn Node>> = Vec::with_capacity(NODES);
    for index in 0..NODES {
        match index % 3 {
            0 => erased.push(Rubick::erase(Tiny::new(index))),
            1 => erased.push(Rubick::erase(Medium::new(index))),
            _ => erased.push(Rubick::erase(Large::new(index))),
        }
    }
    measure("Rubick::erase(..)::value", NODES * PASSES, || {
        let mut total = 0_usize;
        for _ in 0..PASSES {
            for owner in &erased {
                total = total.wrapping_add(owner.value());
            }
        }
        black_box(total);
    });
    measure("Rubick::erase(..)::bump", NODES * PASSES, || {
        for _ in 0..PASSES {
            for owner in erased.iter_mut() {
                owner.bump();
            }
        }
    });
    println!();
}

fn bench_tree_rebuild() {
    const NODES: usize = 8_192;
    const FRAMES: usize = 24;
    const UI_MEMORY_LIMIT: usize = 16 * 1024 * 1024;

    println!("--- widget tree rebuild (build, traverse, drop) ---");
    measure("Rubick tree frame", NODES * FRAMES, || {
        for _ in 0..FRAMES {
            let mut tree: Vec<Rubick<dyn Node>> = Vec::with_capacity(NODES);
            for index in 0..NODES {
                match index % 3 {
                    0 => tree.push(Rubick::new_projected(
                        Tiny::new(index),
                        project,
                        project_mut,
                    )),
                    1 => tree.push(Rubick::new_projected(
                        Medium::new(index),
                        project,
                        project_mut,
                    )),
                    _ => tree.push(Rubick::new_projected(
                        Large::new(index),
                        project,
                        project_mut,
                    )),
                }
            }
            let mut total = 0_usize;
            for node in &tree {
                total = total.wrapping_add(node.value());
            }
            black_box(total);
        }
    });
    measure("Rubick erased tree frame", NODES * FRAMES, || {
        for _ in 0..FRAMES {
            let mut tree: Vec<Rubick<dyn Node>> = Vec::with_capacity(NODES);
            for index in 0..NODES {
                match index % 3 {
                    0 => tree.push(Rubick::erase(Tiny::new(index))),
                    1 => tree.push(Rubick::erase(Medium::new(index))),
                    _ => tree.push(Rubick::erase(Large::new(index))),
                }
            }
            let mut total = 0_usize;
            for node in &tree {
                total = total.wrapping_add(node.value());
            }
            black_box(total);
        }
    });
    measure(
        "Rubick erased tree frame, widget capacity",
        NODES * FRAMES,
        || {
            for _ in 0..FRAMES {
                let mut tree: Vec<Wide> = Vec::with_capacity(NODES);
                for index in 0..NODES {
                    match index % 3 {
                        0 => tree.push(Rubick::erase(Tiny::new(index))),
                        1 => tree.push(Rubick::erase(Medium::new(index))),
                        _ => tree.push(Rubick::erase(Large::new(index))),
                    }
                }
                let mut total = 0_usize;
                for node in &tree {
                    total = total.wrapping_add(node.value());
                }
                black_box(total);
            }
        },
    );
    let ui_memory = UiMemory::new(UI_MEMORY_LIMIT);
    let ui_allocator = ui_memory.allocator();
    measure(
        "UiMemory erased tree frame, matched capacity",
        NODES * FRAMES,
        || {
            ui_allocator.scope(|| {
                for _ in 0..FRAMES {
                    let mut tree: Vec<Rubick<dyn Node>> = Vec::with_capacity(NODES);
                    for index in 0..NODES {
                        match index % 3 {
                            0 => tree.push(Rubick::erase(Tiny::new(index))),
                            1 => tree.push(Rubick::erase(Medium::new(index))),
                            _ => tree.push(Rubick::erase(Large::new(index))),
                        }
                    }
                    let mut total = 0_usize;
                    for node in &tree {
                        total = total.wrapping_add(node.value());
                    }
                    black_box(total);
                }
            });
        },
    );
    measure(
        "UiMemory erased tree frame, element capacity",
        NODES * FRAMES,
        || {
            ui_allocator.scope(|| {
                for _ in 0..FRAMES {
                    let mut tree: Vec<Rubick<dyn Node, 1>> = Vec::with_capacity(NODES);
                    for index in 0..NODES {
                        match index % 3 {
                            0 => tree.push(Rubick::erase(Tiny::new(index))),
                            1 => tree.push(Rubick::erase(Medium::new(index))),
                            _ => tree.push(Rubick::erase(Large::new(index))),
                        }
                    }
                    let mut total = 0_usize;
                    for node in &tree {
                        total = total.wrapping_add(node.value());
                    }
                    black_box(total);
                }
            });
        },
    );
    measure("Box tree frame", NODES * FRAMES, || {
        for _ in 0..FRAMES {
            let mut tree: Vec<Box<dyn Node>> = Vec::with_capacity(NODES);
            for index in 0..NODES {
                match index % 3 {
                    0 => tree.push(Box::new(Tiny::new(index))),
                    1 => tree.push(Box::new(Medium::new(index))),
                    _ => tree.push(Box::new(Large::new(index))),
                }
            }
            let mut total = 0_usize;
            for node in &tree {
                total = total.wrapping_add(node.value());
            }
            black_box(total);
        }
    });
    println!();
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("aimer_rubick benchmark ({profile} profile)\n");
    report_layout();
    bench_construction();
    bench_dispatch();
    bench_tree_rebuild();
}
