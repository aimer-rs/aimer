//! Scroll cost with real text content (the shape real apps scroll).
//!
//! ```text
//! cargo run -p aimer_laboratory --example scroll_text_profile --release --features aimer/frame-stats
//! ```

use std::hint::black_box;
use std::time::Instant;

use aimer::quiver::winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent},
};
use aimer::style::LayoutSpacing;
use aimer::{
    AimerApp, AnyWidget, Color, Column, Container, HeadlessOptions, ScrollController, Scrollable,
    Text, Widget,
};

const FRAME_WIDTH: u32 = 800;
const FRAME_HEIGHT: u32 = 700;
const PARAGRAPHS: usize = 400;
const MEASURED: usize = 64;

/// `SCROLL_FRAMES=<n>` lengthens the run so an external sampler can attach.
/// `SCROLL_PARAGRAPHS=<n>` changes the content size while the viewport stays fixed.
fn paragraphs() -> usize {
    std::env::var("SCROLL_PARAGRAPHS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(PARAGRAPHS)
}

fn measured_frames() -> usize {
    std::env::var("SCROLL_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(MEASURED)
}

fn paragraph(index: usize) -> AnyWidget {
    let body = format!(
        "Paragraph {index}: The compositor keeps this decorated scrolling row on the live \
         culled path because its container owns clipping and hit-test bounds. Text shaping is \
         cached per width, while stable paint islands elsewhere can be retained automatically \
         or promoted explicitly with RepaintBoundary."
    );
    Container::new()
        .color(if index % 2 == 0 {
            Color::WHITE
        } else {
            Color::Rgb(242, 242, 247)
        })
        .padding(LayoutSpacing::all(12))
        .box_child(Text::new(body).wrapped())
}

fn page(controller: ScrollController) -> AnyWidget {
    Scrollable::new()
        .controller(controller)
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(Column::new().children((0..paragraphs()).map(paragraph)))
        .boxed()
}

fn main() {
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        page(controller.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    );

    for _ in 0..4 {
        app.render_frame();
    }

    // Wheel input is routed by cursor position, so park the cursor over the list.
    let device_id = DeviceId::dummy();
    app.send_window_event(WindowEvent::CursorMoved {
        device_id,
        position: PhysicalPosition::new(400.0, 300.0),
    });
    app.render_frame();

    aimer::frame_stats::reset_frame_content_stats();
    aimer::frame_stats::reset_frame_breakdown();

    let mut times = Vec::with_capacity(measured_frames());
    let start_offset = controller.offset().y;
    for step in 0..measured_frames() {
        let phase = if step == 0 {
            TouchPhase::Started
        } else {
            TouchPhase::Moved
        };
        // Reverse every 150 frames so a long run keeps moving instead of
        // parking at the end of the content.
        let direction = if (step / 150) % 2 == 0 { -48.0 } else { 48.0 };
        app.send_window_event(WindowEvent::MouseWheel {
            device_id,
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, direction)),
            phase,
        });
        let start = Instant::now();
        black_box(&mut app);
        app.render_frame();
        times.push(start.elapsed().as_secs_f64() * 1e6);
    }
    let travelled = controller.offset().y - start_offset;

    times.sort_by(f64::total_cmp);
    let p50 = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    let content = aimer::frame_stats::frame_content_stats();
    println!(
        "text scroll: p50={p50:.2}us p95={p95:.2}us  travelled={travelled:.0}px \
         cmds/frame={:.1} nodes/frame={:.1} retained/frame={:.1} text-cmds/frame={:.1} \
         text-miss/frame={:.1}",
        content.average_draw_commands(),
        content.average_drawn_nodes(),
        content.average_retained_layers(),
        content.average_text_commands(),
        content.average_text_cache_misses(),
    );
}
