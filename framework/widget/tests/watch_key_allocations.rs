//! `watch` runs on every build of the widget that owns the future, so the key
//! it is given must not cost an allocation when the future is already running.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;

use aimer_attribute::ResolvedSize;
use aimer_venus::Venus;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, FutureSnapshotExt, Key, LayoutElement,
    Rebuildable, StatelessElement, VisitorElement,
};

struct RecordingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: All allocation operations forward unchanged to System. The counter
// only observes allocations made on the current test thread.
unsafe impl GlobalAlloc for RecordingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: RecordingAllocator = RecordingAllocator;

fn allocations(work: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.with(Cell::get);
    work();
    ALLOCATIONS.with(Cell::get) - before
}

#[test]
fn numeric_and_tagged_keys_are_built_without_allocating() {
    assert_eq!(allocations(|| drop(black_box(Key::from(black_box(7u64))))), 0);
    assert_eq!(allocations(|| drop(black_box(Key::from(black_box(-7i32))))), 0);
    assert_eq!(allocations(|| drop(black_box(Key::from(("user", black_box(7u32)))))), 0);
    assert_eq!(allocations(|| drop(black_box(Key::Static("user")))), 0);
}

#[test]
fn a_text_key_allocates_which_is_what_the_numeric_keys_avoid() {
    assert!(allocations(|| drop(black_box(Key::from(black_box("user"))))) >= 1);
}

struct Leaf;

impl VisitorElement for Leaf {
    fn debug_name(&self) -> &'static str {
        "Leaf"
    }
}
impl Drawable for Leaf {
    fn update(&self, _ctx: &BuildContext) {}
}
impl LayoutElement for Leaf {}
impl EventElement for Leaf {}
impl Rebuildable for Leaf {}

fn context() -> (Rc<Venus>, BuildContext<'static>, tokio::runtime::Runtime) {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let venus = Venus::new();
    venus.install();
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    let ctx = BuildContext::new(
        canvas,
        ResolvedSize::default(),
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone(),
    );
    (venus, ctx, runtime)
}

/// Allocations of one steady-state rebuild of a widget that watches a running
/// future under the key `key()` builds, or that watches nothing for `None`.
fn rebuild_allocations(key: Option<Box<dyn Fn() -> Key>>) -> usize {
    let (venus, ctx, _runtime) = context();
    let element = StatelessElement::from_builder(
        &ctx,
        move |_| {
            if let Some(key) = &key {
                let _ = async { Ok::<u32, ()>(1) }.watch(key());
            }
            let leaf: AnyElement = Leaf.boxed();
            leaf
        },
        None,
        "Watching",
    );
    venus.run_microtasks();
    // Settle: the first rebuilds grow every buffer the steady state reuses.
    for _ in 0..3 {
        element.mark_needs_rebuild();
        element.rebuild_if_dirty(&ctx);
    }

    allocations(|| {
        element.mark_needs_rebuild();
        element.rebuild_if_dirty(&ctx);
    })
}

#[test]
fn a_rebuild_that_watches_with_a_tagged_key_allocates_less_than_with_a_text_key() {
    let text = rebuild_allocations(Some(Box::new(|| Key::from("user:7"))));
    let tagged = rebuild_allocations(Some(Box::new(|| Key::from(("user", 7u64)))));

    eprintln!("allocations per watching rebuild: text key {text}, tagged key {tagged}");
    assert!(
        tagged < text,
        "a tagged key must save the text key's allocation: tagged {tagged}, text {text}"
    );
}

/// The point of the exercise: once its future is running, `watch` with a tagged
/// key costs the build nothing on the heap.
#[test]
fn watching_a_running_future_with_a_tagged_key_adds_no_allocation_to_a_rebuild() {
    let without = rebuild_allocations(None);
    let tagged = rebuild_allocations(Some(Box::new(|| Key::from(("user", 7u64)))));
    let number = rebuild_allocations(Some(Box::new(|| Key::from(7u64))));

    eprintln!("allocations per rebuild: no watch {without}, tagged key {tagged}, number key {number}");
    assert_eq!(tagged, without);
    assert_eq!(number, without);
}
