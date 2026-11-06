//! Loads Lottie files and reports what Aimer would skip.
//!
//! ```text
//! cargo run -p aimer_lottie --example inspect -- animation.json [more.json ...]
//! ```

use aimer_lottie::{Composition, LayerKind, LottieScene};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: inspect <animation.json>...");
        std::process::exit(2);
    }
    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("{path}: cannot read: {error}");
                continue;
            }
        };
        let composition = match Composition::parse(&bytes) {
            Ok(composition) => composition,
            Err(error) => {
                println!("{path}: cannot load: {error}");
                continue;
            }
        };
        let shape_layers = composition
            .layers
            .iter()
            .filter(|layer| matches!(layer.kind, LayerKind::Shape(_)))
            .count();
        println!(
            "{path}: {}x{} @ {} fps, {:.2} s, {} layers ({shape_layers} shape)",
            composition.width,
            composition.height,
            composition.frame_rate,
            composition.duration_secs(),
            composition.layers.len(),
        );
        let loaded = composition.diagnostics.len();
        let scene = LottieScene::new(composition);
        let frame = scene.frame(0.0);
        println!(
            "  scene: {} nodes, {} overrides at frame 0",
            frame.scene.nodes.len(),
            frame.overrides.len()
        );
        // Identical messages are common (one per animated property), so count them.
        let mut counts: Vec<(String, usize)> = Vec::new();
        let all = scene.composition().diagnostics.iter();
        for diagnostic in all {
            let message = diagnostic.to_string();
            match counts.iter_mut().find(|(m, _)| *m == message) {
                Some((_, n)) => *n += 1,
                None => counts.push((message, 1)),
            }
        }
        println!("  {} diagnostics ({loaded} while loading):", counts.iter().map(|c| c.1).sum::<usize>());
        for (message, n) in counts {
            println!("    {n:>3} x {message}");
        }
    }
}
