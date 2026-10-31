//! Head-to-head timing of the borrowing and the consuming widget conversion.
//!
//! The allocation experiments in [`aimer_laboratory::experiment`] answer *how
//! many times a build reaches the allocator*. They do not answer *how long a
//! build takes*, and the two are not the same question: the consuming
//! conversion removes clones but adds one bit copy of the whole widget out of
//! its erased storage, so a widget owning no heap data could plausibly come out
//! slower.
//!
//! Production no longer contains the borrowing conversion, so it is
//! reconstructed here — the same erased owner, the same element type, the same
//! node shapes, and clones exactly where the old implementations cloned. Four
//! scenarios are timed:
//!
//! 1. **decorated tree** — nodes owning a `Vec`, the shape the migration
//!    targeted; the borrowing side clones the vector per node per frame.
//! 2. **scalar widget** — no heap field at all, the shape where the extra bit
//!    copy has nothing to pay for itself with.
//! 3. **pooled widget** — a payload wider than the inline budget, so the bit
//!    copy is a read out of a pooled block rather than out of the handle.
//! 4. **self-rebuilding parent** — a hover, a scroll offset, a theme tick: the
//!    borrowing side re-runs its child subtree, the consuming side places the
//!    element it already has.
//!
//! ```text
//! cargo run -p aimer_laboratory --example consuming_conversion_benchmark
//! cargo run -p aimer_laboratory --example consuming_conversion_benchmark --release
//! ```
//!
//! The two sides of a scenario are timed in **alternating** rounds rather than
//! one after the other. Measured back to back, a first run of this benchmark
//! reported the decorated tree 14% faster and the scalar widget 6.6% slower,
//! and a second run reported the exact opposite — the drift of the machine over
//! a few hundred milliseconds is larger than the effect being measured, so
//! whichever side runs second wins or loses by accident. Alternating puts both
//! sides under the same drift, and the reported minimum discards the rounds
//! that were interrupted as well as the cold round that pays for the pool. The
//! median is printed next to it: when the two disagree, the scenario is noise
//! and its verdict must not be trusted. The unit is nanoseconds per conversion.
//!
//! Each side also reports the allocations it spent per conversion, counted by a
//! recording global allocator. A timing difference the allocation counts cannot
//! explain is a measurement artefact, and an allocation difference that costs no
//! time is a statement about this machine's allocator rather than about the
//! conversion — the calibration scenario prices one allocation so the reader can
//! tell those two apart.
//!
//! # Observed
//!
//! Release profile, Apple silicon, repeated runs in agreement:
//!
//! | scenario | borrowing | consuming | verdict |
//! |---|---|---|---|
//! | decorated tree, per node | 41.2 ns, 2 allocs | 34.5 ns, 1 alloc | 10–16% faster |
//! | scalar widget, 16 bytes | 1.21 ns, 0 allocs | 1.22 ns, 0 allocs | no difference |
//! | 7-word widget, inline | 4.2 ns, 0 allocs | 3.6 ns, 0 allocs | 13–16% faster |
//! | 12-word widget, pooled | 6.8 ns, 0 allocs | 9.0 ns, 0 allocs | 29–39% slower |
//! | self-rebuild, per frame | 1317 ns, 64 allocs | 2.9 ns, 0 allocs | 99.8% faster |
//!
//! Three readings, in ascending order of importance.
//!
//! **The win on a heap-owning node is the allocation.** A decorated node spends
//! two allocations per build borrowing and one consuming, and the calibration
//! prices that pair at about 11 ns — which is the whole 6.7 ns/node gain once
//! the rest of the build is counted alongside it. Nothing else explains it, and
//! nothing else has to.
//!
//! **The copy the consuming conversion adds is not what costs.** That was the
//! expectation going in, and the two payload scenarios refute it: a widget of
//! 56 bytes, sitting in the handle's own storage, converts *faster* consuming
//! than borrowing, because the whole value is read out at once instead of field
//! by field through a reference. The regression appears only when the payload
//! spills out of the eight-word budget into a pooled block — then the read
//! crosses a pointer and `Rubick::take` adds its vacant-table store and its
//! release guard on top, for about 2.3 ns. So the number to tune is the inline
//! budget, not the signature.
//!
//! **The scenario that decides the migration is the last one.** A widget that
//! rebuilds itself over a retained child does not rebuild its subtree at all:
//! 64 allocations and 1.3 µs per frame become zero allocations and 3 ns. Every
//! other line on this table is a percentage; this one is three orders of
//! magnitude, and it is what a hover, a scroll frame, and a theme tick actually
//! cost now.
//!
//! The widgets above are stand-ins, and the third reading is the one that was
//! worth chasing on the real ones. `production_widget_benchmark.rs` duels the
//! shipping framework against the released pre-migration one, and when it was
//! first run a `Container` was 512 bytes against this same eight-word budget:
//! always pooled, so the decorated tree came out a wash rather than 10–16%
//! faster and the plain tree came out 40% *slower*. Packing a color into one
//! word then took `BoxDecoration` from 336 to 192 bytes and `Container` from 512
//! to 352, and both scenarios flipped to the consuming side. So the tunable is
//! the payload against the budget, exactly as the third reading says — and it is
//! tunable from either end.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use aimer_time::Instant;

