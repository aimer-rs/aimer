//! Profile harness: cost of rebuilding a chat transcript the way codex-gui does.
//!
//! codex-gui rebuilds every message widget whenever the session store changes,
//! which for a streaming reply is once per word. Each assistant message is a
//! `MarkdownViewer` (parse -> highlight -> widget tree), each user message is
//! shaped once to decide whether it wraps. This harness reproduces that shape
//! without depending on the app so the framework side can be measured and
//! optimised on its own.
//!
//! Two scenarios per transcript length:
//!
//! - `static`: every message is byte-identical to the previous rebuild (a window
//!   resize, a sidebar drag, any unrelated store change). Pure cache behaviour.
//! - `stream`: the last reply gains one line of code per rebuild, so exactly one
//!   message (and one code block) is new every time.
//!
//! The turn counts straddle the old markdown cache sizes (16 documents, 64
//! highlights), which used to cliff because the transcript is walked in the same
//! order every rebuild, the worst case for an LRU. The caches are now 256 / 512
//! deep, so these counts should scale linearly.
//!
//! Phases, all medians over the samples, in microseconds:
//! `rows` builds the widgets, `elem` is `to_element` (where markdown parses and
//! highlights), `layout` and `update` are the cold first passes of the new
//! element. Ignored by default because it prints numbers instead of asserting;
//! run with `--ignored --nocapture`, single-threaded, in the debug profile:
//!
//! `cargo test --test transcript_profile --features markdown,markdown-syntect -- --ignored --nocapture`
#![cfg(all(feature = "markdown", feature = "markdown-syntect"))]

use aimer::canvas::{FrameCanvas, InnerCanvas};
use aimer::style::{LayoutSpacing, TextOverflow, TextStyle};
use aimer::widget::base::WindowHandle;
use aimer::{
    AnyWidget, BuildContext, Color, Column, Container, MarkdownViewer, ResolvedSize, ScrollAxis,
    Scrollable, SizedBox, Text, Vec2d, Widget,
};
use aimer_attribute::BoxConstraint;
use aimer_time::Instant;

const VIEWPORT_W: f32 = 900.0;
const VIEWPORT_H: f32 = 800.0;
const SAMPLES: usize = 7;
/// The widest line a user message is set on before it wraps (codex-gui derives
/// this from the window; the exact value does not matter here).
const LIMIT: f32 = 500.0;
const ENTRY_GAP: f32 = 22.0;

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

/// What a reply contains.
#[derive(Clone, Copy, PartialEq)]
enum Flavor {
    /// Paragraphs and a list only.
    Prose,
    /// The same, plus two fenced Rust blocks.
    Code,
}

impl Flavor {
    fn name(self) -> &'static str {
        match self {
            Flavor::Prose => "prose",
            Flavor::Code => "code",
        }
    }
}

fn user_text(turn: usize) -> String {
    format!("Turn {turn}: can you walk me through how the scheduler picks the next task?")
}

/// An assistant reply that is unique to `turn`, so no two replies share a cache
/// entry. `extra` appends that many lines to the last code block, which is how a
/// streaming reply grows.
fn reply(turn: usize, flavor: Flavor, extra: usize) -> String {
    let mut text = format!(
        "## Answer {turn}\n\nThe scheduler keeps a ready queue per priority and always picks \
         from the highest non-empty one, so reply {turn} starts there.\n\n\
         - it never blocks the caller for turn {turn}\n\
         - ties are broken by arrival order\n\
         - a task that yields goes to the back of its queue\n\n"
    );
    if flavor == Flavor::Code {
        text.push_str(&format!(
            "```rust\nfn pick_{turn}(queues: &mut [Vec<Task>]) -> Option<Task> {{\n    \
             for queue in queues.iter_mut().rev() {{\n        if !queue.is_empty() {{\n            \
             return Some(queue.remove(0));\n        }}\n    }}\n    None\n}}\n```\n\n"
        ));
        text.push_str("A second look, with the yield path:\n\n```rust\n");
        text.push_str(&format!("fn yield_{turn}(task: Task, queue: &mut Vec<Task>) {{\n"));
        for line in 0..4 + extra {
            text.push_str(&format!("    queue.push(task.clone()); // step {line}\n"));
        }
        text.push_str("}\n```\n\n");
    }
    text.push_str("That is the whole policy; everything else is bookkeeping around it.");
    text
}

