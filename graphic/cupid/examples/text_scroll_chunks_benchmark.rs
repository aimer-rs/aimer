//! Steady-state cost of `TextPipelineV2::prepare` for one long wrapped text, drawn as a single
//! request and as requests of a few hard lines each.
//!
//! A text view that shows a window of a long note hands the pipeline that window as text. As one
//! request, every glyph of it takes part in the per-frame work whether it is on screen or not,
//! since the request as a whole is on screen. Cut into requests of a few lines, the ones off
//! screen are set aside by their boxes before a glyph is looked at. The text, its wrapping and
//! its position are the same in both; only how it is cut differs.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_scroll_chunks_benchmark
//! ```

use std::hint::black_box;
use std::sync::Arc;
use aimer_time::{Duration, Instant};

use aimer_cupid::AntiAlias;
use aimer_cupid::backend::wgpu::WgpuBackend;
use aimer_cupid::font::{FontFamily, FontStyle};
use aimer_cupid::text_layout::TextHorizontalAlign;
use aimer_cupid::text_pipeline::{TextDrawRequest, TextOverflowMode, TextPipelineV2};
use aimer_cupid::utilities::Rgba8;
use aimer_utils::SyncFuture;

const SURFACE_WIDTH: u32 = 1200;
const SURFACE_HEIGHT: u32 = 800;
const LINE_HEIGHT: f32 = 19.0;
const FONT_SIZE: f32 = 16.0;
const SCROLL_STEP: f32 = 12.0;
const MEASURED_FRAMES: usize = 240;
/// Hard lines in the text: about three viewports of it, as a window with a margin of one
/// viewport above and below holds.
const LINES: usize = 3 * SURFACE_HEIGHT as usize / LINE_HEIGHT as usize;

const SAMPLE: &str = "A simple note app built with Aimer. Start typing to edit this note, use the toolbar to create, pin or delete notes.";

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions::default()).block().ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor { label: Some("text scroll chunks device"), ..Default::default() })
        .block()
        .ok()
}

/// The text of `LINES` hard lines, each one visual line wide (so the heights are known).
fn lines() -> Vec<String> {
    (0..LINES).map(|index| format!("{index:>4} {SAMPLE}")).collect()
}

/// The text cut into requests of `per_request` hard lines (the last one may have fewer), the
/// whole window moved up by `scroll_offset`.
fn requests(lines: &[String], per_request: usize, scroll_offset: f32) -> Vec<TextDrawRequest> {
    let clip = [0.0, 0.0, SURFACE_WIDTH as f32, SURFACE_HEIGHT as f32];
    lines
        .chunks(per_request)
        .enumerate()
        .map(|(index, chunk)| TextDrawRequest {
            x: 16.0,
            y: (index * per_request) as f32 * LINE_HEIGHT - scroll_offset,
            text: Arc::from(chunk.join("\n").as_str()),
            font_size: FONT_SIZE,
            color: Rgba8::from_unorm([0.1, 0.1, 0.1, 1.0]),
            bounds_width: SURFACE_WIDTH as f32 - 32.0,
            bounds_height: chunk.len() as f32 * LINE_HEIGHT,
            overflow: TextOverflowMode::Wrap,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: aimer_cupid::text_layout::TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: clip,
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
            alpha_ramp: None,
        })
        .collect()
}

fn main() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let lines = lines();
    println!("{LINES} hard lines, {} bytes, scrolled {SCROLL_STEP} px a frame for {MEASURED_FRAMES} frames", lines.iter().map(String::len).sum::<usize>());
    // Each shape in turn, on a pipeline of its own so that nothing one learned helps the next.
    for per_request in [LINES, 64, 32, 24, 16, 12, 8, 4, 1] {
        let backend = WgpuBackend::new(device.clone(), queue.clone());
        let mut pipeline = TextPipelineV2::new(&backend, wgpu::TextureFormat::Rgba8Unorm, AntiAlias::Analytic);
        let mut warm = 0;
        loop {
            let batch = requests(&lines, per_request, 0.0);
            pipeline.prepare(&backend, SURFACE_WIDTH, SURFACE_HEIGHT, false, &batch, &[]);
            warm += 1;
            if !pipeline.has_postponed_preparation() || warm > 600 {
                break;
            }
        }
        let mut total = Duration::ZERO;
        let mut worst = Duration::ZERO;
        let max_offset = (LINES as f32 * LINE_HEIGHT - SURFACE_HEIGHT as f32).max(1.0);
        for frame in 0..MEASURED_FRAMES {
            let batch = requests(&lines, per_request, (frame as f32 * SCROLL_STEP) % max_offset);
            let start = Instant::now();
            pipeline.prepare(&backend, SURFACE_WIDTH, SURFACE_HEIGHT, false, black_box(&batch), &[]);
            let elapsed = start.elapsed();
            total += elapsed;
            worst = worst.max(elapsed);
        }
        println!(
            "{:>4} lines a request ({:>3} requests): prepare avg {:>10.2?}  worst {:>10.2?}",
            per_request.min(LINES),
            LINES.div_ceil(per_request),
            total / MEASURED_FRAMES as u32,
            worst,
        );
    }
}
