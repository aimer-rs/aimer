//! Per-frame cost of a looping rotation, the shape of Jaime's loading page.
//!
//! ```text
//! cargo run -p aimer_laboratory --example loading_animation_profile
//! cargo run -p aimer_laboratory --example loading_animation_profile --release
//! ```
//!
//! `LOADING_WINDOW=1` runs the same page in a real window instead.
//! `LOADING_BUILDER_FRAMES=<n>` and `LOADING_STATE_FRAMES=<n>` time a build- and
//! a state-driven page; add `LOADING_NO_INCREMENTAL=1` to turn off the
//! incremental synchronization and compare.
//!
//! Prints the median and p95 frame time plus the work counters of one
//! steady-state frame. `LOADING_FRAMES=<n>` changes the sample size.

use std::time::{Duration, Instant};

use aimer::animation::{AnimatedBuilder, AnimationController, curves, RotationTransition};
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::style::{FontWeight, TextAlign, TextStyle};
use aimer::{
    AimerApp, BoxAlignment, Color, Column, Container, Dimension, HeadlessOptions, SizedBox, Svg,
    SvgDocument, Text, Widget,
};
use aimer::{AnyElement, BuildContext, State, StateUpdater, StatefulElement, StatefulWidget};
use std::cell::RefCell;
use std::rc::Rc;

const FRAME_WIDTH: u32 = 1280;
const FRAME_HEIGHT: u32 = 800;

fn measured_frames() -> usize {
    std::env::var("LOADING_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(240)
}

fn page() -> impl Widget {
    let controller = AnimationController::new(Duration::from_millis(1500), curves::LINEAR);
    controller.set_repeat(true);
    controller.set_curve(curves::EASE_IN_OUT);
    controller.forward_from_first_tick();
    let icon = Svg::new(
        SvgDocument::from_svg(include_bytes!(
            "../../jaime/assets/loading-1-svgrepo-com.svg"
        ))
        .expect("the bundled loading icon SVG should be valid"),
    )
    .bounded()
    .width(Dimension::Px(96.0))
    .height(Dimension::Px(96.0));

    Container::new()
        .width(Dimension::Percent(100.0))
        .height(Dimension::Percent(100.0))
        .color(Color::WHITE)
        .child(
            Column::new()
                .horizontal_alignment(BoxAlignment::Center)
                .vertical_alignment(BoxAlignment::Center)
                .children(vec![
                    RotationTransition::new(controller, icon).boxed(),
                    SizedBox::new().height(16).boxed(),
                    Text::new("Loading...")
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(24)
                                .font_weight(FontWeight::Bold)
                                .color(Color::BLACK),
                        )
                        .boxed(),
                ]),
        )
}

/// A repeating `AnimatedBuilder` above a few hundred static rows: the shape of
/// a build-driven animation inside a real page. Used with an external sampler.
fn builder_page(calls: Rc<std::cell::Cell<usize>>) -> impl Widget {
    let controller = AnimationController::new(Duration::from_millis(1500), curves::LINEAR);
    controller.set_repeat(true);
    controller.forward_from_first_tick();
    let animated = AnimatedBuilder::new(controller, move |progress| {
        calls.set(calls.get() + 1);
        SizedBox::new()
            .width(Dimension::Px(96.0))
            .height(Dimension::Px(96.0))
            .color(Color::Rgb((progress * 255.0) as u8, 40, 40))
    });
    let mut rows = vec![animated.boxed()];
    rows.extend((0..300).map(|i| {
        SizedBox::new()
            .width(Dimension::Px(300.0))
            .height(Dimension::Px(8.0))
            .color(if i % 2 == 0 {
                Color::WHITE
            } else {
                Color::Rgb(240, 240, 245)
            })
            .boxed()
    }));
    Container::new()
        .color(Color::WHITE)
        .child(Column::new().children(rows))
}

/// A stateful root above the same few hundred rows: every `set_state` rebuilds
/// one small child, the shape of a click handler or a timer in a real page.
struct StatePage {
    handle: Rc<RefCell<Option<StateUpdater<StatePageState>>>>,
}

struct StatePageState {
    count: u32,
    handle: Rc<RefCell<Option<StateUpdater<StatePageState>>>>,
}

impl StatefulWidget for StatePage {
    type State = StatePageState;

    fn create_state(self) -> StatePageState {
        StatePageState {
            count: 0,
            handle: self.handle,
        }
    }
}

impl State<StatePage> for StatePageState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.handle.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        SizedBox::new()
            .width(Dimension::Px(96.0))
            .height(Dimension::Px(96.0))
            .color(Color::Rgb((self.count % 256) as u8, 40, 40))
    }
}

