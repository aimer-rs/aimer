//! Native Phase 3 acceptance measurements.
//!
//! This executable drives the real native headless event/frame path with three
//! representative page shapes: a static Home-like page, a static Blog-like
//! archive, and a Latest-post-like page with a stable prefix and dynamic
//! suffix. It reports the CPU-side build/paint work separately from GPU encode
//! and presentation, which are not available without a native surface.
//!
//! Run both profiles with frame instrumentation enabled:
//!
//! ```text
//! cargo run -p aimer_laboratory --example native_phase3_acceptance --features aimer/frame-stats
//! cargo run -p aimer_laboratory --example native_phase3_acceptance --release --features aimer/frame-stats
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::process::Command;
use std::rc::Rc;
use std::time::Instant;

use aimer::frame_stats::{FrameBreakdown, FrameContentStats, FrameRequestStats};
use aimer::modal::ModalHost;
use aimer::quiver::winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};
use aimer::{
    AimerApp, AnyElement, AnyWidget, BuildContext, Color, Column, Dimension, Drawable, Element,
    EventElement, HeadlessOptions, LayoutElement, Rebuildable, ResolvedSize, Size, Vec2d,
    VisitorElement, Widget,
};

const FRAME_WIDTH: u32 = 1_150;
const FRAME_HEIGHT: u32 = 800;
const ROWS: usize = 512;
const STATIC_PREFIX_ROWS: usize = 32;
const DYNAMIC_SUFFIX_ROWS: usize = 24;
const WARMUP_FRAMES: usize = 4;
const TRAVERSAL_STEPS: usize = 5;
const SCROLL_PIXELS_PER_STEP: f64 = 256.0;

/// Counts allocations made by the measured thread without changing the
/// allocator used by the framework.
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
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get().saturating_add(1)));
}

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

fn reset_allocations() {
    ALLOCATIONS.with(|count| count.set(0));
}

#[derive(Clone, Copy)]
struct StaticProbeWidget {
    index: usize,
}

impl Widget for StaticProbeWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        StaticProbeElement { index: self.index }.boxed()
    }
}

impl aimer::PortableWidget for StaticProbeWidget {}

struct StaticProbeElement {
    index: usize,
}

impl Drawable for StaticProbeElement {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let color = if self.index % 2 == 0 {
            Color::WHITE
        } else {
            Color::BLACK
        };
        probe_fill(ctx, color);
    }
}

/// Records the probe's 1000 x 32 band into the element's own retained list.
fn probe_fill(ctx: &BuildContext, color: Color) {
    let canvas = aimer::canvas::Canvas::of(ctx);
    canvas.fill_color_rect(
        Vec2d::ZERO,
        ResolvedSize {
            width: 1_000.0,
            height: 32.0,
        },
        color,
    );
    canvas.finish();
}

impl VisitorElement for StaticProbeElement {
    fn debug_name(&self) -> &'static str {
        "StaticProbeElement"
    }
}

impl EventElement for StaticProbeElement {}
impl LayoutElement for StaticProbeElement {
    fn size(&self) -> Option<Size> {
        Some(Size {
            width: Dimension::Px(1_000.0),
            height: Dimension::Px(32.0),
        })
    }
}
impl Rebuildable for StaticProbeElement {}

struct DynamicProbeWidget {
    index: usize,
    draws: Rc<Cell<usize>>,
}

impl Widget for DynamicProbeWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        DynamicProbeElement {
            index: self.index,
            draws: self.draws,
        }
        .boxed()
    }
}

impl aimer::PortableWidget for DynamicProbeWidget {}

struct DynamicProbeElement {
    index: usize,
    draws: Rc<Cell<usize>>,
}

impl Drawable for DynamicProbeElement {
    fn update(&self, _ctx: &BuildContext) {
        self.draws.set(self.draws.get().saturating_add(1));
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let color = if self.index % 2 == 0 {
            Color::BLACK
        } else {
            Color::WHITE
        };
        probe_fill(ctx, color);
    }

    /// A dynamic probe stands for content that changes on every frame.
    fn local_v2_paint_needs_recording(&self, _ctx: &BuildContext) -> bool {
        true
    }
}

