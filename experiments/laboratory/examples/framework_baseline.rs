//! Baseline measurements for framework-level frame work.
//!
//! This is deliberately an executable benchmark rather than a unit benchmark:
//! it drives the real headless frame loop, including widget materialization,
//! layout, reconciliation, drawing, and the in-memory canvas. It also sends
//! resize and pointer events through the production headless handlers. The
//! scenarios separate an eager tree from a declared, windowed list so a future
//! framework optimization can show whether it improves a complete operation
//! rather than only a helper function.
//!
//! Run the release baseline with:
//!
//! ```text
//! cargo run -p aimer_laboratory --example framework_baseline --release
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use aimer_time::Instant;

use aimer::animation::{
    Animated, AnimatedBuilder, AnimatedPaint, AnimationController, AnimationEffect, curves,
};
use aimer::events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer::events::pointer::{PointerButton, PointerInfo};
use aimer::input::button::Button;
use aimer::modal::ModalHost;
use aimer::quiver::winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};
use aimer::{
    AimerApp, Align, AnyElement, AnyWidget, BuildContext, Color, Column, Container, Drawable,
    Element, EventDispatcher, EventElement, Expanded, FocusBehavior, FocusNode, Focusable,
    HeadlessOptions, LayoutElement, OverflowBehavior, Positioned, Rebuildable, Scrollable,
    SizedBox, Stack, Vec2d, VisitorElement, Widget,
};

const ROUNDS: usize = 7;
const WARMUP_FRAMES: usize = 8;
const MEASURED_FRAMES: usize = 64;
const FRAME_WIDTH: u32 = 1_150;
const FRAME_HEIGHT: u32 = 800;
const EAGER_COUNTS: [usize; 3] = [32, 256, 2_048];
const WINDOWED_COUNT: usize = 120_000;

/// Counts allocations on the benchmark thread without changing the allocator
/// used by the framework.
struct RecordingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: Every operation forwards to the system allocator unchanged; the
// thread-local counter only observes allocation and reallocation calls.
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
    let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

fn reset_allocations() {
    ALLOCATIONS.with(|allocations| allocations.set(0));
}

/// A painted row with a small amount of variation so the eager table is not a
/// uniform-stride fast path.
fn row(index: usize) -> AnyWidget {
    let height = 20.0 + (index % 5) as f32;
    let color = if index % 2 == 0 {
        Color::WHITE
    } else {
        Color::BLACK
    };
    Container::new()
        .color(color)
        .box_child(SizedBox::new().width(800.0).height(height))
}

fn eager_column(count: usize) -> AnyWidget {
    Column::new().children((0..count).map(row)).boxed()
}

fn scrollable_column(count: usize) -> AnyWidget {
    Scrollable::new()
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(Column::new().children((0..count).map(row)))
        .boxed()
}

/// Mirrors the old website archive shape: a vertical scroll view whose eager
/// column is unnecessarily put into flex-wrap mode.
fn scrollable_wrapped_column(count: usize) -> AnyWidget {
    Scrollable::new()
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(
            Column::new()
                .overflow(OverflowBehavior::Wrap)
                .children((0..count).map(row)),
        )
        .boxed()
}

fn focusable_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|index| {
            Focusable::new()
                .behavior(FocusBehavior::OnPress)
                .child(row(index))
        }))
        .boxed()
}

fn stateful_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|index| Button::new().child(row(index))))
        .boxed()
}

fn animated_row(index: usize, value: f32) -> AnyWidget {
    let height = 20.0 + (index % 5) as f32 + value * 0.25;
    let color = if index % 2 == 0 {
        Color::WHITE
    } else {
        Color::BLACK
    };
    Container::new()
        .color(color)
        .box_child(SizedBox::new().width(800.0).height(height))
}

fn animated_column(count: usize) -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    AnimatedBuilder::new(controller, move |value| {
        Column::new()
            .children((0..count).map(|index| animated_row(index, value)))
            .boxed()
    })
    .boxed()
}

/// A minimal animation child that keeps application-built subtree work out of
/// the animation measurement. The controller, animation element, and one-child
/// reconciliation still run through the production frame path.
fn animated_probe() -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    AnimatedBuilder::new(controller, |_value| SizedBox::new().width(1.0).height(1.0)).boxed()
}