use aimer_laboratory::experiment::Shadow;

/// Counts every allocation performed by the current thread.
///
/// The counter is thread local and const initialized, so recording costs one
/// non-atomic increment and never allocates itself. That increment is on both
/// sides of every scenario, so it cannot invent a difference between them —
/// though it does make an allocation look marginally more expensive than it is.
struct RecordingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Every method forwards to `System` unchanged; the counter only
// observes the call.
unsafe impl GlobalAlloc for RecordingAllocator {
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
static ALLOCATOR: RecordingAllocator = RecordingAllocator;

fn record() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

const ROUNDS: usize = 15;

/// Nodes in one decorated tree, and rebuilds in one self-rebuild scenario.
const DEPTH: usize = 64;
const FRAMES: usize = 512;
const LEAVES: usize = 100_000;
const REBUILDS: usize = 20_000;

/// The shadow list a decorated node carries.
///
/// Two entries is what a real card uses, and it is the value the borrowing
/// conversion has to clone.
fn shadows() -> Vec<Shadow> {
    vec![
        Shadow {
            blur: 4.0,
            spread: 1.0,
        },
        Shadow {
            blur: 8.0,
            spread: 2.0,
        },
    ]
}

/// The conversion as it was before the migration.
///
/// Nothing here is production code any more; it is the baseline the consuming
/// conversion is measured against. The erased owner, its inline budget, and the
/// element type are the same on both sides, so the only difference the numbers
/// can express is the conversion itself.
mod borrowing {
    use std::ptr;
    use std::rc::Rc;

    use aimer_laboratory::experiment::Shadow;
    use aimer_laboratory::{AnyElement, Element};
    use aimer_rubick::{ErasedFrom, Rubick};

    /// Machine words the erased widget reserves, matching the consuming side.
    const WIDGET_WORDS: usize = 8;

    /// A widget that is *read* rather than consumed.
    ///
    /// Because the method borrows, every field the element keeps has to be
    /// copied out of the widget, and the widget is destroyed immediately
    /// afterwards — which is the waste the migration removed.
    pub trait Widget: 'static {
        /// Reads this widget and produces its element.
        fn to_element(&self) -> AnyElement;
    }

    // SAFETY: The template is `null::<W>()` coerced to the target, so it
    // carries exactly `W`'s vtable and a null data address.
    unsafe impl<W: Widget> ErasedFrom<W> for dyn Widget {
        const TEMPLATE: *const Self = ptr::null::<W>() as *const dyn Widget;
    }

    /// An owned, type-erased borrowing widget.
    pub struct AnyWidget(Rubick<dyn Widget, WIDGET_WORDS>);