impl VisitorElement for DynamicProbeElement {
    fn debug_name(&self) -> &'static str {
        "DynamicProbeElement"
    }
}

impl EventElement for DynamicProbeElement {}
impl LayoutElement for DynamicProbeElement {
    fn size(&self) -> Option<Size> {
        Some(Size {
            width: Dimension::Px(1_000.0),
            height: Dimension::Px(32.0),
        })
    }
}
impl Rebuildable for DynamicProbeElement {}

fn static_page(rows: usize, offset: usize) -> AnyWidget {
    Column::new()
        .children((0..rows).map(|index| StaticProbeWidget {
            index: index + offset,
        }))
        .boxed()
}

fn dynamic_page() -> AnyWidget {
    let draws = Rc::new(Cell::new(0));
    let mut children = Vec::with_capacity(STATIC_PREFIX_ROWS + DYNAMIC_SUFFIX_ROWS);
    children.extend((0..STATIC_PREFIX_ROWS).map(|index| StaticProbeWidget { index }.boxed()));
    children.extend((0..DYNAMIC_SUFFIX_ROWS).map(|index| {
        DynamicProbeWidget {
            index,
            draws: draws.clone(),
        }
        .boxed()
    }));
    Column::new().children(children).boxed()
}

fn scrollable_page(child: AnyWidget) -> AnyWidget {
    aimer::Scrollable::new()
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(child)
        .boxed()
}

fn headless(child: AnyWidget) -> aimer::HeadlessAimerApp<ModalHost<AnyWidget>> {
    AimerApp::start_headless_with(
        scrollable_page(child),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    )
}

fn scroll_event(direction: f64, phase: TouchPhase) -> WindowEvent {
    WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            0.0,
            direction * SCROLL_PIXELS_PER_STEP,
        )),
        phase,
    }
}

fn percentile(samples: &[f64], fraction: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let index = ((samples.len() - 1) as f64 * fraction).round() as usize;
    samples[index]
}

fn diff_u64(after: u64, before: u64) -> u64 {
    after.saturating_sub(before)
}

fn diff_requests(after: FrameRequestStats, before: FrameRequestStats) -> FrameRequestStats {
    FrameRequestStats {
        accepted: diff_u64(after.accepted, before.accepted),
        coalesced: diff_u64(after.coalesced, before.coalesced),
        display_ticks: diff_u64(after.display_ticks, before.display_ticks),
    }
}

fn resident_memory_kib() -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", pid.as_str()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .ok()
}

#[derive(Default)]
struct PassMeasurement {
    frame_times_us: Vec<f64>,
    allocations: usize,
    breakdown: FrameBreakdown,
    content: FrameContentStats,
    frames: usize,
    over_60hz: usize,
    over_120hz: usize,
    requests: FrameRequestStats,
    start_rss_kib: Option<u64>,
    peak_rss_kib: Option<u64>,
    end_rss_kib: Option<u64>,
    pending_after_limit: bool,
}