/// A retained opacity wrapper around one stable child. The controller is
/// sampled during synchronization, and the child element is never rebuilt.
fn retained_paint_probe() -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    AnimatedPaint::new(controller, SizedBox::new().width(1.0).height(1.0))
        .opacity()
        .boxed()
}

fn retained_paint_column(count: usize) -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    AnimatedPaint::new(controller, stable_column(count))
        .opacity()
        .boxed()
}

/// A fixed visual animation whose stable child can be retained while opacity
/// is updated in the compositor scene.
fn compositor_animation_probe() -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    Animated::new(
        controller,
        AnimationEffect::Opacity { from: 0.0, to: 1.0 },
        SizedBox::new().width(1.0).height(1.0),
    )
    .boxed()
}

fn compositor_animation_column(count: usize) -> AnyWidget {
    let controller = AnimationController::with_millis(1_000, curves::LINEAR);
    controller.forward_from_first_tick();
    Animated::new(
        controller,
        AnimationEffect::Opacity { from: 0.0, to: 1.0 },
        stable_column(count),
    )
    .boxed()
}

fn stable_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|_| SizedBox::new().width(800.0).height(20.0)))
        .boxed()
}

fn stable_column_with_animation(count: usize) -> AnyWidget {
    Stack::new()
        .children([stable_column(count), animated_probe()])
        .boxed()
}

/// A direct-child uniform column that exercises the compact stride table.
fn uniform_size_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|_| SizedBox::new().width(800.0).height(20.0)))
        .boxed()
}

/// A direct-child varying column that exercises the full offset table without
/// adding container, color, or application drawing work.
fn varying_size_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|index| {
            SizedBox::new()
                .width(800.0)
                .height(20.0 + (index % 5) as f32)
        }))
        .boxed()
}

/// Exercises the numeric flex-share distribution path with a repeatable mix
/// of regular and weighted children. The child values are erased so every
/// branch remains one homogeneous `Column` collection.
fn expanded_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|index| {
            if index % 4 == 0 {
                row(index)
            } else {
                Expanded::new()
                    .flex((index % 3 + 1) as f32)
                    .box_child(row(index))
            }
        }))
        .boxed()
}

fn layered_column(count: usize) -> AnyWidget {
    Column::new()
        .children((0..count).map(|index| {
            Align::new()
                .layer((count - index) as u32)
                .box_child(row(index))
        }))
        .boxed()
}

fn layered_stack(count: usize) -> AnyWidget {
    Stack::new()
        .children((0..count).map(|index| {
            Align::new()
                .layer((count - index) as u32)
                .box_child(row(index))
        }))
        .boxed()
}

/// A sparse layered stack whose positioned children occupy disjoint vertical
/// regions. It isolates the value of position-aware candidate filtering from
/// the intentionally full-area `layered_stack` workload above.
fn sparse_layered_stack(count: usize) -> AnyWidget {
    Stack::new()
        .children((0..count).map(|index| {
            let height = 20.0 + (index % 5) as f32;
            let color = if index % 2 == 0 {
                Color::WHITE
            } else {
                Color::BLACK
            };
            Positioned::new()
                .top(index as f32 * 24.0)
                .layer((count - index) as u32)
                .box_child(
                    Container::new()
                        .width(800.0)
                        .height(height)
                        .color(color)
                        .box_child(SizedBox::new().width(800.0).height(height)),
                )
        }))
        .boxed()
}

/// A no-op retained tree used to isolate framework traversal from widget and
/// canvas work. Leaves optionally expose a focus node; branches only retain
/// children so the dispatcher walks the same structural seams as production.
struct TraversalElement {
    children: Vec<AnyElement>,
    node: Option<FocusNode>,
}

impl Drawable for TraversalElement {
    fn update(&self, _ctx: &BuildContext) {}
}

impl VisitorElement for TraversalElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "TraversalElement"
    }
}

impl EventElement for TraversalElement {
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    #[inline]
    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in self.children.iter().rev() {
            visitor(child.as_ref());
        }
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.node.as_ref()
    }
}

impl LayoutElement for TraversalElement {}
impl Rebuildable for TraversalElement {}

