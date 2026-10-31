//! Head-to-head timing of the real framework, before and after the migration.
//!
//! [`consuming_conversion_benchmark`](../consuming_conversion_benchmark.rs)
//! measures the ownership model on stand-in widgets: nodes written for the
//! experiment, an element type with two methods, no build context. It answers
//! whether the *conversion* got cheaper, and it leaves one question open — what
//! the change is worth on `Container`, `Column` and `Button`, where a build also
//! resolves decoration, allocates elements out of a one-word owner, and walks a
//! `BuildContext`.
//!
//! That question needs the pre-migration framework, which no longer exists in
//! this working tree. It exists on the branch it was released from, so both are
//! dependencies here: `aimer` is the shipping consuming framework and
//! `aimer_legacy` is the same crate at `1.97.1-alpha-1`, where
//! `Widget::to_element` still took `&self` and a self-rebuilding parent still
//! held its child in an `Rc`.
//!
//! ```text
//! cargo run -p aimer_laboratory --example production_widget_benchmark --release
//! ```
//!
//! Every scenario builds the *same* tree on both sides, out of the same public
//! builders, and rebuilds the widgets each frame the way a real `build()` does —
//! the consuming side has no choice, and letting the borrowing side keep its
//! widgets would measure a program nobody writes.
//!
//! The harness is the one the sibling benchmark established: alternating rounds
//! so a machine that slows down halfway through cannot hand the verdict to
//! whichever side ran second, the minimum next to the median so a scenario that
//! disagrees with itself is reported as noise, and a recording allocator so a
//! timing difference can be checked against the allocations that should explain
//! it.
//!
//! # Observed
//!
//! Release profile, Apple silicon, repeated runs in agreement. The borrowing
//! side is the released framework and cannot change; the consuming side is this
//! working tree, so the table is re-read whenever something structural moves.
//!
//! | scenario | borrowing | consuming | verdict |
//! |---|---|---|---|
//! | `SizedBox` leaf | 9.5 ns, 0 allocs | 5.2 ns, 0 allocs | 45% faster |
//! | decorated `Container` tree, per node | 78 ns, 3 allocs | 59 ns, 1 alloc | 24–30% faster |
//! | plain `Container` tree, per node | 40.4 ns, 1 alloc | 38.7 ns, 0 allocs | 4% faster |
//! | `Column` child, per child | 12.8 ns, 0.05 allocs | 12.7 ns, 0.03 allocs | within noise |
//! | `Button` dirty rebuild, per frame | 5041 ns, 109 allocs | 3280 ns, 44 allocs | 35% faster |
//!
//! Compare timings across scenarios rather than across runs: the absolute
//! numbers move with the machine, and only the ratios and the allocation counts
//! have been stable across runs.
//!
//! Four readings, in ascending order of importance.
//!
//! **A child list barely moves.** Moving each handle out of the `Vec` instead of
//! building it through a reference saves the copy of a 72-byte owner and one
//! allocation per twenty children, which is a percent or two — and the erased
//! child underneath is converted the same way on both sides, so this scenario is
//! really the container result diluted by sixty-four leaves.
//!
//! **Widget size decides the conversion, and the payload was the fix.** An
//! erased widget reserves 64 bytes inline; a payload past that lives in a pooled
//! block, and the consuming conversion has to copy it *out* of that block before
//! the concrete `to_element` runs, where the borrowing one read its fields where
//! they already were. When this benchmark was first written a `Container` was
//! **512 bytes**, the plain tree came out 40% *slower* consuming, and the
//! decorated tree — the scenario the migration was designed for — landed inside
//! the noise. That was a payload problem, not a signature problem: two thirds of
//! a `Container` was its inline `BoxDecoration`, and two thirds of *that* was
//! eight colors carrying HSLA components. Storing a color as one packed word
//! took `BoxDecoration` from 336 to 192 bytes, `Container` from 512 to 352,
//! `SizedBox` from 40 to 24, and every scenario above flipped to the consuming
//! side.
//!
//! **The allocation counts show where that went.** A container element also
//! shrank below 512 bytes, the largest class `aimer_rubick`'s pool serves, so it
//! is now recycled instead of allocated and freed per node per frame: the plain
//! tree pays 0 allocations against 1, and the decorated tree 1 against 3. This
//! is the reading worth carrying forward — a widget or element that crosses
//! either threshold, the 64-byte inline budget or the 512-byte pooled class,
//! costs far more than any signature choice.
//!
//! **The retained child is still the biggest number.** A dirty-marked frame
//! under a `Button` — a hover, a press, a scroll offset, a resize — costs a
//! third less time and 65 fewer allocations, because the child element is placed
//! again instead of being built again. That is the scenario a user feels, and the
//! allocation count also shows the limitation the migration left open: 44
//! allocations per frame is not zero, because a re-placed retained child is
//! still marked needs-rebuild and its own build closures run again.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use aimer_time::Instant;