fn measure_workload(name: &str, build: impl Fn() -> AnyWidget) {
    let mut app = headless(build());
    for _ in 0..WARMUP_FRAMES {
        app.render_frame();
    }

    aimer::frame_stats::reset_frame_breakdown();
    aimer::frame_stats::reset_frame_content_stats();
    aimer::frame_stats::reset_frame_request_stats();
    let request_start = app.frame_request_stats();
    let mut active = PassMeasurement {
        start_rss_kib: resident_memory_kib(),
        ..Default::default()
    };
    reset_allocations();

    for (pass, direction) in [-1.0, 1.0, -1.0, 1.0].into_iter().enumerate() {
        for step in 0..TRAVERSAL_STEPS {
            let phase = if step == 0 {
                TouchPhase::Started
            } else if step + 1 == TRAVERSAL_STEPS {
                TouchPhase::Ended
            } else {
                TouchPhase::Moved
            };
            app.send_window_event(scroll_event(direction, phase));
            let start = Instant::now();
            black_box(&mut app);
            app.render_frame();
            let elapsed_us = start.elapsed().as_secs_f64() * 1e6;
            active.frame_times_us.push(elapsed_us);
            active.frames += 1;
            if elapsed_us > 16_666.667 {
                active.over_60hz += 1;
            }
            if elapsed_us > 8_333.333 {
                active.over_120hz += 1;
            }
        }
        if let Some(rss) = resident_memory_kib() {
            active.peak_rss_kib = Some(active.peak_rss_kib.unwrap_or(rss).max(rss));
        }
        println!(
            "  {name}: traversal {} direction={} frames={}",
            pass + 1,
            if direction < 0.0 { "down" } else { "up" },
            TRAVERSAL_STEPS
        );
    }

    active.allocations = allocations();
    active.breakdown = aimer::frame_stats::frame_breakdown();
    active.content = aimer::frame_stats::frame_content_stats();
    active.requests = diff_requests(app.frame_request_stats(), request_start);
    active.end_rss_kib = resident_memory_kib();

    aimer::frame_stats::reset_frame_breakdown();
    aimer::frame_stats::reset_frame_content_stats();
    reset_allocations();
    let mut settled = PassMeasurement {
        start_rss_kib: resident_memory_kib(),
        ..Default::default()
    };
    const SETTLE_LIMIT: usize = 64;
    while settled.frames < SETTLE_LIMIT && app.take_redraw_request() {
        let start = Instant::now();
        black_box(&mut app);
        app.render_frame();
        settled
            .frame_times_us
            .push(start.elapsed().as_secs_f64() * 1e6);
        settled.frames += 1;
    }
    settled.pending_after_limit = app.take_redraw_request();
    settled.allocations = allocations();
    settled.breakdown = aimer::frame_stats::frame_breakdown();
    settled.content = aimer::frame_stats::frame_content_stats();
    settled.end_rss_kib = resident_memory_kib();

    print_measurement(name, "active", &active);
    print_measurement(name, "settled", &settled);
}

fn print_measurement(name: &str, phase: &str, measurement: &PassMeasurement) {
    let frames = measurement.frames.max(1) as f64;
    let mut times = measurement.frame_times_us.clone();
    times.sort_by(f64::total_cmp);
    let build_ms = measurement.breakdown.build.average().as_secs_f64() * 1e3;
    let allocations_per_frame = measurement.allocations as f64 / frames;
    println!(
        "{name:<28} {phase:<7} frames={:<3} wall_us={:>8.2}/{:>8.2} build_ms={:>7.3} \
         cmds/frame={:>7.2} nodes/frame={:>7.2} retained/frame={:>5.2} \
         allocs/frame={:>6.2} over60={:>3} over120={:>3} \
         requests={}/{}/{} rss_kib={:?}->{:?} pending={}",
        measurement.frames,
        percentile(&times, 0.50),
        percentile(&times, 0.95),
        build_ms,
        measurement.content.average_draw_commands(),
        measurement.content.average_drawn_nodes(),
        measurement.content.average_retained_layers(),
        allocations_per_frame,
        measurement.over_60hz,
        measurement.over_120hz,
        measurement.requests.accepted,
        measurement.requests.coalesced,
        measurement.requests.display_ticks,
        measurement.start_rss_kib,
        measurement.end_rss_kib,
        measurement.pending_after_limit,
    );
    if phase == "active" {
        println!(
            "  {name:<26} GPU encode/present: unavailable in headless native driver; \
             retained layers/frame={:.2}, text misses/frame={:.2}, image uploads/frame={:.2}",
            measurement.content.average_retained_layers(),
            measurement.content.average_text_cache_misses(),
            measurement.content.average_image_uploads(),
        );
    }
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "native Phase 3 acceptance ({profile}, {} traversals, {} frames/workload)",
        4,
        4 * TRAVERSAL_STEPS
    );
    println!(
        "display budgets: 60Hz=16,666.667us, 120Hz=8,333.333us; \
         active times include event delivery + native headless build/paint"
    );

    measure_workload("Home / static", || static_page(ROWS, 0));
    measure_workload("Blog / static", || static_page(ROWS, 1_000));
    measure_workload("Latest / dynamic islands", dynamic_page);
}