fn one_line_width(ctx: &BuildContext, text: &str, style: &TextStyle) -> f32 {
    let metrics = ctx.canvas.measure_text_metrics_styled(
        text,
        style.font_size.max(1) as f32 * ctx.scale,
        0.0,
        style.font_family,
        style.font_style,
        style.font_weight.numeric(),
    );
    metrics.width / ctx.scale
}

/// Mirrors `bubble::user_text`: measure on one line, wrap only if it overflows.
fn user_row(ctx: &BuildContext, text: &str) -> AnyWidget {
    let style = TextStyle::new().font_size(15).color(Color::Rgba(20, 20, 20, 255));
    let text_widget = if one_line_width(ctx, text, &style) <= LIMIT {
        Text::new(text.to_owned()).text_style(style.text_overflow(TextOverflow::Clip))
    } else {
        Text::new(text.to_owned()).text_style(style).wrapped()
    };
    Container::new()
        .fit_content()
        .padding(LayoutSpacing::new().top(10).bottom(10).left(18).right(18))
        .color(Color::Rgba(230, 230, 235, 255))
        .child(text_widget)
        .boxed()
}

fn activity_row() -> AnyWidget {
    Text::new("Initialized session".to_owned())
        .text_style(TextStyle::new().font_size(14))
        .boxed()
}

fn assistant_row(text: &str) -> AnyWidget {
    MarkdownViewer::new()
        .scrollable(false)
        .padding(LayoutSpacing::all(0))
        .markdown(text.to_owned())
        .boxed()
}

/// The rows of `turns` turns, as `ChatState::transcript` builds them. Only the
/// last reply carries `extra`.
fn rows(ctx: &BuildContext, turns: usize, flavor: Flavor, extra: usize) -> Vec<AnyWidget> {
    let mut rows = Vec::with_capacity(turns * 6);
    for turn in 0..turns {
        let grow = if turn + 1 == turns { extra } else { 0 };
        rows.push(user_row(ctx, &user_text(turn)));
        rows.push(SizedBox::new().height(ENTRY_GAP).boxed());
        rows.push(activity_row());
        rows.push(SizedBox::new().height(ENTRY_GAP).boxed());
        rows.push(assistant_row(&reply(turn, flavor, grow)));
        rows.push(SizedBox::new().height(ENTRY_GAP).boxed());
    }
    rows
}

fn scrollable(child: AnyWidget) -> AnyWidget {
    Scrollable::new()
        .axis(ScrollAxis::Vertical)
        .child(
            Container::new()
                .padding(LayoutSpacing::new().top(80).bottom(150).left(20).right(20))
                .child(child),
        )
        .boxed()
}