/// Counts every allocation performed by the current thread.
///
/// The counter is thread local and const initialized, so recording costs one
/// non-atomic increment and never allocates itself. It sits on both sides of
/// every scenario, so it cannot invent a difference between them.
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

const ROUNDS: usize = 11;

/// Nested containers in one decorated tree.
const DEPTH: usize = 32;

/// Frames of a tree build.
const FRAMES: usize = 64;

/// Children in the wide column.
const CHILDREN: usize = 64;

/// Frames of a dirty-marked rebuild.
const REBUILDS: usize = 2_000;

/// Conversions of a single leaf widget.
const LEAVES: usize = 100_000;

/// The shipping framework.
mod consuming {
    use aimer::style::{BoxDecoration, BoxShadow};
    use aimer::{AnyWidget, Button, Column, Container, SizedBox, Widget};

    use super::{CHILDREN, DEPTH};

    /// The decoration a real card carries, and the value the borrowing
    /// conversion had to clone per node per frame.
    fn decoration() -> BoxDecoration {
        BoxDecoration::new()
            .border_radius(8.0)
            .add_shadow(BoxShadow::new())
            .add_shadow(BoxShadow::new())
    }

    /// A tree of decorated containers over an empty leaf.
    pub fn decorated_tree() -> AnyWidget {
        let mut widget = SizedBox::new().boxed();
        for _ in 0..DEPTH {
            widget = Container::new()
                .box_decoration(decoration())
                .box_child(widget);
        }
        widget
    }

    /// The same tree with nothing to decorate, so a node owns no heap field.
    pub fn plain_tree() -> AnyWidget {
        let mut widget = SizedBox::new().boxed();
        for _ in 0..DEPTH {
            widget = Container::new().box_child(widget);
        }
        widget
    }

    /// A column of leaves, the shape that moves a `Vec<AnyWidget>` into its
    /// element.
    pub fn wide_column() -> AnyWidget {
        Column::new()
            .children((0..CHILDREN).map(|_| SizedBox::new().boxed()))
            .boxed()
    }

    /// A self-rebuilding parent over a decorated subtree.
    pub fn interactive_tree() -> AnyWidget {
        Button::new().box_child(decorated_tree())
    }

    /// The one production widget small enough to be stored inline.
    pub fn small_leaf() -> AnyWidget {
        SizedBox::new().width(8.0).height(8.0).boxed()
    }
}

/// The framework as it was before the migration.
mod borrowing {
    use aimer_legacy::style::{BoxDecoration, BoxShadow};
    use aimer_legacy::{AnyWidget, Button, Column, Container, SizedBox, Widget};

    use super::{CHILDREN, DEPTH};

    fn decoration() -> BoxDecoration {
        BoxDecoration::new()
            .border_radius(8.0)
            .add_shadow(BoxShadow::new())
            .add_shadow(BoxShadow::new())
    }

    /// A tree of decorated containers over an empty leaf.
    pub fn decorated_tree() -> AnyWidget {
        let mut widget = SizedBox::new().boxed();
        for _ in 0..DEPTH {
            widget = Container::new()
                .box_decoration(decoration())
                .box_child(widget);
        }
        widget
    }

    /// The same tree with nothing to decorate.
    pub fn plain_tree() -> AnyWidget {
        let mut widget = SizedBox::new().boxed();
        for _ in 0..DEPTH {
            widget = Container::new().box_child(widget);
        }
        widget
    }

    /// A column of leaves, built by copying each handle out of the widget.
    pub fn wide_column() -> AnyWidget {
        Column::new()
            .children((0..CHILDREN).map(|_| SizedBox::new().boxed()))
            .boxed()
    }

    /// A self-rebuilding parent over a decorated subtree, holding its child in
    /// an `Rc` so it can build it again.
    pub fn interactive_tree() -> AnyWidget {
        Button::new().box_child(decorated_tree())
    }

    /// The one production widget small enough to be stored inline.
    pub fn small_leaf() -> AnyWidget {
        SizedBox::new().width(8.0).height(8.0).boxed()
    }
}

