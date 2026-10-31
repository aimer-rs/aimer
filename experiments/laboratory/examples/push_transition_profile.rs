//! Cost of a route push transition: the first frame, then the frames of the animation.
//!
//! ```text
//! cargo run -p aimer_laboratory --example push_transition_profile
//! cargo run -p aimer_laboratory --example push_transition_profile --release
//! ```
//!
//! Build without `aimer/frame-stats` for timings: that feature turns on a per-frame
//! census walk that is part of what is measured. The `cmds`/`nodes` counters read zero
//! without it; add `--features aimer/frame-stats` to see them, and do not trust the
//! timings of that run.
//!
//! A push has two parts worth telling apart. The *first frame* builds the incoming
//! page, lays it out and records it for the first time, while the outgoing page is
//! kept. The *animation frames* then move two pages that already exist. The harness
//! prints both, with how many page widgets were built in each, so it shows whether
//! the pages are rebuilt while they animate or only once at the start.
//!
//! Knobs:
//! - `PUSH_PARAGRAPHS=<n>`: rows on each page (default 200).
//! - `PUSH_FRAMES=<n>`: animation frames measured per round (default 60).
//! - `PUSH_ROUNDS=<n>`: fresh apps measured; the first frame is reported as the
//!   median over rounds (default 5).
//! - `PUSH_TRANSITION=slide|fade|zoom`: the effect (default `slide`).
//! - `PUSH_DURATION_MS=<n>`: effect duration (default 10000). It is wall-clock time and
//!   must outlast the measured frames, otherwise the tail of the run is settled frames;
//!   the harness warns when it does not.

use std::cell::{Cell, RefCell};
use std::hint::black_box;
use std::time::Duration;
use aimer_time::Instant;

use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::router::{
    CrossFadeTransition, Navigator, NavigatorController, Route, Router, SlideTransition,
    ZoomTransition,
};
use aimer::style::LayoutSpacing;
use aimer::{
    AimerApp, AnyElement, AnyWidget, BuildContext, Color, Column, Container, HeadlessOptions,
    PortableWidget, Scrollable, Text, Widget,
};

const FRAME_WIDTH: u32 = 800;
const FRAME_HEIGHT: u32 = 700;

thread_local! {
    /// Pages built so far, by `Page::to_element`.
    static PAGE_BUILDS: Cell<usize> = const { Cell::new(0) };
    /// The navigator the home page looked up while it was built.
    static NAVIGATOR: RefCell<Option<NavigatorController<AppRoute>>> = const { RefCell::new(None) };
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum AppRoute {
    Home,
    Detail,
}

impl Route for AppRoute {
    fn parse(path: &str) -> Option<Self> {
        match path {
            "/" => Some(Self::Home),
            "/detail" => Some(Self::Detail),
            _ => None,
        }
    }

    fn format(&self) -> String {
        match self {
            Self::Home => "/".to_owned(),
            Self::Detail => "/detail".to_owned(),
        }
    }
}

impl Router for AppRoute {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        routes(*self)
    }
}

fn routes(route: AppRoute) -> AnyWidget {
    Page { route }.boxed()
}

/// A route's page. Counting here is what tells a rebuild from a retained page.
struct Page {
    route: AppRoute,
}

impl PortableWidget for Page {}

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        PAGE_BUILDS.with(|builds| builds.set(builds.get() + 1));
        if self.route == AppRoute::Home {
            NAVIGATOR.with_borrow_mut(|slot| *slot = Some(NavigatorController::of(ctx)));
        }
        page_body(self.route).to_element(ctx)
    }
}

fn paragraph(route: AppRoute, index: usize) -> AnyWidget {
    let body = format!(
        "{route:?} paragraph {index}: The compositor keeps this decorated scrolling row on the \
         live culled path because its container owns clipping and hit-test bounds. Text shaping \
         is cached per width, while stable paint islands elsewhere can be retained automatically \
         or promoted explicitly with RepaintBoundary."
    );
    let (even, odd) = match route {
        AppRoute::Home => (Color::WHITE, Color::Rgb(242, 242, 247)),
        AppRoute::Detail => (Color::Rgb(255, 250, 240), Color::Rgb(247, 240, 228)),
    };
    Container::new()
        .color(if index % 2 == 0 { even } else { odd })
        .padding(LayoutSpacing::all(12))
        .box_child(Text::new(body).wrapped())
}

fn page_body(route: AppRoute) -> AnyWidget {
    Scrollable::new()
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(Column::new().children(
            (0..env_usize("PUSH_PARAGRAPHS", 200)).map(|index| paragraph(route, index)),
        ))
        .boxed()
}

fn navigator(duration: Duration) -> impl Widget {
    let navigator = Navigator::new(AppRoute::Home, routes);
    match std::env::var("PUSH_TRANSITION").as_deref() {
        Ok("fade") => navigator
            .transition(CrossFadeTransition::new().duration(duration))
            .boxed(),
        Ok("zoom") => navigator
            .transition(ZoomTransition::new().duration(duration))
            .boxed(),
        _ => navigator
            .transition(SlideTransition::new().duration(duration))
            .boxed(),
    }
}

