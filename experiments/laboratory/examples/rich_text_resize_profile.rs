//! Resize cost with rich text paragraphs.

use std::hint::black_box;
use aimer_time::Instant;

use aimer::quiver::winit::{dpi::PhysicalSize, event::WindowEvent};
use aimer::{AimerApp, AnyWidget, HeadlessOptions, RichText, TextSpan, Widget};

const FRAME_WIDTH: u32 = 800;
const FRAME_HEIGHT: u32 = 700;
const PARAGRAPHS: usize = 400;
const MEASURED: usize = 32;

fn paragraph(index: usize) -> AnyWidget {
    let body = format!(
        "Paragraph {index}: Rich text paragraphs exercise the shared text measurement path. \
         The same content is rebuilt while the available width changes, which exposes whether \
         font advances and layout measurements survive the rebuild."
    );
    RichText::new(TextSpan::new(body)).wrapped().boxed()
}

fn page() -> AnyWidget {
    aimer::Column::new()
        .children((0..PARAGRAPHS).map(paragraph))
        .boxed()
}

fn main() {
    let mut app = AimerApp::start_headless_with(
        page(),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    );

    for _ in 0..4 {
        app.render_frame();
    }

    aimer::frame_stats::reset_frame_content_stats();
    aimer::frame_stats::reset_frame_breakdown();

    let mut times = Vec::with_capacity(MEASURED);
    for step in 0..MEASURED {
        let width = if step % 2 == 0 {
            FRAME_WIDTH - 1
        } else {
            FRAME_WIDTH
        };
        let start = Instant::now();
        black_box(&mut app);
        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, FRAME_HEIGHT)));
        times.push(start.elapsed().as_secs_f64() * 1e6);
    }

    times.sort_by(f64::total_cmp);
    let p50 = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    let breakdown = aimer::frame_stats::frame_breakdown();
    let content = aimer::frame_stats::frame_content_stats();
    println!(
        "rich resize: p50={p50:.2}us p95={p95:.2}us build={:.2}ms encode={:.2}ms \
         cmds/frame={:.1} nodes/frame={:.1} text-cmds/frame={:.1} text-hit/frame={:.1} \
         text-miss/frame={:.1} paint/frame={:.1}",
        breakdown.build.average().as_secs_f64() * 1e3,
        breakdown.encode.average().as_secs_f64() * 1e3,
        content.average_draw_commands(),
        content.average_drawn_nodes(),
        content.average_text_commands(),
        content.text_cache_hits as f64 / content.frames as f64,
        content.average_text_cache_misses(),
        content.paint_calls as f64 / content.frames as f64,
    );
}