/// A build context that talks to no window and no GPU.
///
/// The canvas is leaked because a `BuildContext` borrows it and the benchmark
/// keeps one for its whole run; a leak of a single canvas is cheaper than
/// threading a lifetime through every scenario.
fn consuming_context() -> aimer::BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    aimer::BuildContext::new(
        canvas,
        Default::default(),
        1.0,
        Default::default(),
        Default::default(),
        aimer::aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

fn borrowing_context() -> aimer_legacy::BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas_legacy::InnerCanvas::new()));
        aimer_canvas_legacy::FrameCanvas::new(inner)
    };
    aimer_legacy::BuildContext::new(
        canvas,
        Default::default(),
        1.0,
        Default::default(),
        Default::default(),
        aimer_legacy::aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

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
/// The first call warms the element pool the way a running application does:
/// the first frame pays for its blocks and the steady state reuses them, so
/// only the second call is counted.
fn allocations_per_operation(operations: usize, body: &mut impl FnMut()) -> f64 {
    body();
    let start = allocations();
    body();
    (allocations() - start) as f64 / operations as f64
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
    // A verdict is only reported when the two statistics point the same way,
    // when the smaller of them clears the run-to-run spread of this harness
    // (repeated runs put that at roughly a percent), and when they agree within
    // a factor of two — a scenario whose minimum says 9% and whose median says
    // 2% has measured the machine, not the conversion.
    let smaller = best.abs().min(median.abs());
    let larger = best.abs().max(median.abs());
    let agreement = if best.signum() != median.signum() || smaller < 1.0 || larger > 2.0 * smaller {
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

/// Prints the payload sizes the erased owner has to store.
///
/// This is the explanatory variable for every scenario below. An erased widget
/// reserves eight machine words inline; a payload larger than that lives in a
/// pooled block, and the consuming conversion then reads it *out* of that block
/// before the concrete `to_element` runs, where the borrowing one read its
/// fields where they already were. Production widgets are far past the budget,
/// so the read is a real copy and not a free one.
fn report_layout() {
    println!("--- payload layout ---");
    for (name, size) in [
        ("borrowing: AnyWidget", size_of::<aimer_legacy::AnyWidget>()),
        ("consuming: AnyWidget", size_of::<aimer::AnyWidget>()),
        (
            "borrowing: Container<AnyWidget>",
            size_of::<aimer_legacy::Container<aimer_legacy::AnyWidget>>(),
        ),
        (
            "consuming: Container<AnyWidget>",
            size_of::<aimer::Container<aimer::AnyWidget>>(),
        ),
        (
            "borrowing: Button<AnyWidget>",
            size_of::<aimer_legacy::Button<aimer_legacy::AnyWidget>>(),
        ),
        (
            "consuming: Button<AnyWidget>",
            size_of::<aimer::Button<aimer::AnyWidget>>(),
        ),
        (
            "borrowing: SizedBox (leaf)",
            size_of::<aimer_legacy::SizedBox>(),
        ),
        ("consuming: SizedBox (leaf)", size_of::<aimer::SizedBox>()),
    ] {
        println!("{name:<46} {size:>9} bytes");
    }
    println!(
        "{:<46} {:>9} bytes\n",
        "inline budget of an erased widget",
        8 * size_of::<usize>()
    );
}

fn header(scenario: &str) {
    println!("--- {scenario} ---");
    println!("{:<46} {:>9} {:>9} {:>9}", "", "min", "median", "allocs");
}

/// The archetype of the migration: a container whose decoration owns a shadow
/// list, which the borrowing conversion cloned once per node per frame.
fn bench_decorated_tree(
    consuming_ctx: &aimer::BuildContext<'_>,
    borrowing_ctx: &aimer_legacy::BuildContext<'_>,
) {
    header(&format!(
        "decorated Container tree: {DEPTH} nodes, {FRAMES} frames, per node"
    ));

    duel(
        DEPTH * FRAMES,
        "borrowing: clone the decoration per node",
        || {
            for _ in 0..FRAMES {
                black_box(aimer_legacy::Widget::to_element(
                    &borrowing::decorated_tree(),
                    borrowing_ctx,
                ));
            }
        },
        "consuming: move the decoration per node",
        || {
            for _ in 0..FRAMES {
                black_box(aimer::Widget::to_element(
                    consuming::decorated_tree(),
                    consuming_ctx,
                ));
            }
        },
    );
}

/// The same tree with no heap field to save, isolating what the conversion
/// costs when it has nothing to gain.
fn bench_plain_tree(
    consuming_ctx: &aimer::BuildContext<'_>,
    borrowing_ctx: &aimer_legacy::BuildContext<'_>,
) {
    header(&format!(
        "plain Container tree: {DEPTH} nodes, {FRAMES} frames, per node"
    ));

    duel(
        DEPTH * FRAMES,
        "borrowing: copy the fields per node",
        || {
            for _ in 0..FRAMES {
                black_box(aimer_legacy::Widget::to_element(
                    &borrowing::plain_tree(),
                    borrowing_ctx,
                ));
            }
        },
        "consuming: move the fields per node",
        || {
            for _ in 0..FRAMES {
                black_box(aimer::Widget::to_element(
                    consuming::plain_tree(),
                    consuming_ctx,
                ));
            }
        },
    );
}

/// A child list: `iter().map(to_element)` against `into_iter()`.
fn bench_wide_column(
    consuming_ctx: &aimer::BuildContext<'_>,
    borrowing_ctx: &aimer_legacy::BuildContext<'_>,
) {
    header(&format!(
        "Column of {CHILDREN} children, {FRAMES} frames, per child"
    ));

    duel(
        CHILDREN * FRAMES,
        "borrowing: build each child through a reference",
        || {
            for _ in 0..FRAMES {
                black_box(aimer_legacy::Widget::to_element(
                    &borrowing::wide_column(),
                    borrowing_ctx,
                ));
            }
        },
        "consuming: move each child out of the list",
        || {
            for _ in 0..FRAMES {
                black_box(aimer::Widget::to_element(
                    consuming::wide_column(),
                    consuming_ctx,
                ));
            }
        },
    );
}

/// A frame of a parent rebuilding itself: a hover, a scroll offset, a resize.
///
/// The borrowing side rebuilds its child subtree out of the `Rc` it kept; the
/// consuming side places the child element it already has. This is the scenario
/// the retained child slot exists for.
fn bench_dirty_rebuild(
    consuming_ctx: &aimer::BuildContext<'_>,
    borrowing_ctx: &aimer_legacy::BuildContext<'_>,
) {
    header(&format!(
        "Button over a {DEPTH}-node subtree: {REBUILDS} dirty rebuilds, per frame"
    ));

    duel(
        REBUILDS,
        "borrowing: rebuild the subtree per frame",
        || {
            use aimer_legacy::Rebuildable;

            let element =
                aimer_legacy::Widget::to_element(&borrowing::interactive_tree(), borrowing_ctx);
            for _ in 0..REBUILDS {
                element.mark_needs_rebuild();
                element.rebuild_if_dirty(borrowing_ctx);
            }
            black_box(element);
        },
        "consuming: place the retained child per frame",
        || {
            use aimer::Rebuildable;

            let element = aimer::Widget::to_element(consuming::interactive_tree(), consuming_ctx);
            for _ in 0..REBUILDS {
                element.mark_needs_rebuild();
                element.rebuild_if_dirty(consuming_ctx);
            }
            black_box(element);
        },
    );
}

/// The one production shape that fits the inline budget.
///
/// If the regression on the container tree is the copy out of a pooled block,
/// then a widget whose payload never leaves the handle must not show it — the
/// laboratory's stand-ins put a 7-word widget *ahead* of the borrowing
/// conversion, and this is the same claim on a real type.
fn bench_small_leaf(
    consuming_ctx: &aimer::BuildContext<'_>,
    borrowing_ctx: &aimer_legacy::BuildContext<'_>,
) {
    header(&format!(
        "SizedBox leaf (inline storage): {LEAVES} conversions"
    ));

    duel(
        LEAVES,
        "borrowing: copy the fields where they are",
        || {
            for _ in 0..LEAVES {
                black_box(aimer_legacy::Widget::to_element(
                    &borrowing::small_leaf(),
                    borrowing_ctx,
                ));
            }
        },
        "consuming: read the widget out of the handle",
        || {
            for _ in 0..LEAVES {
                black_box(aimer::Widget::to_element(
                    consuming::small_leaf(),
                    consuming_ctx,
                ));
            }
        },
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("production widget benchmark, consuming vs borrowing ({profile} profile)\n");
    report_layout();

    let consuming_ctx = consuming_context();
    let borrowing_ctx = borrowing_context();

    bench_small_leaf(&consuming_ctx, &borrowing_ctx);
    bench_decorated_tree(&consuming_ctx, &borrowing_ctx);
    bench_plain_tree(&consuming_ctx, &borrowing_ctx);
    bench_wide_column(&consuming_ctx, &borrowing_ctx);
    bench_dirty_rebuild(&consuming_ctx, &borrowing_ctx);
}