    impl AnyWidget {
        /// Erases a concrete widget.
        #[inline]
        pub fn new<W: Widget>(widget: W) -> Self {
            Self(Rubick::erase(widget))
        }

        /// Builds the element, then destroys the widget.
        ///
        /// This is the whole cost the old framework paid at the root of a
        /// build: the conversion reads through the handle, and the handle —
        /// with the widget still inside it — is dropped on the next line.
        #[inline]
        pub fn into_element(self) -> AnyElement {
            let element = self.0.to_element();
            drop(self);
            element
        }
    }

    /// An erased widget is itself a widget, so handles nest.
    impl Widget for AnyWidget {
        #[inline]
        fn to_element(&self) -> AnyElement {
            self.0.to_element()
        }
    }

    /// A node that owns heap data and one child, modelling `Container`.
    pub struct Decorated {
        shadows: Vec<Shadow>,
        child: AnyWidget,
    }

    impl Decorated {
        /// Creates a decorated node around `child`.
        #[inline]
        pub fn new(shadows: Vec<Shadow>, child: AnyWidget) -> Self {
            Self { shadows, child }
        }
    }

    struct DecoratedElement {
        _shadows: Vec<Shadow>,
        child: AnyElement,
    }

    impl Element for DecoratedElement {
        fn debug_name(&self) -> &'static str {
            "DecoratedElement"
        }

        fn rebuild(&mut self) {
            self.child.rebuild();
        }
    }

    impl Widget for Decorated {
        #[inline]
        fn to_element(&self) -> AnyElement {
            AnyElement::new(DecoratedElement {
                // The archetype of the migration: a heap allocation per node
                // per build, thrown away one line later.
                _shadows: self.shadows.clone(),
                child: self.child.to_element(),
            })
        }
    }

    /// A leaf node, terminating a tree.
    pub struct Leaf;

    struct LeafElement;

    impl Element for LeafElement {
        fn debug_name(&self) -> &'static str {
            "LeafElement"
        }
    }

    impl Widget for Leaf {
        #[inline]
        fn to_element(&self) -> AnyElement {
            AnyElement::new(LeafElement)
        }
    }

    /// A node holding nothing but scalars, so the conversion has no heap field
    /// to save.
    pub struct Scalar {
        pub width: f32,
        pub height: f32,
        pub radius: f32,
        pub flags: u32,
    }

    struct ScalarElement {
        _width: f32,
        _height: f32,
        _radius: f32,
        _flags: u32,
    }

    impl Element for ScalarElement {
        fn debug_name(&self) -> &'static str {
            "ScalarElement"
        }
    }

    impl Widget for Scalar {
        #[inline]
        fn to_element(&self) -> AnyElement {
            AnyElement::new(ScalarElement {
                _width: self.width,
                _height: self.height,
                _radius: self.radius,
                _flags: self.flags,
            })
        }
    }

    /// A node whose payload size is the variable under test.
    ///
    /// Below the inline budget the payload lives in the handle's own bytes;
    /// above it, in a pooled block. The borrowing conversion reads the fields
    /// where they are in both cases.
    pub struct Bulky<const N: usize> {
        pub words: [usize; N],
    }

    struct BulkyElement<const N: usize> {
        _words: [usize; N],
    }

    impl<const N: usize> Element for BulkyElement<N> {
        fn debug_name(&self) -> &'static str {
            "BulkyElement"
        }
    }

    impl<const N: usize> Widget for Bulky<N> {
        #[inline]
        fn to_element(&self) -> AnyElement {
            AnyElement::new(BulkyElement { _words: self.words })
        }
    }

    /// A node that rebuilds itself, modelling `Button` before the migration.
    ///
    /// The child is an `Rc` because that is the only way a borrowing
    /// conversion can hand the same child to the element it produces *and*
    /// keep it for the next rebuild.
    pub struct SelfRebuilding<W: Widget> {
        child: Rc<W>,
    }

    impl<W: Widget> SelfRebuilding<W> {
        /// Creates a self-rebuilding node around a shared child.
        #[inline]
        pub fn new(child: W) -> Self {
            Self {
                child: Rc::new(child),
            }
        }
    }

    struct SelfRebuildingElement<W: Widget> {
        child: Rc<W>,
        placement: AnyElement,
    }

    impl<W: Widget> Element for SelfRebuildingElement<W> {
        fn debug_name(&self) -> &'static str {
            "SelfRebuildingElement"
        }

        fn rebuild(&mut self) {
            // The whole subtree is produced again, which is what a hover cost
            // before the child could be retained.
            self.placement = self.child.to_element();
        }
    }

    impl<W: Widget> Widget for SelfRebuilding<W> {
        #[inline]
        fn to_element(&self) -> AnyElement {
            AnyElement::new(SelfRebuildingElement {
                child: Rc::clone(&self.child),
                placement: self.child.to_element(),
            })
        }
    }
}

