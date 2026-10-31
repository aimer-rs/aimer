//! Cold glyph rasterization cost — the work a frame pays when a large blob of
//! text no glyph cache has seen yet enters the screen.
//!
//! Shaping and layout are measured by `text_shaping_benchmark`; this one
//! isolates the step after them, where every distinct `(face, glyph, size)`
//! triple is turned into coverage. That is the step a freshly scrolled code
//! block stalls on, so it is the number to watch when optimizing it.
//!
//! ```text
//! cargo run --release --example glyph_rasterization_benchmark
//! ```

use std::hint::black_box;
use aimer_time::{Duration, Instant};

use aimer_cupid::text_pipeline::glyph_rasterizer::GlyphRasterizer;

/// Sizes a document mixes: body, captions, headings, code.
const SIZES: &[f32] = &[12.0, 13.0, 14.0, 16.0, 18.0, 20.0, 24.0, 32.0];
const DEFAULT_ITERATIONS: usize = 5;

/// Every printable ASCII character, which is what a code block draws from.
fn charset() -> Vec<char> {
    (0x21u8..0x7f).map(char::from).collect()
}

fn average(duration: Duration, iterations: usize) -> Duration {
    duration / u32::try_from(iterations).expect("iteration count must fit in u32")
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

    let charset = charset();
    let glyphs_per_pass = charset.len() * SIZES.len();
    let text = charset.iter().collect::<String>();

    let mut runs = Duration::ZERO;
    for _ in 0..iterations {
        let mut rasterizer = GlyphRasterizer::new();
        let start = Instant::now();
        let mut checksum = 0usize;
        for size in SIZES {
            rasterizer.preload_text_into(black_box(&text), *size, None, |key, glyph| {
                checksum = checksum
                    .wrapping_add(usize::from(key.glyph_id))
                    .wrapping_add(glyph.bitmap.len())
                    .wrapping_add(glyph.width as usize)
                    .wrapping_add(glyph.height as usize);
            });
        }
        black_box(checksum);
        runs += start.elapsed();
    }

    let mut key_total = Duration::ZERO;
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        // A rasterizer per iteration is a cold glyph cache, which is the
        // situation being measured. Its construction is outside the timing.
        let mut rasterizer = GlyphRasterizer::new();
        let mut keys = Vec::with_capacity(glyphs_per_pass);
        let key_start = Instant::now();
        for size in SIZES {
            for codepoint in &charset {
                keys.push((rasterizer.glyph_key_for_codepoint(*codepoint, *size), *size));
            }
        }
        key_total += key_start.elapsed();

        let start = Instant::now();
        for (key, size) in &keys {
            black_box(rasterizer.rasterize_key(black_box(*key), *size));
        }
        total += start.elapsed();
    }

    let glyphs = u32::try_from(glyphs_per_pass).expect("glyph count must fit in u32");
    let one_at_a_time = average(total, iterations);
    let in_runs = average(runs, iterations);
    let key_preparation = average(key_total, iterations);

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "{profile} cold glyph rasterization: {glyphs_per_pass} distinct glyphs, {iterations} iterations"
    );
    println!(
        "one glyph at a time: {one_at_a_time:?} ({:?} per glyph)",
        one_at_a_time / glyphs
    );
    println!(
        "in runs:             {in_runs:?} ({:?} per glyph)",
        in_runs / glyphs
    );
    println!(
        "key preparation:     {key_preparation:?} ({:?} per glyph)",
        key_preparation / glyphs
    );
    println!(
        "saved:               {:.0}%",
        100.0 * (1.0 - in_runs.as_secs_f64() / one_at_a_time.as_secs_f64())
    );
}
