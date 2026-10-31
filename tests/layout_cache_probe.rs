//! TEMPORARY diagnostic: measures the `LayoutCache` hit rate across the
//! measure pass and the draw pass of a scroll-list-shaped tree.

use aimer::canvas::{FrameCanvas, InnerCanvas};
use aimer::style::LayoutSpacing;
use aimer::widget::base::WindowHandle;
use aimer::widget::layout_cache::probe;
use aimer::{
    AnyWidget, BoxAlignment, BuildContext, Color, Column, Container, Drawable, LayoutElement,
    ResolvedSize, Text, Vec2d, Widget,
};
use aimer::{ScrollAxis, ScrollController, Scrollable};
use aimer_attribute::BoxConstraint;

const VIEWPORT_W: f32 = 400.0;
const VIEWPORT_H: f32 = 800.0;

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

/// One list row: a padded container wrapping a nested container and a label —
/// the shape every real row in the showcase has.
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

/// A row carrying a realistic paragraph rather than a six-character label.
fn text_row(index: usize) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::new().left(12).right(12))
        .color(Color::Rgba(240, 240, 240, 255))
        .child(Text::new(format!(
            "Row {index}: the quick brown fox jumps over the lazy dog, and then \
             keeps running until the paragraph is long enough to require real \
             shaping and line breaking work from the text engine."
        )))
        .boxed()
}

#[test]
fn frame_cost_while_scrolling() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime");

    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("profile = {profile}");
    println!(
        "{:>22}  {:>10}  {:>12}  {:>26}",
        "case", "us/draw", "draws/16ms", "cache per draw"
    );

    const ROWS: usize = 500;
    const ITERS: u32 = 100;

    let bench = |label: &str, scrolling: bool, make: fn(usize) -> AnyWidget| {
        let inner = InnerCanvas::new();
        let canvas = FrameCanvas::new(&inner);
        let context = context(canvas, &runtime);

        let controller = ScrollController::new();
        let element = Container::new()
            .color(Color::Rgba(255, 255, 255, 255))
            .child(
                Scrollable::new()
                    .axis(ScrollAxis::Vertical)
                    .controller(controller.clone())
                    .child(
                        Column::new()
                            .horizontal_alignment(BoxAlignment::Start)
                            .children((0..ROWS).map(make).collect::<Vec<AnyWidget>>()),
                    ),
            )
            .to_element(&context);

        element.layout(&context);
        element.update(&context);
        inner.take_draw_list();

        probe::take();
        let start = aimer_time::Instant::now();
        for i in 0..ITERS {
            if scrolling {
                // What a wheel/drag does every frame: move the offset.
                controller.jump_to(Vec2d {
                    x: 0.0,
                    y: i as f32 * 3.0,
                });
            }
            element.update(&context);
            inner.take_draw_list();
        }
        let elapsed = start.elapsed();
        let (hits, misses, sets) = probe::take();

        let us = elapsed.as_secs_f64() * 1e6 / ITERS as f64;
        println!(
            "{:>22}  {:>10.1}  {:>12.0}  {:>7}h {:>7}m {:>7}sets",
            label,
            us,
            16_667.0 / us,
            hits / ITERS as u64,
            misses / ITERS as u64,
            sets / ITERS as u64,
        );
    };

    bench("short label, idle", false, row);
    bench("short label, scrolling", true, row);
    bench("paragraph, idle", false, text_row);
    bench("paragraph, scrolling", true, text_row);
}