/// The conversion as it ships, plus the two node shapes the laboratory does
/// not already model.
mod consuming {
    use aimer_laboratory::{AnyElement, Element, Widget};

    /// A node holding nothing but scalars.
    pub struct Scalar {
        pub width: f32,
        pub height: f32,
        pub radius: f32,
        pub flags: u32,
    }

    struct ScalarElement {
        _width: f32,
        _height: f32,
        _radius: f32,
        _flags: u32,
    }

    impl Element for ScalarElement {
        fn debug_name(&self) -> &'static str {
            "ScalarElement"
        }
    }

    impl Widget for Scalar {
        #[inline]
        fn to_element(self) -> AnyElement {
            AnyElement::new(ScalarElement {
                _width: self.width,
                _height: self.height,
                _radius: self.radius,
                _flags: self.flags,
            })
        }
    }

    /// A node whose payload size is the variable under test.
    ///
    /// The consuming conversion copies the whole payload out of its storage
    /// before the concrete method runs, so this is the shape that says whether
    /// that copy costs anything and whether it costs more once the storage is a
    /// pooled block rather than the handle itself.
    pub struct Bulky<const N: usize> {
        pub words: [usize; N],
    }

    struct BulkyElement<const N: usize> {
        _words: [usize; N],
    }

    impl<const N: usize> Element for BulkyElement<N> {
        fn debug_name(&self) -> &'static str {
            "BulkyElement"
        }
    }

    impl<const N: usize> Widget for Bulky<N> {
        #[inline]
        fn to_element(self) -> AnyElement {
            AnyElement::new(BulkyElement { _words: self.words })
        }
    }
}

/// Words in the payload that still fits an erased widget's eight-word budget,
/// so its storage is the handle itself.
const INLINE_WORDS: usize = 7;

/// Words in the payload that exceeds the budget, so its storage is a pooled
/// block and the consuming read crosses a pointer to reach it.
const POOLED_WORDS: usize = 12;

/// One round of a scenario, in nanoseconds per operation.
fn round(operations: usize, body: &mut impl FnMut()) -> f64 {
    let start = Instant::now();
    body();
    start.elapsed().as_secs_f64() * 1e9 / operations as f64
}

/// The minimum and the median of a scenario's rounds.
fn summarize(samples: &mut [f64]) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    (samples[0], samples[samples.len() / 2])
}

/// Allocations one operation of `body` spends, on a warm pool.
///
/// The first call warms the pool the way a running application does: the first
/// frame pays for its blocks and the steady state reuses them, so only the
/// second call is counted.
fn allocations_per_operation(operations: usize, body: &mut impl FnMut()) -> f64 {
    body();
    let start = allocations();
    body();
    (allocations() - start) as f64 / operations as f64
}

/// Times one scenario that has no counterpart to be compared against.
fn solo(name: &str, operations: usize, mut body: impl FnMut()) {
    let spent = allocations_per_operation(operations, &mut body);
    let mut samples = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        samples.push(round(operations, &mut body));
    }
    let (best, median) = summarize(&mut samples);
    println!("{name:<46} {best:>9.2} {median:>9.2} {spent:>9.2}\n");
}