fn page_builds() -> usize {
    PAGE_BUILDS.with(Cell::get)
}

/// One measured round: the first frame after the push, then the animation frames.
struct Round {
    idle_us: Vec<f64>,
    first_us: f64,
    first_builds: usize,
    first_content: aimer::frame_stats::FrameContentStats,
    animation_us: Vec<f64>,
    animation_builds: usize,
    animation_content: aimer::frame_stats::FrameContentStats,
    wall: Duration,
}

fn measure_round(duration: Duration, frames: usize) -> Round {
    let mut app = AimerApp::start_headless_with(
        navigator(duration),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    );
    for _ in 0..4 {
        app.render_frame();
    }
    let controller = NAVIGATOR
        .with_borrow(Clone::clone)
        .expect("the home page was built inside the navigator");

    // A frame with no navigation in flight, for scale. These are cheap by design.
    let mut idle_us = Vec::with_capacity(32);
    for _ in 0..32 {
        let start = Instant::now();
        black_box(&mut app);
        app.render_frame();
        idle_us.push(start.elapsed().as_secs_f64() * 1e6);
    }

    aimer::frame_stats::reset_frame_content_stats();
    aimer::frame_stats::reset_frame_breakdown();
    let wall_start = Instant::now();

    let builds_before = page_builds();
    controller.push(AppRoute::Detail);
    let start = Instant::now();
    black_box(&mut app);
    app.render_frame();
    let first_us = start.elapsed().as_secs_f64() * 1e6;
    let first_builds = page_builds() - builds_before;
    let first_content = aimer::frame_stats::frame_content_stats();

    aimer::frame_stats::reset_frame_content_stats();
    let builds_before = page_builds();
    let mut animation_us = Vec::with_capacity(frames);
    for _ in 0..frames {
        let start = Instant::now();
        black_box(&mut app);
        app.render_frame();
        animation_us.push(start.elapsed().as_secs_f64() * 1e6);
    }
    assert_eq!(
        controller.current_route(),
        AppRoute::Detail,
        "the push must have taken effect"
    );
    Round {
        idle_us,
        first_us,
        first_builds,
        first_content,
        animation_us,
        animation_builds: page_builds() - builds_before,
        animation_content: aimer::frame_stats::frame_content_stats(),
        wall: wall_start.elapsed(),
    }
}

fn percentile(sorted: &[f64], per_cent: usize) -> f64 {
    sorted[(sorted.len() * per_cent / 100).min(sorted.len() - 1)]
}

fn main() {
    let duration = Duration::from_millis(env_usize("PUSH_DURATION_MS", 10_000) as u64);
    let frames = env_usize("PUSH_FRAMES", 60);
    let rounds = env_usize("PUSH_ROUNDS", 5);

    let mut idle = Vec::new();
    let mut first = Vec::new();
    let mut animation = Vec::new();
    let mut last = None;
    let mut outlasted = true;
    for _ in 0..rounds {
        let round = measure_round(duration, frames);
        outlasted &= round.wall < duration.mul_f64(0.9);
        idle.extend_from_slice(&round.idle_us);
        first.push(round.first_us);
        animation.extend_from_slice(&round.animation_us);
        last = Some(round);
    }
    let last = last.expect("PUSH_ROUNDS must be at least 1");

    idle.sort_by(f64::total_cmp);
    first.sort_by(f64::total_cmp);
    animation.sort_by(f64::total_cmp);

    println!(
        "push transition: rounds={rounds} frames/round={frames} paragraphs={} duration={}ms",
        env_usize("PUSH_PARAGRAPHS", 200),
        duration.as_millis(),
    );
    println!(
        "  idle frame      p50={:.1}us p95={:.1}us",
        percentile(&idle, 50),
        percentile(&idle, 95),
    );
    println!(
        "  first frame     p50={:.1}us max={:.1}us  pages built={}  cmds={:.0} nodes={:.0} \
         retained={:.0} text-cmds={:.0}",
        percentile(&first, 50),
        first[first.len() - 1],
        last.first_builds,
        last.first_content.average_draw_commands(),
        last.first_content.average_drawn_nodes(),
        last.first_content.average_retained_layers(),
        last.first_content.average_text_commands(),
    );
    println!(
        "  animation frame p50={:.1}us p95={:.1}us max={:.1}us  pages built={} over {frames} \
         frames  cmds/frame={:.1} nodes/frame={:.1} retained/frame={:.1} text-cmds/frame={:.1} \
         text-miss/frame={:.1}",
        percentile(&animation, 50),
        percentile(&animation, 95),
        animation[animation.len() - 1],
        last.animation_builds,
        last.animation_content.average_draw_commands(),
        last.animation_content.average_drawn_nodes(),
        last.animation_content.average_retained_layers(),
        last.animation_content.average_text_commands(),
        last.animation_content.average_text_cache_misses(),
    );
    if !outlasted {
        println!(
            "  WARNING: a round took {:?}, close to the {}ms effect; raise PUSH_DURATION_MS or \
             the tail of the run measured settled frames",
            last.wall,
            duration.as_millis(),
        );
    }
}
