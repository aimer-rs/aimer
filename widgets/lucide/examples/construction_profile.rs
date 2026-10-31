//! Run with `cargo run -p aimer_lucide --example construction_profile`, then
//! repeat with `--release` to compare first use and cached construction.
use std::hint::black_box;
use aimer_time::Instant;

use aimer_lucide::Icons;
use aimer_svg::{Svg, SvgDocument};

fn main() {
    let count = Icons::ALL.len();
    let start = Instant::now();
    for (_, icon) in Icons::ALL {
        black_box(icon.document());
    }
    println!("{count} cold documents: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    for (label, build) in [
        ("cached document", 0), ("cached Svg widget", 1), ("runtime parse + Svg widget", 2),
    ] {
        let start = Instant::now();
        let rounds = 10;
        for _ in 0..rounds {
            for (_, icon) in Icons::ALL {
                match build {
                    0 => { black_box(icon.document()); }
                    1 => { black_box(Svg::new(*icon)); }
                    _ => { black_box(Svg::new(SvgDocument::from_svg(icon.source()).unwrap())); }
                }
            }
        }
        println!("{label}: {:.1} ns/icon", start.elapsed().as_nanos() as f64 / (rounds * count) as f64);
    }
}
