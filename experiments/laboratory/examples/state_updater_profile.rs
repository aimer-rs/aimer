//! Release measurements for the copyable `StateUpdater` path.
//!
//! This profile keeps the benchmark close to the production API. It measures
//! copyable-handle movement, stateful construction, queueing, and a
//! callback-heavy rebuild through `StatefulElement`. The frame-request probe
//! is installed through the same thread-local requester used by the native
//! event loop, so it also checks the existing dirty/request coalescing rule.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p aimer_laboratory --example state_updater_profile --release
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;
use std::time::Instant;

use aimer::input::button::Button;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    AnyElement, BuildContext, Column, ResolvedSize, SizedBox, State, StateUpdater, StatefulElement,
    StatefulWidget, Vec2d, Widget,
};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_events::window::{restore_thread_redraw_requester, set_thread_redraw_requester};
use aimer_widget::base::WindowHandle;

const ROUNDS: usize = 7;
const WARMUP_BATCHES: usize = 3;
const MEASURED_BATCHES: usize = 8;
const UPDATES_PER_BATCH: usize = 256;
const CONSTRUCTIONS_PER_ROUND: usize = 16;
const CALLBACK_COUNT: usize = 32;
const COPY_ITERATIONS: usize = 1_000_000;
const FRAME_WIDTH: f32 = 1_150.0;
const FRAME_HEIGHT: f32 = 800.0;

/// Counts allocation and reallocation calls on the benchmark thread without
/// changing the allocator used by the framework.
struct RecordingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Every operation forwards to the system allocator unchanged; the
// thread-local counter only observes allocation and reallocation calls.
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

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

fn reset_allocations() {
    ALLOCATIONS.with(|allocations| allocations.set(0));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("state updater profile runtime")
}

fn context(runtime: &tokio::runtime::Runtime) -> BuildContext<'static> {
    let inner = Box::leak(Box::new(InnerCanvas::new()));
    let mut context = BuildContext::new(
        FrameCanvas::new(inner),
        ResolvedSize {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(
            PhysicalSize::new(FRAME_WIDTH as u32, FRAME_HEIGHT as u32),
            1.0,
        ),
        runtime.handle().clone(),
    );
    context.box_constraint.max_width = FRAME_WIDTH;
    context.box_constraint.max_height = FRAME_HEIGHT;
    context
}

struct ProfileWidget {
    callback_count: usize,
}

struct ProfileState {
    value: usize,
    callback_count: usize,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for ProfileWidget {
    type State = ProfileState;

    fn create_state(self) -> Self::State {
        ProfileState {
            value: 0,
            callback_count: self.callback_count,
            updater: StateUpdater::new(),
        }
    }
}

impl aimer::PortableWidget for ProfileWidget {}

impl Widget for ProfileWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "StateUpdaterProfile", None)
            .0
            .boxed()
    }
}

impl State<ProfileWidget> for ProfileState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater;
        Column::new().children((0..self.callback_count).map(move |index| {
            let updater = updater;
            Button::new()
                .on_press(move || {
                    updater.set_state(move |state| {
                        state.value = state.value.wrapping_add(index + 1);
                    });
                })
                .child(SizedBox::new().width(1.0).height(1.0))
        }))
    }
}

fn make_element(
    callback_count: usize,
    runtime: &tokio::runtime::Runtime,
) -> (
    BuildContext<'static>,
    StatefulElement,
    StateUpdater<ProfileState>,
) {
    let context = context(runtime);
    let (element, updater) = StatefulElement::new_with_name(
        ProfileWidget { callback_count },
        &context,
        "StateUpdaterProfile",
        None,
    );
    (context, element, updater)
}

#[derive(Default)]
struct Samples {
    microseconds_per_operation: Vec<f64>,
    allocations_per_operation: Vec<f64>,
    frame_requests_per_batch: Vec<f64>,
}

impl Samples {
    fn new() -> Self {
        Self {
            microseconds_per_operation: Vec::with_capacity(ROUNDS),
            allocations_per_operation: Vec::with_capacity(ROUNDS),
            frame_requests_per_batch: Vec::with_capacity(ROUNDS),
        }
    }