fn traversal_tree(count: usize, focusable: bool) -> (AnyElement, Option<FocusNode>) {
    let mut first_node = None;
    let children = (0..count)
        .map(|index| {
            let node = focusable.then(FocusNode::new);
            if index == 0 {
                first_node = node.clone();
            }
            TraversalElement {
                children: Vec::new(),
                node,
            }
            .boxed()
        })
        .collect();

    (
        TraversalElement {
            children,
            node: None,
        }
        .boxed(),
        first_node,
    )
}

fn windowed_list() -> AnyWidget {
    Scrollable::new()
        .child(
            Column::new()
                .list(0..WINDOWED_COUNT)
                .item_extent(32.0)
                .builder(|index: &usize| row(*index)),
        )
        .boxed()
}

#[derive(Default)]
struct Measurements {
    samples: Vec<f64>,
    allocations_per_frame: f64,
}

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    let index = ((samples.len() - 1) as f64 * fraction).round() as usize;
    samples[index]
}

fn print_measurement(name: &str, samples: &mut [f64], allocations_per_frame: f64) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{name:<42} p50 {:>9.2} us  p95 {:>9.2} us  allocs/op {:>7.2}",
        percentile(samples, 0.50),
        percentile(samples, 0.95),
        allocations_per_frame,
    );
}

fn start_headless(
    build: impl FnOnce() -> AnyWidget,
) -> aimer::HeadlessAimerApp<ModalHost<AnyWidget>> {
    AimerApp::start_headless_with(
        build(),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    )
}

fn measure_cold_frames(name: &str, build: impl Fn() -> AnyWidget) {
    let mut samples = Vec::with_capacity(ROUNDS);
    let mut allocations_per_frame = 0.0;

    for _ in 0..ROUNDS {
        let mut app = start_headless(&build);
        reset_allocations();
        let start = Instant::now();
        black_box(&mut app);
        app.render_frame();
        samples.push(start.elapsed().as_secs_f64() * 1e6);
        allocations_per_frame += allocations() as f64;
    }

    allocations_per_frame /= ROUNDS as f64;
    print_measurement(
        &format!("{name} (cold frame)"),
        &mut samples,
        allocations_per_frame,
    );
}

fn measure_cached_frames(name: &str, build: impl Fn() -> AnyWidget) {
    let mut measurements = Measurements::default();

    for _ in 0..ROUNDS {
        let mut app = start_headless(&build);
        app.render_frame();
        for _ in 0..WARMUP_FRAMES {
            app.render_frame();
        }

        reset_allocations();
        let start = Instant::now();
        for _ in 0..MEASURED_FRAMES {
            black_box(&mut app);
            app.render_frame();
        }
        measurements
            .samples
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_FRAMES as f64);
        measurements.allocations_per_frame += allocations() as f64 / MEASURED_FRAMES as f64;
    }

    measurements.allocations_per_frame /= ROUNDS as f64;
    print_measurement(
        &format!("{name} (cached frame)"),
        &mut measurements.samples,
        measurements.allocations_per_frame,
    );
}

fn cursor_move(index: usize, device_id: DeviceId) -> WindowEvent {
    WindowEvent::CursorMoved {
        device_id,
        position: PhysicalPosition::new(400.0, 1.0 + (index % 30) as f64 * 20.0),
    }
}

fn resize(index: usize) -> WindowEvent {
    let width = if index % 2 == 0 {
        FRAME_WIDTH - 1
    } else {
        FRAME_WIDTH
    };
    WindowEvent::Resized(PhysicalSize::new(width, FRAME_HEIGHT))
}

fn scroll_event(device_id: DeviceId) -> WindowEvent {
    WindowEvent::MouseWheel {
        device_id,
        delta: MouseScrollDelta::LineDelta(0.0, -2.0),
        phase: TouchPhase::Moved,
    }
}

fn measure_pointer_moves(name: &str, build: impl Fn() -> AnyWidget) {
    let mut samples = Vec::with_capacity(ROUNDS);
    let mut allocations_per_event = 0.0;
    let device_id = DeviceId::dummy();

    for _ in 0..ROUNDS {
        let mut app = start_headless(&build);
        app.render_frame();
        for index in 0..WARMUP_FRAMES {
            app.send_window_event(cursor_move(index, device_id));
        }

        reset_allocations();
        let start = Instant::now();
        for index in 0..MEASURED_FRAMES {
            black_box(&mut app);
            app.send_window_event(cursor_move(index + WARMUP_FRAMES, device_id));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_FRAMES as f64);
        allocations_per_event += allocations() as f64 / MEASURED_FRAMES as f64;
    }

    allocations_per_event /= ROUNDS as f64;
    print_measurement(
        &format!("{name} (pointer move)"),
        &mut samples,
        allocations_per_event,
    );
}