/// Times both sides of a scenario against each other.
///
/// The rounds alternate, so a machine that slows down halfway through the
/// measurement slows both sides down equally instead of handing the verdict to
/// whichever side happened to run first.
fn duel(
    operations: usize,
    borrowing_name: &str,
    mut borrowing: impl FnMut(),
    consuming_name: &str,
    mut consuming: impl FnMut(),
) {
    let borrowing_allocations = allocations_per_operation(operations, &mut borrowing);
    let consuming_allocations = allocations_per_operation(operations, &mut consuming);

    let mut borrowing_samples = Vec::with_capacity(ROUNDS);
    let mut consuming_samples = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        borrowing_samples.push(round(operations, &mut borrowing));
        consuming_samples.push(round(operations, &mut consuming));
    }

    let (borrowed, borrowed_median) = summarize(&mut borrowing_samples);
    let (consumed, consumed_median) = summarize(&mut consuming_samples);
    println!(
        "{borrowing_name:<46} {borrowed:>9.2} {borrowed_median:>9.2} {borrowing_allocations:>9.2}"
    );
    println!(
        "{consuming_name:<46} {consumed:>9.2} {consumed_median:>9.2} {consuming_allocations:>9.2}"
    );

    let best = (consumed - borrowed) / borrowed * 100.0;
    let median = (consumed_median - borrowed_median) / borrowed_median * 100.0;
    let verdict = if best < 0.0 { "faster" } else { "slower" };
    // A verdict is only reported when both statistics point the same way and
    // the smaller of them clears the run-to-run spread of this harness, which
    // repeated runs put at roughly a percent.
    let agreement = if best.signum() != median.signum() || best.abs().min(median.abs()) < 1.0 {
        "   (within noise: no difference)"
    } else {
        ""
    };
    println!(
        "{:<46} {:>8.1}% {:>8.1}% {verdict}{agreement}\n",
        "consuming vs borrowing",
        best.abs(),
        median.abs()
    );
}

fn header(scenario: &str) {
    println!("--- {scenario} ---");
    println!("{:<46} {:>9} {:>9} {:>9}", "", "min", "median", "allocs");
}

/// Prices the allocation the borrowing conversion used to spend per node.
///
/// Without this line a null result in the decorated scenario is unreadable: it
/// could mean the clone was never there, or it could mean an allocation is
/// simply cheap on this machine. The scenario allocates and frees exactly the
/// shadow list `Decorated` carries, which is the value the old implementation
/// cloned.
fn bench_allocation_price() {
    header(&format!(
        "calibration: one shadow-list allocation, {LEAVES} times"
    ));

    solo("malloc + memcpy + free a two-entry Vec", LEAVES, || {
        for _ in 0..LEAVES {
            black_box(shadows());
        }
    });
}

fn borrowing_tree() -> borrowing::AnyWidget {
    let mut widget = borrowing::AnyWidget::new(borrowing::Leaf);
    for _ in 0..DEPTH {
        widget = borrowing::AnyWidget::new(borrowing::Decorated::new(shadows(), widget));
    }
    widget
}

fn consuming_tree() -> aimer_laboratory::AnyWidget {
    use aimer_laboratory::Widget;
    use aimer_laboratory::experiment::{Decorated, Leaf};

    let mut widget = Leaf.boxed();
    for _ in 0..DEPTH {
        widget = Decorated::new(shadows(), widget).boxed();
    }
    widget
}

fn bench_decorated_tree() {
    header(&format!(
        "decorated tree: {DEPTH} nodes owning a Vec, {FRAMES} frames"
    ));

    duel(
        DEPTH * FRAMES,
        "borrowing: clone the shadow list per node",
        || {
            for _ in 0..FRAMES {
                black_box(borrowing_tree().into_element());
            }
        },
        "consuming: move the shadow list per node",
        || {
            for _ in 0..FRAMES {
                black_box(consuming_tree().into_element());
            }
        },
    );
}