    fn print(&mut self, name: &str, show_frame_requests: bool) {
        self.microseconds_per_operation.sort_by(f64::total_cmp);
        self.allocations_per_operation.sort_by(f64::total_cmp);
        self.frame_requests_per_batch.sort_by(f64::total_cmp);
        let p50 = percentile(&self.microseconds_per_operation, 0.50);
        let p95 = percentile(&self.microseconds_per_operation, 0.95);
        let allocations = percentile(&self.allocations_per_operation, 0.50);
        if show_frame_requests {
            let requests = percentile(&self.frame_requests_per_batch, 0.50);
            println!(
                "{name:<43} p50 {:>9.2} us/op  p95 {:>9.2} us/op  allocs/op {:>7.2}  frame requests/batch {:>5.2}",
                p50, p95, allocations, requests
            );
        } else {
            println!(
                "{name:<43} p50 {:>9.2} us/op  p95 {:>9.2} us/op  allocs/op {:>7.2}",
                p50, p95, allocations
            );
        }
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

fn measure_handle_copy(runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::new();
    let (_, element, updater) = make_element(0, runtime);
    for _ in 0..ROUNDS {
        let mut current = updater;
        let start = Instant::now();
        for _ in 0..COPY_ITERATIONS {
            current = black_box(current);
        }
        black_box(current);
        samples
            .microseconds_per_operation
            .push(start.elapsed().as_secs_f64() * 1e6 / COPY_ITERATIONS as f64);
        samples.allocations_per_operation.push(0.0);
    }
    black_box(element);
    samples.microseconds_per_operation.sort_by(f64::total_cmp);
    let p50 = percentile(&samples.microseconds_per_operation, 0.50) * 1_000.0;
    let p95 = percentile(&samples.microseconds_per_operation, 0.95) * 1_000.0;
    println!(
        "{:<43} p50 {:>9.2} ns/op  p95 {:>9.2} ns/op  allocs/op {:>7.2}",
        "StateUpdater copy (one million moves)", p50, p95, 0.0
    );
}

fn measure_construction(runtime: &tokio::runtime::Runtime) {
    // Prime the thread-local arena and allocator paths so this reports the
    // steady-state construction cost paid by repeated rebuilds.
    let (_, element, _) = make_element(CALLBACK_COUNT, runtime);
    drop(element);

    let mut samples = Samples::new();
    for _ in 0..ROUNDS {
        reset_allocations();
        let context = context(runtime);
        let start = Instant::now();
        for _ in 0..CONSTRUCTIONS_PER_ROUND {
            let (element, updater) = StatefulElement::new_with_name(
                ProfileWidget {
                    callback_count: CALLBACK_COUNT,
                },
                &context,
                "StateUpdaterProfile",
                None,
            );
            black_box(updater.has_state());
            black_box(element);
        }
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        samples
            .microseconds_per_operation
            .push(elapsed / CONSTRUCTIONS_PER_ROUND as f64);
        samples
            .allocations_per_operation
            .push(allocations() as f64 / CONSTRUCTIONS_PER_ROUND as f64);
    }
    samples.print("StatefulElement construction (32 callbacks)", false);
}

fn queue_updates(updater: StateUpdater<ProfileState>, count: usize) {
    for _ in 0..count {
        updater.set_state(|state| state.value = state.value.wrapping_add(1));
    }
}

fn measure_queue_and_rebuild(callback_count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::new();
    for _ in 0..ROUNDS {
        let (context, element, updater) = make_element(callback_count, runtime);
        for _ in 0..WARMUP_BATCHES {
            queue_updates(updater, UPDATES_PER_BATCH);
            element.rebuild_if_dirty(&context);
        }

        let requests = Rc::new(Cell::new(0usize));
        let requests_for_callback = requests.clone();
        let previous = set_thread_redraw_requester(move || {
            requests_for_callback.set(requests_for_callback.get() + 1);
        });
        reset_allocations();
        let start = Instant::now();
        for _ in 0..MEASURED_BATCHES {
            queue_updates(updater, UPDATES_PER_BATCH);
            element.rebuild_if_dirty(&context);
        }
        let elapsed = start.elapsed().as_secs_f64() * 1e6;
        restore_thread_redraw_requester(previous);

        let operations = (MEASURED_BATCHES * UPDATES_PER_BATCH) as f64;
        samples
            .microseconds_per_operation
            .push(elapsed / operations);
        samples
            .allocations_per_operation
            .push(allocations() as f64 / operations);
        samples
            .frame_requests_per_batch
            .push(requests.get() as f64 / MEASURED_BATCHES as f64);
        black_box(element);
    }
    let label = if callback_count == 0 {
        "queue + rebuild (empty callback tree)"
    } else {
        "queue + rebuild (32 callback tree)"
    };
    samples.print(label, true);
}

fn main() {
    println!(
        "StateUpdater size: {} bytes | rounds: {ROUNDS} | release profile",
        std::mem::size_of::<StateUpdater<ProfileState>>()
    );
    let runtime = runtime();
    measure_handle_copy(&runtime);
    measure_construction(&runtime);
    measure_queue_and_rebuild(0, &runtime);
    measure_queue_and_rebuild(CALLBACK_COUNT, &runtime);
}
