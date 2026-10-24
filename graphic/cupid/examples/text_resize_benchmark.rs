//! Cost of `TextPipelineV2::prepare` while a text-heavy window is being
//! resized.
//!
//! Resizing is the text pipeline's worst case that is *not* about scrolling:
//! a wrapped layout is keyed by the width it wraps at, so every resize frame
//! misses the layout cache for the whole document. The visible part must be
//! re-laid out — the screen shows it at the new width — but every other cost
//! is avoidable: laying out the off-screen tail at a width the next frame
//! invalidates, and flooding the layout cache with transient-width entries
//! until it hits its capacity and is torn down wholesale. This benchmark
//! measures a sustained live resize: a document several times taller than the
//! viewport, fully prepared up front, then re-prepared with the surface width
//! changing a pixel per frame.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_resize_benchmark --release
//! ```

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aimer_cupid::AntiAlias;
use aimer_cupid::backend::wgpu::WgpuBackend;
use aimer_cupid::font::{FontFamily, FontStyle};
use aimer_cupid::text_layout::TextHorizontalAlign;
use aimer_cupid::text_pipeline::{TextDrawRequest, TextOverflowMode, TextPipelineV2};
use aimer_cupid::utilities::Rgba8;
use aimer_utils::SyncFuture;

const SURFACE_WIDTH: u32 = 1200;
const SURFACE_HEIGHT: u32 = 800;
/// How many times taller than the viewport the document is.
const OVERDRAW_FACTOR: usize = 4;
const LINE_HEIGHT: f32 = 24.0;
const FONT_SIZE: f32 = 16.0;
const MEASURED_FRAMES: usize = 240;

const SAMPLE_LINES: [&str; 4] = [
    "Aimer builds native user interfaces from a single declarative widget tree.",
    "Cupid batches rectangles, text and images into a single hardware pass;",
    "the glyph atlas keeps every rasterized bitmap so resizing re-rasterizes nothing,",
    "and the shaping cache is width-independent so resizing re-shapes nothing either.",
];

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("text resize benchmark device"),
            ..Default::default()
        })
        .block()
        .ok()
}

/// The document: one wrapped request per line, each wrapping at the surface
/// width the way a full-width column of text does.
fn document(line_count: usize, surface_width: u32) -> Vec<TextDrawRequest> {
    let clip = [0.0, 0.0, surface_width as f32, SURFACE_HEIGHT as f32];
    (0..line_count)
        .map(|index| TextDrawRequest {
            x: 16.0,
            y: index as f32 * LINE_HEIGHT,
            // Every line is unique, as a real document's lines are.
            text: Arc::from(
                format!("{index:>4} {}", SAMPLE_LINES[index % SAMPLE_LINES.len()]).as_str(),
            ),
            font_size: FONT_SIZE,
            color: Rgba8::from_unorm([0.1, 0.1, 0.1, 1.0]),
            bounds_width: surface_width as f32 - 32.0,
            bounds_height: LINE_HEIGHT,
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
        })
        .collect()
}

fn main() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device, queue);
    let mut pipeline = TextPipelineV2::new(
        &backend,
        wgpu::TextureFormat::Rgba8Unorm,
        AntiAlias::Analytic,
    );

    let line_count = SURFACE_HEIGHT as usize / LINE_HEIGHT as usize * OVERDRAW_FACTOR;

    // Warm up at the starting width until everything is prepared, so the
    // measured frames exercise only what the resize itself costs.
    let mut warm_frames = 0;
    loop {
        let requests = document(line_count, SURFACE_WIDTH);
        pipeline.prepare(
            &backend,
            SURFACE_WIDTH,
            SURFACE_HEIGHT,
            false,
            &requests,
            &[],
        );
        warm_frames += 1;
        if !pipeline.has_postponed_preparation() || warm_frames > 600 {
            break;
        }
    }

    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;
    for frame in 0..MEASURED_FRAMES {
        // A live drag: the width walks one pixel per frame, never repeating
        // the previous frame's value.
        let width = SURFACE_WIDTH - (frame as u32 % 120) - 1;
        let requests = document(line_count, width);

        let start = Instant::now();
        pipeline.prepare(
            &backend,
            width,
            SURFACE_HEIGHT,
            false,
            black_box(&requests),
            &[],
        );
        let elapsed = start.elapsed();
        total += elapsed;
        worst = worst.max(elapsed);
    }

    println!(
        "text resize benchmark: {line_count} lines ({OVERDRAW_FACTOR}x viewport), \
         {MEASURED_FRAMES} frames, warmed in {warm_frames} frames"
    );
    println!("prepare avg:   {:?}", total / MEASURED_FRAMES as u32);
    println!("prepare worst: {worst:?}");
}