/// One windowed row per turn, for comparison: the same content behind
/// `Column::list`, built on demand from the turn index alone.
fn windowed(turns: usize, flavor: Flavor, extra: usize) -> AnyWidget {
    scrollable(
        Column::new()
            .list(0..turns)
            .builder(move |&turn| {
                let grow = if turn + 1 == turns { extra } else { 0 };
                Column::new().children([
                    Container::new()
                        .fit_content()
                        .child(Text::new(user_text(turn)))
                        .boxed(),
                    SizedBox::new().height(ENTRY_GAP).boxed(),
                    activity_row(),
                    SizedBox::new().height(ENTRY_GAP).boxed(),
                    assistant_row(&reply(turn, flavor, grow)),
                    SizedBox::new().height(ENTRY_GAP).boxed(),
                ])
            })
            .boxed(),
    )
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

#[derive(Default)]
struct Phases {
    rows: Vec<f64>,
    elem: Vec<f64>,
    layout: Vec<f64>,
    update: Vec<f64>,
}

struct Line {
    label: String,
    rows: f64,
    elem: f64,
    layout: f64,
    update: f64,
}

impl Line {
    fn total(&self) -> f64 {
        self.rows + self.elem + self.layout + self.update
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Scenario {
    Static,
    Stream,
    Windowed,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::Static => "static",
            Scenario::Stream => "stream",
            Scenario::Windowed => "windowed",
        }
    }
}

fn run(
    runtime: &tokio::runtime::Runtime,
    turns: usize,
    flavor: Flavor,
    scenario: Scenario,
) -> Line {
    let inner = InnerCanvas::new();
    let canvas = FrameCanvas::new(&inner);
    let ctx = context(canvas, runtime);
    let mut phases = Phases::default();

    // Sample 0 warms allocators, fonts and the markdown caches, so every
    // reported sample is a steady-state rebuild, as in a long session.
    for sample in 0..SAMPLES + 1 {
        let extra = if scenario == Scenario::Stream { sample } else { 0 };

        let start = Instant::now();
        let widget = if scenario == Scenario::Windowed {
            windowed(turns, flavor, extra)
        } else {
            scrollable(Column::new().children(rows(&ctx, turns, flavor, extra)).boxed())
        };
        let rows_us = start.elapsed().as_secs_f64() * 1e6;

        let start = Instant::now();
        let element = widget.to_element(&ctx);
        let elem_us = start.elapsed().as_secs_f64() * 1e6;

        let start = Instant::now();
        element.layout(&ctx);
        let layout_us = start.elapsed().as_secs_f64() * 1e6;

        let start = Instant::now();
        element.update(&ctx);
        let update_us = start.elapsed().as_secs_f64() * 1e6;
        inner.take_draw_list();

        if sample > 0 {
            phases.rows.push(rows_us);
            phases.elem.push(elem_us);
            phases.layout.push(layout_us);
            phases.update.push(update_us);
        }
        drop(element);
    }
    Line {
        label: format!("{} {} x{turns}", scenario.name(), flavor.name()),
        rows: median(&mut phases.rows),
        elem: median(&mut phases.elem),
        layout: median(&mut phases.layout),
        update: median(&mut phases.update),
    }
}

fn print(lines: &[Line]) {
    println!(
        "{:<20} {:>10} {:>11} {:>10} {:>10} | {:>11} {:>10}",
        "case", "rows us", "to_element", "layout us", "update us", "total ms", "ms/turn"
    );
    for line in lines {
        let turns: f64 = line
            .label
            .rsplit('x')
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1.0);
        println!(
            "{:<20} {:>10.0} {:>11.0} {:>10.0} {:>10.0} | {:>11.2} {:>10.3}",
            line.label,
            line.rows,
            line.elem,
            line.layout,
            line.update,
            line.total() / 1e3,
            line.total() / 1e3 / turns,
        );
    }
}

#[test]
#[ignore = "profile harness: prints numbers, run with --ignored --nocapture"]
fn transcript_rebuild_profile() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("test runtime");
            println!("profile = {}", if cfg!(debug_assertions) { "debug" } else { "release" });
            println!("doc cache = 256 documents, highlight cache = 512 code blocks (2 per code reply)");

            for scenario in [Scenario::Static, Scenario::Stream] {
                for flavor in [Flavor::Prose, Flavor::Code] {
                    println!("\n== {} / {} ==", scenario.name(), flavor.name());
                    let mut lines = vec![];
                    for turns in [4, 8, 16, 17, 24, 32, 33, 48] {
                        lines.push(run(&runtime, turns, flavor, scenario));
                    }
                    print(&lines);
                }
            }

            println!("\n== windowed (Column::list) / code ==");
            let mut lines = vec![];
            for turns in [16, 17, 33, 48] {
                lines.push(run(&runtime, turns, Flavor::Code, Scenario::Windowed));
            }
            print(&lines);
        })
        .expect("spawn")
        .join()
        .expect("profile thread");
}