fn measure_resizes(name: &str, build: impl Fn() -> AnyWidget) {
    let mut samples = Vec::with_capacity(ROUNDS);
    let mut allocations_per_resize = 0.0;

    for _ in 0..ROUNDS {
        let mut app = start_headless(&build);
        app.render_frame();
        for index in 0..WARMUP_FRAMES {
            app.send_window_event(resize(index));
        }

        reset_allocations();
        let start = Instant::now();
        for index in 0..MEASURED_FRAMES {
            black_box(&mut app);
            app.send_window_event(resize(index + WARMUP_FRAMES));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_FRAMES as f64);
        allocations_per_resize += allocations() as f64 / MEASURED_FRAMES as f64;
    }

    allocations_per_resize /= ROUNDS as f64;
    print_measurement(
        &format!("{name} (resize)"),
        &mut samples,
        allocations_per_resize,
    );
}

/// Measures a wheel event together with the frame it schedules. The headless
/// event path does not render automatically for wheel input, so rendering here
/// makes this a complete scroll operation rather than only a dispatch sample.
fn measure_scrolls(name: &str, build: impl Fn() -> AnyWidget) {
    let mut samples = Vec::with_capacity(ROUNDS);
    let mut allocations_per_scroll = 0.0;
    let device_id = DeviceId::dummy();

    for _ in 0..ROUNDS {
        let mut app = start_headless(&build);
        app.render_frame();
        app.send_window_event(cursor_move(0, device_id));
        for _ in 0..WARMUP_FRAMES {
            app.send_window_event(scroll_event(device_id));
            app.render_frame();
        }

        reset_allocations();
        let start = Instant::now();
        for _ in 0..MEASURED_FRAMES {
            black_box(&mut app);
            app.send_window_event(scroll_event(device_id));
            app.render_frame();
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_FRAMES as f64);
        allocations_per_scroll += allocations() as f64 / MEASURED_FRAMES as f64;
    }

    allocations_per_scroll /= ROUNDS as f64;
    print_measurement(
        &format!("{name} (scroll + frame)"),
        &mut samples,
        allocations_per_scroll,
    );
}

