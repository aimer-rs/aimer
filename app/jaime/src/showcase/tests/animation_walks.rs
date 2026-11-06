//! Which example pages keep asking for frames once they have settled, and how
//! many of those frames walk the whole tree.
//!
//! Run with `cargo nextest run -p jaime --lib --run-ignored only animation_walks
//! --no-capture`. It prints a table and asserts nothing.

use super::*;

use std::time::Duration;

use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{AnyElement, HeadlessOptions};

struct Page(ExampleId);

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let state = ExampleShowcaseState::with_selected(self.0);
        state
            .build(ctx)
            .to_element(ctx)
    }
}

#[test]
#[ignore = "diagnostic harness: prints which pages keep walking the tree and asserts nothing"]
fn animation_walks_by_page() {
    let only = std::env::var("ONLY_PAGE").ok();
    for &id in EXAMPLES {
        if only
            .as_deref()
            .is_some_and(|name| name != id.label())
        {
            continue;
        }
        let mut app = AimerApp::start_headless_with(
            theme::provide(Page(id)),
            HeadlessOptions {
                size: PhysicalSize::new(1280, 800),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(8);
        let before = aimer::traversal_counts();
        let gens = || {
            (
                aimer::rebuild_invalidation_generation(),
                aimer::element_tree_generation(),
                aimer::retained_render_structure_generation(),
                aimer::layout_invalidation_generation(),
            )
        };
        let gens_before = gens();
        let mut frames = 0;
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(10));
            if app.pump_frames(1) == 0 {
                break;
            }
            frames += 1;
        }
        let after = aimer::traversal_counts();
        if frames > 0 {
            let gens_after = gens();
            println!(
                "ANIMATING {:<28} frames={frames:<3} full_walks={:<3} animation_only={:<3} \
                 per-frame moves: rebuild={} element_tree={} render_structure={} layout={}",
                id.label(),
                after.0 - before.0,
                after.1 - before.1,
                gens_after.0 - gens_before.0,
                gens_after.1 - gens_before.1,
                gens_after.2 - gens_before.2,
                gens_after.3 - gens_before.3,
            );
        }
    }
}

/// Per-frame wall time of one page that animates on its own, in the showcase
/// shell. `ONLY_PAGE` picks the page by label; the default is the loading page.
///
/// Run with `cargo nextest run -p jaime --lib --run-ignored only animation_frame_time
/// --no-capture`.
#[test]
#[ignore = "diagnostic harness: prints frame times and asserts nothing"]
fn animation_frame_time() {
    let label = std::env::var("ONLY_PAGE").unwrap_or_else(|_| "Loading animation".into());
    let id = EXAMPLES
        .iter()
        .copied()
        .find(|id| id.label() == label)
        .expect("ONLY_PAGE names a registered example");
    let options = HeadlessOptions {
        size: PhysicalSize::new(1280, 800),
        scale_factor: 1.0,
    };
    // `BARE=1` mounts the page without the showcase shell, as a control.
    let page = if std::env::var_os("BARE").is_some() {
        theme::provide(crate::loading_animation::loading_animation_example()).boxed()
    } else {
        theme::provide(Page(id)).boxed()
    };
    let mut app = AimerApp::start_headless_with(page, options);
    app.pump_frames(8);
    let mut times = Vec::new();
    let frames = std::env::var("FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(240);
    for _ in 0..frames {
        // Without a pause the controller barely advances between frames.
        std::thread::sleep(Duration::from_millis(2));
        let start = std::time::Instant::now();
        let drawn = app.pump_frames(1);
        if drawn > 0 {
            times.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    times.sort_by(f64::total_cmp);
    let p = |q: usize| times[(times.len() * q / 100).min(times.len() - 1)];
    println!(
        "FRAME {label}: n={} p50={:.1}us p95={:.1}us",
        times.len(),
        p(50),
        p(95)
    );
}
