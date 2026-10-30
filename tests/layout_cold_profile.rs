//! Profile harness: cost of the *first* layout of a freshly built element tree,
//! for wide trees and for deeply nested ones.
//!
//! Every sample builds a new element, so no `LayoutCache` entry exists yet; the
//! cache probe counts show how many measurements one cold pass really takes
//! relative to the node count (a ratio above ~1 means children are measured
//! more than once per pass).
//!
//! Ignored by default because it prints numbers instead of asserting; run with
//! `--ignored --nocapture`, single-threaded, in the debug profile.

use aimer::canvas::{FrameCanvas, InnerCanvas};
use aimer::style::LayoutSpacing;
use aimer::widget::base::WindowHandle;
use aimer::widget::layout_cache::probe;
use aimer::{
    AnyWidget, BoxAlignment, BuildContext, Color, Column, Container, Expanded, LayoutElement,
    ResolvedSize, Text, Vec2d, Widget,
};
use aimer::{ScrollAxis, Scrollable};
use aimer_attribute::BoxConstraint;
use std::time::Instant;

const VIEWPORT_W: f32 = 400.0;
const VIEWPORT_H: f32 = 800.0;
const SAMPLES: usize = 15;

fn context<'a>(canvas: FrameCanvas<'a>, runtime: &tokio::runtime::Runtime) -> BuildContext<'a> {
    let mut ctx = BuildContext::new(
        canvas,
        ResolvedSize {
            width: VIEWPORT_W,
            height: VIEWPORT_H,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(
            winit::dpi::PhysicalSize::new(VIEWPORT_W as u32, VIEWPORT_H as u32),
            1.0,
        ),
        runtime.handle().clone(),
    );
    ctx.box_constraint = BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: VIEWPORT_W,
        max_height: VIEWPORT_H,
    };
    ctx
}

/// Container(padding) > Container(padding) > Text: three nodes.
fn row(index: usize) -> AnyWidget {
    Container::new()
        .height(40)
        .padding(LayoutSpacing::new().left(12).right(12))
        .color(Color::Rgba(240, 240, 240, 255))
        .child(
            Container::new()
                .padding(LayoutSpacing::new().left(4))
                .child(Text::new(format!("row {index}"))),
        )
        .boxed()
}

fn leaf() -> AnyWidget {
    Text::new("leaf").boxed()
}

/// `n` rows in a plain `Column`: every row is measured.
fn wide_column(n: usize) -> AnyWidget {
    Column::new()
        .horizontal_alignment(BoxAlignment::Start)
        .children((0..n).map(row).collect::<Vec<AnyWidget>>())
        .boxed()
}

/// The same rows behind a `Scrollable`: only the viewport should be measured.
fn wide_scroll(n: usize) -> AnyWidget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(wide_column(n))
        .boxed()
}

/// `depth` nested padded containers around one text.
fn deep_container(depth: usize) -> AnyWidget {
    let mut widget = leaf();
    for _ in 0..depth {
        widget = Container::new()
            .padding(LayoutSpacing::new().left(1).right(1))
            .child(widget)
            .boxed();
    }
    widget
}

/// `depth` nested single-child columns around one text.
fn deep_column(depth: usize) -> AnyWidget {
    let mut widget = leaf();
    for _ in 0..depth {
        widget = Column::new().children(vec![widget]).boxed();
    }
    widget
}

/// `depth` nested columns, each wrapping its child in `Expanded`, which makes
/// the flex parent measure that child in a second, share-constrained pass.
fn deep_expanded(depth: usize) -> AnyWidget {
    let mut widget = leaf();
    for _ in 0..depth {
        widget = Column::new()
            .children(vec![Expanded::new().box_child(widget)])
            .boxed();
    }
    widget
}