/// The stateful box above static rows: the state is local, as in a real page.
fn state_page(handle: Rc<RefCell<Option<StateUpdater<StatePageState>>>>) -> impl Widget {
    let mut rows = vec![StatePage { handle }.boxed()];
    rows.extend((0..300).map(|i| {
        SizedBox::new()
            .width(Dimension::Px(300.0))
            .height(Dimension::Px(8.0))
            .color(if i % 2 == 0 {
                Color::WHITE
            } else {
                Color::Rgb(240, 240, 245)
            })
            .boxed()
    }));
    Container::new()
        .color(Color::WHITE)
        .child(Column::new().children(rows))
}

impl Widget for StatePage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "StatePage", None)
            .0
            .boxed()
    }
}

impl aimer::PortableWidget for StatePage {}

fn main() {
    // `LOADING_STATE_FRAMES=<n>` calls `set_state` before each of `n` frames of
    // the page above, flat out, so a sampler can attach.
    if let Some(frames) = std::env::var("LOADING_STATE_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let handle = Rc::new(RefCell::new(None));
        let mut app = AimerApp::start_headless_with(
            state_page(handle.clone()),
            HeadlessOptions {
                size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
                scale_factor: 1.0,
            },
        );
        app.set_incremental_sync(std::env::var_os("LOADING_NO_INCREMENTAL").is_none());
        app.render_frame();
        app.render_frame();
        let updater = handle.borrow().expect("the state mounted");
        let start = Instant::now();
        // The update is made by a task, as a timer or a finished request makes
        // it, and the scheduler's wake asks for the frame. A direct `set_state`
        // keeps the ordinary full frame.
        let venus = app.venus().clone();
        for _ in 0..frames {
            venus.spawn(async move { updater.set_state(|state| state.count += 1) });
            aimer_events::window::request_scoped_frame();
            app.render_frame();
        }
        println!(
            "set_state frame: {:.1}us",
            start.elapsed().as_secs_f64() * 1e6 / frames as f64
        );
        println!("syncs (full, scoped): {:?}", app.sync_counts());
        println!("tree walks, passes: {:?}", aimer::traversal_counts());
        return;
    }
    // `LOADING_BUILDER_FRAMES=<n>` renders a build-driven page flat out so a
    // sampler can attach; it prints nothing else.
    if let Some(frames) = std::env::var("LOADING_BUILDER_FRAMES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let calls = Rc::new(std::cell::Cell::new(0));
        let mut app = AimerApp::start_headless_with(
            builder_page(calls.clone()),
            HeadlessOptions {
                size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
                scale_factor: 1.0,
            },
        );
        app.set_incremental_sync(std::env::var_os("LOADING_NO_INCREMENTAL").is_none());
        let start = Instant::now();
        for _ in 0..frames {
            app.render_frame();
        }
        println!(
            "builder frame: {:.1}us, builder calls: {}",
            start.elapsed().as_secs_f64() * 1e6 / frames as f64,
            calls.get()
        );
        println!("syncs (full, scoped): {:?}", app.sync_counts());
        println!(
            "tree walks, animation-only passes: {:?}",
            aimer::traversal_counts()
        );
        return;
    }
    // `LOADING_WINDOW=1` opens a real window, so the native encode and present
    // path can be sampled by an external tool; the headless run below cannot see it.
    if std::env::var_os("LOADING_WINDOW").is_some() {
        AimerApp::start(page());
        return;
    }
    let mut app = AimerApp::start_headless_with(
        page(),
        HeadlessOptions {
            size: PhysicalSize::new(FRAME_WIDTH, FRAME_HEIGHT),
            scale_factor: 1.0,
        },
    );
    for _ in 0..8 {
        app.render_frame();
    }

    aimer::frame_stats::reset_frame_content_stats();
    let mut times = Vec::with_capacity(measured_frames());
    for _ in 0..measured_frames() {
        let start = Instant::now();
        app.render_frame();
        times.push(start.elapsed().as_secs_f64() * 1e6);
        // Without a pause the controller would barely advance between frames.
        std::thread::sleep(Duration::from_millis(2));
    }
    times.sort_by(f64::total_cmp);
    let p50 = times[times.len() / 2];
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    let s = aimer::frame_stats::frame_content_stats();
    let per = |value: u64| value as f64 / s.frames.max(1) as f64;
    println!(
        "loading: p50={p50:.1}us p95={p95:.1}us  (60 Hz budget share p50={:.1}%)",
        p50 / 16_667.0 * 100.0
    );
    println!(
        "per frame: drawn_nodes={:.1} draw_cmds={:.1} layout_calls={:.1} paint_calls={:.1} \
         stateful_builds={:.1} stateless_builds={:.1} rebuild_visits={:.1} redraw_requests={:.1} \
         invalidations={:.1} text_cmds={:.1}",
        per(s.drawn_nodes),
        per(s.draw_commands),
        per(s.layout_calls),
        per(s.paint_calls),
        per(s.stateful_builds),
        per(s.stateless_builds),
        per(s.rebuild_visits),
        per(s.redraw_requests),
        per(s.invalidations_queued),
        per(s.text_commands),
    );
}
