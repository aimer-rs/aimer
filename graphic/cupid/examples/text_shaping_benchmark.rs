use std::hint::black_box;
use aimer_time::{Duration, Instant};

use aimer_cupid::font::{FontFamily, FontStyle, FontWeight};
use aimer_cupid::text_layout::shape_text_styled;
use aimer_cupid::text_pipeline::glyph_rasterizer::GlyphRasterizer;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use unicode_segmentation::UnicodeSegmentation;

const TARGET_CHARACTERS: usize = 2_000;
const DEFAULT_ITERATIONS: usize = 10;
const COLD_BATCH_SIZE: usize = 16;
const PARALLEL_WORKERS: usize = 4;
const SAMPLE: &str =
    "Aimer shapes styled text into glyph runs before wrapping and painting the markdown document. ";
const UNICODE_SAMPLE: &str = "éclair سلام नमस्ते שלום 你好世界 សួស្តី العربية עברית ";

fn benchmark(iterations: usize, mut operation: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        operation();
    }
    start.elapsed()
}

fn average(duration: Duration, iterations: usize) -> Duration {
    duration / u32::try_from(iterations).expect("iteration count must fit in u32")
}

#[cfg(not(target_arch = "wasm32"))]
fn manual_parallel_for_each<T, F>(items: &[T], workers: usize, operation: F)
where
    T: Sync,
    F: Fn(&T) + Sync,
{
    let worker_count = workers.min(items.len()).max(1);
    let next_item = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                loop {
                    let index = next_item.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else {
                        break;
                    };
                    operation(item);
                }
            });
        }
    });
}

fn main() {
    let iterations = std::env::args()
        .nth(1)
        .map(|value| {
            value
                .parse()
                .expect("iterations must be a positive integer")
        })
        .unwrap_or(DEFAULT_ITERATIONS);
    assert!(iterations > 0, "iterations must be greater than zero");

    let text = SAMPLE
        .repeat(TARGET_CHARACTERS.div_ceil(SAMPLE.len()))
        .chars()
        .take(TARGET_CHARACTERS)
        .collect::<String>();
    let unicode_text = UNICODE_SAMPLE
        .repeat(TARGET_CHARACTERS.div_ceil(UNICODE_SAMPLE.chars().count()))
        .chars()
        .take(TARGET_CHARACTERS)
        .collect::<String>();

    let per_cluster = benchmark(iterations, || {
        let mut rasterizer = GlyphRasterizer::new();
        for cluster in text.graphemes(true) {
            black_box(rasterizer.shape_cluster_for_family(
                black_box(cluster),
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
            ));
        }
    });
    let per_run = benchmark(iterations, || {
        let mut rasterizer = GlyphRasterizer::new();
        black_box(shape_text_styled(
            &mut rasterizer,
            black_box(&text),
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        ));
    });
    let full_unicode_run = benchmark(iterations, || {
        let mut rasterizer = GlyphRasterizer::new();
        black_box(shape_text_styled(
            &mut rasterizer,
            black_box(&unicode_text),
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        ));
    });
    let direct_run = benchmark(iterations, || {
        let mut rasterizer = GlyphRasterizer::new();
        black_box(rasterizer.shape_run_for_family(
            black_box(&text),
            16.0,
            FontFamily::SANS_SERIF,
            FontWeight::Normal,
            FontStyle::Normal,
        ));
    });
    let direct_font_id_run = benchmark(iterations, || {
        let mut rasterizer = GlyphRasterizer::new();
        black_box(rasterizer.shape_run_with_font_id(
            black_box(&text),
            16.0,
            rasterizer.primary_font_id(),
            FontWeight::Normal,
        ));
    });

    let cold_inputs = (0..COLD_BATCH_SIZE)
        .map(|index| format!("{text} batch-{index}"))
        .collect::<Vec<_>>();
    let cold_serial = benchmark(iterations, || {
        for input in &cold_inputs {
            let mut rasterizer = GlyphRasterizer::new();
            black_box(shape_text_styled(
                &mut rasterizer,
                black_box(input),
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
                None,
            ));
        }
    });
    let cold_parallel = benchmark(iterations, || {
        #[cfg(not(target_arch = "wasm32"))]
        manual_parallel_for_each(&cold_inputs, PARALLEL_WORKERS, |input| {
            let mut rasterizer = GlyphRasterizer::new();
            black_box(shape_text_styled(
                &mut rasterizer,
                black_box(input),
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
                None,
            ));
        });

        #[cfg(target_arch = "wasm32")]
        for input in &cold_inputs {
            let mut rasterizer = GlyphRasterizer::new();
            black_box(shape_text_styled(
                &mut rasterizer,
                black_box(input),
                16.0,
                FontFamily::SANS_SERIF,
                FontWeight::Normal,
                FontStyle::Normal,
                None,
            ));
        }
    });

    let per_cluster_average = average(per_cluster, iterations);
    let per_run_average = average(per_run, iterations);
    let full_unicode_run_average = average(full_unicode_run, iterations);
    let direct_run_average = average(direct_run, iterations);
    let direct_font_id_run_average = average(direct_font_id_run, iterations);
    let speedup = per_cluster_average.as_secs_f64() / per_run_average.as_secs_f64();
    let cold_serial_average = average(cold_serial, iterations);
    let cold_parallel_average = average(cold_parallel, iterations);
    let cold_speedup = cold_serial_average.as_secs_f64() / cold_parallel_average.as_secs_f64();

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "{profile} text shaping benchmark: {} characters, {iterations} iterations",
        text.chars().count()
    );
    println!("per-cluster average: {per_cluster_average:?}");
    println!("per-run average:     {per_run_average:?}");
    println!("full Unicode run:    {full_unicode_run_average:?}");
    println!("direct run average:  {direct_run_average:?}");
    println!("direct font-id run:  {direct_font_id_run_average:?}");
    println!("speedup:             {speedup:.2}x");
    println!("cold serial batch:   {cold_serial_average:?}");
    println!("cold parallel batch: {cold_parallel_average:?}");
    println!("cold batch speedup:  {cold_speedup:.2}x");
}
