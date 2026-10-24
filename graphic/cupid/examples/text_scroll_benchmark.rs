//! Steady-state cost of `TextPipelineV2::prepare` while a text-heavy screen
//! scrolls.
//!
//! Scrolling is the text pipeline's worst case that is *not* about shaping:
//! every layout is already cached, yet every request moves a few pixels per
//! frame, so the per-frame instance rebuild — cache-key lookups, atlas
//! lookups, glyph loops — runs over the whole document every frame. This
//! benchmark measures exactly that path: a document several times taller than
//! the viewport, fully prepared up front, then scrolled one step per frame.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_scroll_benchmark --release
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
/// How many times taller than the viewport the document is — the ratio a
/// long article or chat log easily reaches.
const OVERDRAW_FACTOR: usize = 4;
const LINE_HEIGHT: f32 = 24.0;
const FONT_SIZE: f32 = 16.0;
const SCROLL_STEP: f32 = 6.0;
const MEASURED_FRAMES: usize = 240;

const SAMPLE_LINES: [&str; 4] = [
    "Aimer builds native user interfaces from a single declarative widget tree.",
    "Cupid batches rectangles, text and images into a single hardware pass;",
    "the glyph atlas keeps every rasterized bitmap so scrolling re-rasterizes nothing,",
    "and the layout cache keeps every line so scrolling re-shapes nothing either.",
];

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("text scroll benchmark device"),
            ..Default::default()
        })
        .block()
        .ok()
}

/// The document: one request per line, stacked vertically, clipped by the
/// viewport exactly the way a `Scrollable` clips its content.
fn document(line_count: usize, scroll_offset: f32) -> Vec<TextDrawRequest> {
    let clip = [0.0, 0.0, SURFACE_WIDTH as f32, SURFACE_HEIGHT as f32];
    (0..line_count)
        .map(|index| TextDrawRequest {
            x: 16.0,
            y: index as f32 * LINE_HEIGHT - scroll_offset,
            // Every line is unique, as a real document's lines are — shared
            // strings would let one cached layout serve the whole screen.
            text: Arc::from(
                format!("{index:>4} {}", SAMPLE_LINES[index % SAMPLE_LINES.len()]).as_str(),
            ),
            font_size: FONT_SIZE,
            color: Rgba8::from_unorm([0.1, 0.1, 0.1, 1.0]),
            bounds_width: SURFACE_WIDTH as f32 - 32.0,
            bounds_height: LINE_HEIGHT,
            overflow: TextOverflowMode::Clip,
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

    // Warm up until every line — visible and ahead of view — is prepared, so
    // the measured frames exercise only the steady-state scroll path.
    let mut warm_frames = 0;
    loop {
        let requests = document(line_count, 0.0);
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
    let max_offset = (line_count as f32 * LINE_HEIGHT - SURFACE_HEIGHT as f32).max(1.0);
    for frame in 0..MEASURED_FRAMES {
        let offset = (frame as f32 * SCROLL_STEP) % max_offset;
        let requests = document(line_count, offset);

        let start = Instant::now();
        pipeline.prepare(
            &backend,
            SURFACE_WIDTH,
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
        "text scroll benchmark: {line_count} lines ({OVERDRAW_FACTOR}x viewport), \
         {MEASURED_FRAMES} frames, warmed in {warm_frames} frames"
    );
    println!("prepare avg:   {:?}", total / MEASURED_FRAMES as u32);
    println!("prepare worst: {worst:?}");
}
