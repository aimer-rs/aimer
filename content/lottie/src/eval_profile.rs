//! Timing harness for the per-frame evaluation path. Ignored by default; run
//! with `cargo nextest run -p aimer_lottie --run-ignored only eval_profile`.
//! Debug build on purpose: that is the build the framework is tuned for.

use crate::*;
use std::hint::black_box;
use std::time::Instant;

/// Median nanoseconds per call over several batches.
fn time_ns(iterations: u32, mut work: impl FnMut(u32)) -> f64 {
    let mut samples: Vec<f64> = (0..9)
        .map(|_| {
            let start = Instant::now();
            for i in 0..iterations {
                work(i);
            }
            start.elapsed().as_nanos() as f64 / f64::from(iterations)
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn keys(n: usize, interpolation: Interpolation) -> Property<f32> {
    Property::Animated(
        (0..n)
            .map(|i| Keyframe { time: i as f32 * 10.0, value: (i % 7) as f32, interpolation })
            .collect(),
    )
}

/// `layers` layers in a parent chain of `depth`, each with animated position,
/// rotation and scale.
fn composition(layers: usize, depth: usize) -> Composition {
    let layer = |i: usize| {
        let parent = if !i.is_multiple_of(depth) { format!(r#""parent":{},"#, i) } else { String::new() };
        format!(
            r#"{{"ty":3,"ind":{ind},{parent}"ks":{{
                "p":{{"a":1,"k":[{{"t":0,"s":[0,0]}},{{"t":100,"s":[50,20]}}]}},
                "r":{{"a":1,"k":[{{"t":0,"s":[0]}},{{"t":100,"s":[90]}}]}},
                "s":{{"a":1,"k":[{{"t":0,"s":[100,100]}},{{"t":100,"s":[120,120]}}]}}}}}}"#,
            ind = i + 1
        )
    };
    let body: Vec<_> = (0..layers).map(layer).collect();
    Composition::parse(format!(
        r#"{{"fr":60,"ip":0,"op":100,"w":512,"h":512,"layers":[{}]}}"#,
        body.join(",")
    ))
    .expect("valid document")
}

#[test]
#[ignore = "timing harness"]
fn eval_profile() {
    let linear = keys(64, Interpolation::Linear);
    let eased = keys(
        64,
        Interpolation::Bezier { ease_out: [0.42, 0.0], ease_in: [0.58, 1.0] },
    );
    println!("scalar, 64 keys, linear   {:8.1} ns/eval", time_ns(20_000, |i| {
        black_box(linear.value_at(black_box(i as f32 % 630.0 + 0.37)));
    }));
    println!("scalar, 64 keys, bezier   {:8.1} ns/eval", time_ns(20_000, |i| {
        black_box(eased.value_at(black_box(i as f32 % 630.0 + 0.37)));
    }));

    for (layers, depth) in [(10, 1), (100, 1), (100, 4), (400, 8)] {
        let c = composition(layers, depth);
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        let mut out = Vec::new();
        let ns = time_ns(300, |i| {
            c.layer_states(black_box(i as f32 % 100.0 + 0.37), &mut out);
            black_box(&out);
        });
        println!(
            "layer_states {layers:>3} layers, chain {depth}  {:9.0} ns/frame  {:6.0} ns/layer",
            ns,
            ns / layers as f64
        );
    }

    // A scene frame: `n` groups of rect+fill under one layer, each group's
    // position and rotation animated, so every node needs an override.
    for n in [10usize, 100, 400] {
        let group = |i: usize| {
            format!(
                r#"{{"ty":"gr","it":[
                    {{"ty":"rc","p":{{"a":0,"k":[0,0]}},"s":{{"a":0,"k":[8,8]}},"r":{{"a":0,"k":2}}}},
                    {{"ty":"fl","c":{{"a":0,"k":[1,0,0,1]}},"o":{{"a":0,"k":100}}}},
                    {{"ty":"tr","p":{{"a":1,"k":[{{"t":0,"s":[{i},0]}},{{"t":100,"s":[{i},50]}}]}},
                     "r":{{"a":1,"k":[{{"t":0,"s":[0]}},{{"t":100,"s":[90]}}]}}}}]}}"#
            )
        };
        let shapes: Vec<_> = (0..n).map(group).collect();
        let c = Composition::parse(format!(
            r#"{{"fr":60,"ip":0,"op":100,"w":512,"h":512,"layers":[
                {{"ty":4,"nm":"L","ks":{{}},"shapes":[{}]}}]}}"#,
            shapes.join(",")
        ))
        .expect("valid document");
        let scene = LottieScene::new(c);
        let ns = time_ns(200, |i| {
            black_box(scene.frame(black_box(i as f32 % 100.0 + 0.37)));
        });
        println!(
            "LottieScene::frame {n:>3} animated nodes  {:9.0} ns/frame  {:6.0} ns/node",
            ns,
            ns / n as f64
        );
    }

    // The Jaime fingerprint scan: six animated trims and a layer transform, so
    // most of its 12 nodes rebuild their geometry every frame while it scans.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../app/jaime/assets/lottie/FingerprintScan.json");
    if let Ok(source) = std::fs::read_to_string(path) {
        let scene = LottieScene::new(Composition::parse(source).expect("valid document"));
        for (label, frame) in [("drawing on", 40.0f32), ("holding", 170.0)] {
            let ns = time_ns(200, |i| {
                black_box(scene.frame(black_box(frame + (i % 5) as f32 * 0.1)));
            });
            println!("fingerprint scan, {label:<10}  {ns:9.0} ns/frame");
        }
    }
}