fn bench_scalar() {
    header(&format!(
        "scalar widget: no heap field, {LEAVES} conversions"
    ));

    duel(
        LEAVES,
        "borrowing: copy four scalars out",
        || {
            for index in 0..LEAVES {
                let widget = borrowing::AnyWidget::new(borrowing::Scalar {
                    width: black_box(index as f32),
                    height: 24.0,
                    radius: 4.0,
                    flags: 0b1010,
                });
                black_box(widget.into_element());
            }
        },
        "consuming: read the widget out of storage",
        || {
            use aimer_laboratory::Widget;

            for index in 0..LEAVES {
                let widget = consuming::Scalar {
                    width: black_box(index as f32),
                    height: 24.0,
                    radius: 4.0,
                    flags: 0b1010,
                }
                .boxed();
                black_box(widget.into_element());
            }
        },
    );
}

/// Times a payload of `N` words through both conversions.
///
/// Run once inside the inline budget and once outside it, this isolates the one
/// thing the consuming conversion adds: the payload is read out of its storage
/// before the concrete method runs, where the borrowing conversion read the
/// fields where they already were.
fn bench_bulky<const N: usize>(storage: &str) {
    header(&format!(
        "{N}-word widget ({storage}): {LEAVES} conversions"
    ));

    duel(
        LEAVES,
        "borrowing: copy the fields where they are",
        || {
            for index in 0..LEAVES {
                let mut words = [0; N];
                words[0] = black_box(index);
                black_box(borrowing::AnyWidget::new(borrowing::Bulky { words }).into_element());
            }
        },
        "consuming: read the payload out of storage",
        || {
            use aimer_laboratory::Widget;

            for index in 0..LEAVES {
                let mut words = [0; N];
                words[0] = black_box(index);
                black_box(consuming::Bulky { words }.boxed().into_element());
            }
        },
    );
}

fn bench_self_rebuild() {
    header(&format!(
        "self-rebuilding parent over a {DEPTH}-node subtree, {REBUILDS} rebuilds"
    ));

    duel(
        REBUILDS,
        "borrowing: rebuild the subtree per frame",
        || {
            let parent = borrowing::SelfRebuilding::new(borrowing_tree());
            let mut element = borrowing::AnyWidget::new(parent).into_element();
            for _ in 0..REBUILDS {
                element.rebuild();
            }
            black_box(element);
        },
        "consuming: place the retained child per frame",
        || {
            use aimer_laboratory::Widget;
            use aimer_laboratory::{RetainedChild, experiment::SelfRebuilding};

            let child = RetainedChild::new(consuming_tree());
            let mut element = SelfRebuilding::new(child).boxed().into_element();
            for _ in 0..REBUILDS {
                element.rebuild();
            }
            black_box(element);
        },
    );
}

fn report_layout() {
    println!("--- layout ---");
    println!(
        "{:<52} {:>4} bytes",
        "borrowing::Scalar",
        size_of::<borrowing::Scalar>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "consuming::Scalar",
        size_of::<consuming::Scalar>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Bulky<INLINE_WORDS>",
        size_of::<consuming::Bulky<INLINE_WORDS>>()
    );
    println!(
        "{:<52} {:>4} bytes",
        "Bulky<POOLED_WORDS>",
        size_of::<consuming::Bulky<POOLED_WORDS>>()
    );
    println!("{:<52} {:>4} words\n", "inline widget budget", 8);
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("consuming vs borrowing conversion benchmark ({profile} profile)\n");
    report_layout();
    bench_allocation_price();
    bench_decorated_tree();
    bench_scalar();
    bench_bulky::<INLINE_WORDS>("inline storage");
    bench_bulky::<POOLED_WORDS>("pooled storage");
    bench_self_rebuild();
}
