//! CPU preparation cost of a dense text-decoration frame.
//!
//! The workload keeps decoration geometry in final screen-space pixels and
//! moves it a little on every iteration. That avoids the complete retained
//! frame fast path and measures the steady-state instance build/upload work
//! that remains while a decorated text view scrolls.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p aimer_cupid --example text_decoration_benchmark --release
//! ```

use std::hint::black_box;
use aimer_time::{Duration, Instant};

use aimer_cupid::AntiAlias;
use aimer_cupid::backend::wgpu::WgpuBackend;
use aimer_cupid::text_pipeline::{TextDecorationDraw, TextPipelineV2, TextPreparationProfile};
use aimer_cupid::utilities::Rgba8;
use aimer_utils::SyncFuture;

const SURFACE_WIDTH: u32 = 1200;
const SURFACE_HEIGHT: u32 = 800;
const LINE_HEIGHT: f32 = 24.0;
const DECORATED_LINES: usize = 1_024;
const MEASURED_FRAMES: usize = 120;
const SCROLL_STEP: f32 = 6.0;

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions::default())
        .block()
        .ok()
        .or_else(|| {
            instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    force_fallback_adapter: true,
                    ..Default::default()
                })
                .block()
                .ok()
        })?;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("text decoration benchmark device"),
            ..Default::default()
        })
        .block()
        .ok()
}

fn decorations(scroll_offset: f32, lines_per_fragment: usize) -> Vec<TextDecorationDraw> {
    let clip = [0.0, 0.0, SURFACE_WIDTH as f32, SURFACE_HEIGHT as f32];
    (0..DECORATED_LINES)
        .flat_map(|line| {
            let y = line as f32 * LINE_HEIGHT - scroll_offset;
            (0..lines_per_fragment).map(move |stroke| TextDecorationDraw {
                x: 16.0,
                y: y + stroke as f32 * 4.0,
                width: SURFACE_WIDTH as f32 - 32.0,
                band_height: 2.0,
                thickness: 1.0,
                period: 8.0,
                style: stroke as u32,
                color: Rgba8::from_unorm([0.1, 0.1, 0.1, 1.0]),
                clip_rect: clip,
                clip_border_radius: [0.0; 4],
            })
        })
        .collect()
}

fn average_profile(samples: &[TextPreparationProfile]) -> TextPreparationProfile {
    let mut average = TextPreparationProfile::default();
    for sample in samples {
        average.total += sample.total;
        average.instance_build += sample.instance_build;
        average.instance_upload += sample.instance_upload;
    }
    if let Some(count) = u32::try_from(samples.len())
        .ok()
        .filter(|&count| count != 0)
    {
        average.total /= count;
        average.instance_build /= count;
        average.instance_upload /= count;
    }
    average
}

fn run_case(
    backend: &WgpuBackend,
    lines_per_fragment: usize,
) -> (Vec<Duration>, Vec<TextPreparationProfile>) {
    let mut pipeline = TextPipelineV2::new(
        backend,
        wgpu::TextureFormat::Rgba8Unorm,
        AntiAlias::Analytic,
    );
    let _ = pipeline.prepare_profiled(
        backend,
        SURFACE_WIDTH,
        SURFACE_HEIGHT,
        false,
        &[],
        &decorations(-SCROLL_STEP, lines_per_fragment),
    );

    let mut durations = Vec::with_capacity(MEASURED_FRAMES);
    let mut profiles = Vec::with_capacity(MEASURED_FRAMES);
    for frame in 0..MEASURED_FRAMES {
        let offset = frame as f32 * SCROLL_STEP;
        let frame_decorations = decorations(offset, lines_per_fragment);
        let start = Instant::now();
        let profile = pipeline.prepare_profiled(
            backend,
            SURFACE_WIDTH,
            SURFACE_HEIGHT,
            false,
            &[],
            black_box(&frame_decorations),
        );
        durations.push(start.elapsed());
        profiles.push(profile);
    }
    (durations, profiles)
}

fn print_case(label: &str, durations: &mut [Duration], profiles: &[TextPreparationProfile]) {
    durations.sort_unstable();
    let total = durations.iter().copied().sum::<Duration>();
    let average = total / durations.len() as u32;
    let p50 = durations[durations.len() / 2];
    let p95 = durations[(durations.len() - 1) * 95 / 100];
    let worst = durations[durations.len() - 1];
    let profile = average_profile(profiles);
    println!(
        "{label}: avg={average:?}, p50={p50:?}, p95={p95:?}, worst={worst:?}, \
         profiled_total={:?}, instance_build={:?}, instance_upload={:?}",
        profile.total, profile.instance_build, profile.instance_upload,
    );
}

fn main() {
    let Some((device, queue)) = gpu() else {
        eprintln!("skipping: no GPU adapter available");
        return;
    };
    let backend = WgpuBackend::new(device, queue);

    let (mut one_durations, one_profiles) = run_case(&backend, 1);
    let (mut three_durations, three_profiles) = run_case(&backend, 3);
    let one_count = DECORATED_LINES;
    let three_count = DECORATED_LINES * 3;

    println!(
        "text decoration benchmark: {DECORATED_LINES} fragments, \
         {MEASURED_FRAMES} moving frames"
    );
    println!("one stroke per fragment ({one_count} instances):");
    print_case("  steady state", &mut one_durations, &one_profiles);
    println!("three strokes per fragment ({three_count} instances):");
    print_case("  steady state", &mut three_durations, &three_profiles);
}