/// A balanced tree: `fanout` columns per level, `depth` levels, rows at leaves.
fn balanced(depth: usize, fanout: usize) -> AnyWidget {
    if depth == 0 {
        return row(0);
    }
    Column::new()
        .children((0..fanout).map(|_| balanced(depth - 1, fanout)).collect::<Vec<_>>())
        .boxed()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

struct Row {
    label: String,
    nodes: usize,
    build_us: f64,
    cold_us: f64,
    warm_us: f64,
    sets: u64,
    misses: u64,
    hits: u64,
    /// First `update` after layout: where children are placed and drawn.
    first_update_us: f64,
    update_sets: u64,
    update_misses: u64,
    update_hits: u64,
    warm_update_us: f64,
}

fn run(
    runtime: &tokio::runtime::Runtime,
    label: impl Into<String>,
    nodes: usize,
    make: &dyn Fn() -> AnyWidget,
) -> Row {
    let inner = InnerCanvas::new();
    let canvas = FrameCanvas::new(&inner);
    let context = context(canvas, runtime);

    let (mut build, mut cold, mut warm) = (vec![], vec![], vec![]);
    let (mut first_update, mut warm_update) = (vec![], vec![]);
    let (mut sets, mut misses, mut hits) = (0, 0, 0);
    let (mut update_sets, mut update_misses, mut update_hits) = (0, 0, 0);
    for sample in 0..SAMPLES + 1 {
        let widget = make();

        let start = Instant::now();
        let element = widget.to_element(&context);
        let build_us = start.elapsed().as_secs_f64() * 1e6;

        probe::take();
        let start = Instant::now();
        element.layout(&context);
        let cold_us = start.elapsed().as_secs_f64() * 1e6;
        let (h, m, s) = probe::take();

        let start = Instant::now();
        element.layout(&context);
        let warm_us = start.elapsed().as_secs_f64() * 1e6;

        probe::take();
        let start = Instant::now();
        element.update(&context);
        let first_update_us = start.elapsed().as_secs_f64() * 1e6;
        let (uh, um, us) = probe::take();
        inner.take_draw_list();

        let start = Instant::now();
        element.update(&context);
        let warm_update_us = start.elapsed().as_secs_f64() * 1e6;
        inner.take_draw_list();

        // Sample 0 warms allocators and text caches; it is not reported.
        if sample > 0 {
            build.push(build_us);
            cold.push(cold_us);
            warm.push(warm_us);
            (hits, misses, sets) = (h, m, s);
            first_update.push(first_update_us);
            warm_update.push(warm_update_us);
            (update_hits, update_misses, update_sets) = (uh, um, us);
        }
        drop(element);
    }
    Row {
        label: label.into(),
        nodes,
        build_us: median(&mut build),
        cold_us: median(&mut cold),
        warm_us: median(&mut warm),
        sets,
        misses,
        hits,
        first_update_us: median(&mut first_update),
        update_sets,
        update_misses,
        update_hits,
        warm_update_us: median(&mut warm_update),
    }
}

fn print(rows: &[Row]) {
    println!(
        "{:<24} {:>6} {:>9} | {:>9} {:>6} | {:>10} {:>8} {:>6} {:>6} {:>6} | {:>10}",
        "case", "nodes", "build us", "layout us", "sets", "1st upd us", "us/node", "sets", "miss", "hits", "2nd upd us"
    );
    for r in rows {
        println!(
            "{:<24} {:>6} {:>9.1} | {:>9.1} {:>6} | {:>10.1} {:>8.2} {:>6} {:>6} {:>6} | {:>10.1}",
            r.label,
            r.nodes,
            r.build_us,
            r.cold_us,
            r.sets,
            r.first_update_us,
            r.first_update_us / r.nodes as f64,
            r.update_sets,
            r.update_misses,
            r.update_hits,
            r.warm_update_us,
        );
    }
}

#[test]
#[ignore = "profile harness: prints numbers, run with --ignored --nocapture"]
fn cold_layout_profile() {
    // Deep trees recurse once per level through several frames each; a debug
    // build overflows the default 2 MiB test stack long before the depths
    // below, so measure on a thread with room to spare.
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("test runtime");
            println!("profile = {}", if cfg!(debug_assertions) { "debug" } else { "release" });

            println!("\n== wide ==");
            let mut rows = vec![];
            for n in [100, 1_000, 5_000] {
                rows.push(run(&runtime, format!("column of {n} rows"), n * 3, &|| wide_column(n)));
            }
            for n in [1_000, 5_000] {
                rows.push(run(&runtime, format!("scrollable of {n} rows"), n * 3, &|| wide_scroll(n)));
            }
            print(&rows);

            println!("\n== deep: single chain ==");
            let mut rows = vec![];
            for d in [25, 50, 100, 200, 400] {
                rows.push(run(&runtime, format!("container x{d}"), d + 1, &|| deep_container(d)));
            }
            for d in [25, 50, 100, 200, 400] {
                rows.push(run(&runtime, format!("column x{d}"), d + 1, &|| deep_column(d)));
            }
            print(&rows);

            println!("\n== deep: Expanded chain (second pass per level) ==");
            let mut rows = vec![];
            for d in [2, 4, 8, 12, 16, 20] {
                rows.push(run(&runtime, format!("column>expanded x{d}"), d * 2 + 1, &|| {
                    deep_expanded(d)
                }));
            }
            print(&rows);

            println!("\n== balanced tree (depth, fanout) ==");
            let mut rows = vec![];
            for (d, f) in [(3, 8), (4, 6), (5, 4), (6, 4)] {
                let leaves = f_pow(f, d);
                let columns = (f_pow(f, d) - 1) / (f - 1);
                rows.push(run(&runtime, format!("balanced d{d} f{f}"), leaves * 3 + columns, &|| {
                    balanced(d, f)
                }));
            }
            print(&rows);
        })
        .expect("spawn")
        .join()
        .expect("profile thread");
}

fn f_pow(base: usize, exp: usize) -> usize {
    (0..exp).fold(1, |acc, _| acc * base)
}