fn measure_framework_dispatch(name: &str, count: usize, focusable: bool) {
    let event = if focusable {
        ElementEvent::KeyInput {
            key: NamedKey::Tab,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        }
    } else {
        ElementEvent::PointerMove(PointerInfo::mouse(Vec2d::default(), PointerButton::Primary))
    };

    let mut samples = Vec::with_capacity(ROUNDS);
    let mut allocations_per_operation = 0.0;
    for _ in 0..ROUNDS {
        let (root, first_node) = traversal_tree(count, focusable);
        let mut dispatcher = EventDispatcher::new();

        if let Some(node) = first_node {
            node.request_focus();
            let _ = dispatcher.dispatch(root.as_ref(), Vec2d::default(), &ElementEvent::Cancel);
        }
        for _ in 0..WARMUP_FRAMES {
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }

        reset_allocations();
        let start = Instant::now();
        for _ in 0..MEASURED_FRAMES {
            black_box(&mut dispatcher);
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_FRAMES as f64);
        allocations_per_operation += allocations() as f64 / MEASURED_FRAMES as f64;
    }
    allocations_per_operation /= ROUNDS as f64;
    print_measurement(
        &format!("{name}: {count} nodes"),
        &mut samples,
        allocations_per_operation,
    );
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "framework baseline ({profile} profile, {ROUNDS} rounds, \
         {MEASURED_FRAMES} measured operations)"
    );
    println!("scenario                                   p50        p95  allocs/op");

    for count in EAGER_COUNTS {
        let name = format!("eager column: {count} rows");
        measure_cold_frames(&name, move || eager_column(count));
        measure_cached_frames(&name, move || eager_column(count));
    }
    for count in EAGER_COUNTS {
        let name = format!("layered column: {count} rows");
        measure_cold_frames(&name, move || layered_column(count));
        measure_cached_frames(&name, move || layered_column(count));
    }
    for count in [256, 2_048] {
        let name = format!("layered stack: {count} rows");
        measure_cold_frames(&name, move || layered_stack(count));
        measure_cached_frames(&name, move || layered_stack(count));
    }
    let windowed_name = format!("windowed declared list: {WINDOWED_COUNT} rows");
    measure_cold_frames(&windowed_name, windowed_list);
    measure_cached_frames(&windowed_name, windowed_list);

    println!("\nresize workloads");
    for count in [256, 2_048] {
        measure_resizes(&format!("eager column: {count} rows"), move || {
            eager_column(count)
        });
        measure_resizes(&format!("layered column: {count} rows"), move || {
            layered_column(count)
        });
        measure_resizes(&format!("layered stack: {count} rows"), move || {
            layered_stack(count)
        });
    }
    measure_resizes(&windowed_name, windowed_list);

    println!("\nflex size-table workloads");
    for count in [256, 2_048] {
        measure_cold_frames(&format!("uniform size column: {count} rows"), move || {
            uniform_size_column(count)
        });
        measure_cold_frames(&format!("varying size column: {count} rows"), move || {
            varying_size_column(count)
        });
        measure_resizes(&format!("uniform size column: {count} rows"), move || {
            uniform_size_column(count)
        });
        measure_resizes(&format!("varying size column: {count} rows"), move || {
            varying_size_column(count)
        });
    }

    println!("\nflex distribution workloads");
    for count in [256, 2_048] {
        measure_cold_frames(&format!("expanded column: {count} rows"), move || {
            expanded_column(count)
        });
        measure_resizes(&format!("expanded column: {count} rows"), move || {
            expanded_column(count)
        });
    }

    println!("\npointer hit-test workloads");
    for count in EAGER_COUNTS {
        measure_pointer_moves(&format!("eager column: {count} rows"), move || {
            eager_column(count)
        });
        measure_pointer_moves(&format!("layered column: {count} rows"), move || {
            layered_column(count)
        });
        measure_pointer_moves(&format!("layered stack: {count} rows"), move || {
            layered_stack(count)
        });
        if count != 32 {
            measure_pointer_moves(&format!("sparse layered stack: {count} rows"), move || {
                sparse_layered_stack(count)
            });
        }
    }
    measure_pointer_moves(&windowed_name, windowed_list);

    println!("\nscroll workloads");
    for count in [256, 2_048] {
        measure_scrolls(&format!("scrollable column: {count} rows"), move || {
            scrollable_column(count)
        });
    }
    println!("\nwrapped-column regression");
    for count in [256, 2_048] {
        measure_scrolls(
            &format!("scrollable wrapped column: {count} rows"),
            move || scrollable_wrapped_column(count),
        );
    }
    measure_scrolls(&windowed_name, windowed_list);

    println!("\nframework-only traversal workloads");
    for count in EAGER_COUNTS {
        measure_framework_dispatch("event dispatch", count, false);
        measure_framework_dispatch("focus traversal", count, true);
    }

    println!("\nstateful and focusable workloads");
    for count in EAGER_COUNTS {
        measure_cached_frames(&format!("focusable column: {count} controls"), move || {
            focusable_column(count)
        });
        measure_cached_frames(&format!("stateful column: {count} buttons"), move || {
            stateful_column(count)
        });
    }

    println!("\nanimation workloads");
    measure_cached_frames("animation framework shell", animated_probe);
    measure_cached_frames("retained paint framework shell", retained_paint_probe);
    measure_cached_frames(
        "compositor animation framework shell",
        compositor_animation_probe,
    );
    for count in [256, 2_048] {
        measure_cached_frames(&format!("animated column: {count} rows"), move || {
            animated_column(count)
        });
        measure_cached_frames(
            &format!("stable column + animation: {count} rows"),
            move || stable_column_with_animation(count),
        );
        measure_cached_frames(&format!("retained paint column: {count} rows"), move || {
            retained_paint_column(count)
        });
        measure_cached_frames(
            &format!("compositor animation column: {count} rows"),
            move || compositor_animation_column(count),
        );
    }
}
