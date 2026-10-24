//! Phase-specific measurements for the retained framework tree.
//!
//! The regular framework baseline reports complete operations. This executable
//! keeps the same representative sizes but isolates the work that a complete
//! frame combines: widget materialization, layout invalidation and measurement,
//! painting, dirty reconciliation, hit testing, event delivery, and focus
//! traversal.
//!
//! Run the profile with:
//!
//! ```text
//! cargo run -p aimer_laboratory --example framework_phase_profile --release
//! ```
//!
//! The phase boundaries are deliberate. `draw (cached frame)` includes the
//! production `Drawable::update` call, which performs its normal clean rebuild
//! check and cached layout reads. `reconcile (dirty tree)` marks the tree before
//! the timer starts, because dirty marking normally happens during input or a
//! state update before the next frame. Hit testing is a pure structural walk;
//! `pointer dispatch` is the full production dispatcher path and is reported
//! separately so the two costs are not confused.

use std::hint::black_box;
use std::time::Instant;

use aimer::events::element::ElementEvent;
use aimer::events::pointer::PointerButton;
use aimer::events::pointer::PointerInfo;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::{
    AnyElement, AnyWidget, BuildContext, Color, Column, Drawable, Element,
    EventDispatcher, EventElement, LayoutElement, Rebuildable, SizedBox, StatelessElement, Vec2d,
    VisitorElement, Widget, broadcast_event,
};
use aimer_canvas::{FrameCanvas, InnerCanvas};
use aimer_widget::base::WindowHandle;

const ROUNDS: usize = 7;
const WARMUP_OPERATIONS: usize = 4;
const MEASURED_OPERATIONS: usize = 32;
const FRAME_WIDTH: f32 = 1_150.0;
const FRAME_HEIGHT: f32 = 800.0;
const TREE_SIZES: [usize; 3] = [32, 256, 2_048];

fn row(index: usize) -> AnyWidget {
    let height = 20.0 + (index % 5) as f32;
    let color = if index % 2 == 0 {
        Color::WHITE
    } else {
        Color::BLACK
    };
    aimer::Container::new()
        .color(color)
        .box_child(SizedBox::new().width(800.0).height(height))
}

fn phase_widget(count: usize) -> AnyWidget {
    Column::new().children((0..count).map(row)).boxed()
}

fn context(runtime: &tokio::runtime::Runtime) -> BuildContext<'static> {
    let inner = Box::leak(Box::new(InnerCanvas::new()));
    let mut context = BuildContext::new(
        FrameCanvas::new(inner),
        aimer::ResolvedSize {
            width: FRAME_WIDTH,
            height: FRAME_HEIGHT,
        },
        1.0,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(
            PhysicalSize::new(FRAME_WIDTH as u32, FRAME_HEIGHT as u32),
            1.0,
        ),
        runtime.handle().clone(),
    );
    context.box_constraint.max_width = FRAME_WIDTH;
    context.box_constraint.max_height = FRAME_HEIGHT;
    context
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("phase profile runtime")
}

#[derive(Default)]
struct Samples {
    values: Vec<f64>,
}

impl Samples {
    fn record(&mut self, start: Instant) {
        self.values.push(start.elapsed().as_secs_f64() * 1e6);
    }

    fn print(&mut self, name: &str) {
        self.values.sort_by(f64::total_cmp);
        let p50 = percentile(&self.values, 0.50);
        let p95 = percentile(&self.values, 0.95);
        println!("{name:<42} p50 {:>9.2} us  p95 {:>9.2} us", p50, p95);
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

fn build_tree(
    count: usize,
    runtime: &tokio::runtime::Runtime,
) -> (BuildContext<'static>, AnyElement) {
    let context = context(runtime);
    let root = phase_widget(count).into_element(&context);
    (context, root)
}

fn measure_materialization(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let context = context(runtime);
        let start = Instant::now();
        let root = phase_widget(count).into_element(&context);
        black_box(root.id());
        samples.record(start);
    }
    samples.print(&format!("{count} nodes: materialization (cold)"));
}

fn measure_layout(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let (context, root) = build_tree(count, runtime);
        for _ in 0..WARMUP_OPERATIONS {
            root.invalidate_layout();
            black_box(root.layout(&context));
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            root.invalidate_layout();
            black_box(root.layout(&context));
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: layout (invalidate + measure)"));
}

fn measure_layout_invalidation(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let (_, root) = build_tree(count, runtime);
        for _ in 0..WARMUP_OPERATIONS {
            root.invalidate_layout();
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            root.invalidate_layout();
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: layout invalidation (marker)"));
}

fn measure_draw(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let (context, root) = build_tree(count, runtime);
        context.canvas.begin_frame();
        root.update(&context);
        for _ in 0..WARMUP_OPERATIONS {
            context.canvas.begin_frame();
            root.update(&context);
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            black_box(&root);
            context.canvas.begin_frame();
            root.update(&context);
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: draw (cached frame)"));
}

fn measure_reconciliation(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let context = context(runtime);
        let root = StatelessElement::from_builder(
            &context,
            move |context| phase_widget(count).into_element(context),
            None,
            "PhaseProfileRoot",
        )
        .boxed();
        root.rebuild_if_dirty(&context);

        for _ in 0..WARMUP_OPERATIONS {
            root.mark_needs_rebuild();
            root.rebuild_if_dirty(&context);
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            root.mark_needs_rebuild();
            root.rebuild_if_dirty(&context);
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: reconcile (dirty tree)"));
}

fn measure_clean_reconciliation(count: usize, runtime: &tokio::runtime::Runtime) {
    let mut gated_samples = Samples::default();
    let mut ungated_samples = Samples::default();
    for _ in 0..ROUNDS {
        let context = context(runtime);
        let (child, _) = traversal_tree(count, false);
        let gated = StatelessElement::wrapper(child, None, "CleanRebuildRoot").boxed();
        gated.rebuild_if_dirty(&context);

        let (ungated, _) = traversal_tree(count, false);
        ungated.rebuild_if_dirty(&context);

        for _ in 0..WARMUP_OPERATIONS {
            ungated.mark_needs_rebuild();
            ungated.rebuild_if_dirty(&context);
        }
        for _ in 0..WARMUP_OPERATIONS {
            gated.rebuild_if_dirty(&context);
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            gated.rebuild_if_dirty(&context);
        }
        gated_samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);

        let mut ungated_elapsed = 0.0;
        for _ in 0..MEASURED_OPERATIONS {
            // Dirty marking normally occurs before the frame. Keep it outside
            // the timer so this remains a retained-walk comparison.
            ungated.mark_needs_rebuild();
            let start = Instant::now();
            ungated.rebuild_if_dirty(&context);
            ungated_elapsed += start.elapsed().as_secs_f64() * 1e6;
        }
        ungated_samples
            .values
            .push(ungated_elapsed / MEASURED_OPERATIONS as f64);
    }
    gated_samples.print(&format!("{count} nodes: reconcile (clean, gated)"));
    ungated_samples.print(&format!("{count} nodes: reconcile (clean, old walk)"));
}

/// A no-op retained tree used to isolate the framework's structural event
/// seams from widget and canvas work.
struct TraversalElement {
    children: Vec<AnyElement>,
    node: Option<aimer::FocusNode>,
}

impl Drawable for TraversalElement {
    fn update(&self, _context: &BuildContext) {}
}

impl VisitorElement for TraversalElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "PhaseTraversalElement"
    }
}

impl EventElement for TraversalElement {
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in &self.children {
            visitor(child.as_ref());
        }
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for child in self.children.iter().rev() {
            visitor(child.as_ref());
        }
    }

    fn focus_node(&self) -> Option<&aimer::FocusNode> {
        self.node.as_ref()
    }
}

impl LayoutElement for TraversalElement {}
impl Rebuildable for TraversalElement {}

fn traversal_tree(count: usize, focusable: bool) -> (AnyElement, Option<aimer::FocusNode>) {
    let mut first_node = None;
    let children = (0..count)
        .map(|index| {
            let node = focusable.then(aimer::FocusNode::new);
            if index == 0 {
                first_node = node.clone();
            }
            TraversalElement {
                children: Vec::new(),
                node,
            }
            .boxed()
        })
        .collect();

    (
        TraversalElement {
            children,
            node: None,
        }
        .boxed(),
        first_node,
    )
}

fn contains(element: &dyn Element, pos: Vec2d) -> bool {
    element.pos_start_end().is_none_or(|(start, end)| {
        pos.x >= start.x && pos.x <= end.x && pos.y >= start.y && pos.y <= end.y
    })
}

fn hit_test_only(element: &dyn Element, pos: Vec2d, visited: &mut usize) {
    if !contains(element, pos) {
        return;
    }
    *visited += 1;
    element.hit_test_children_at(pos, &mut |child| hit_test_only(child, pos, visited));
}

fn measure_hit_testing(count: usize) {
    let mut samples = Samples::default();
    let pos = Vec2d::default();
    for _ in 0..ROUNDS {
        let (root, _) = traversal_tree(count, false);
        for _ in 0..WARMUP_OPERATIONS {
            let mut visited = 0;
            hit_test_only(root.as_ref(), pos, &mut visited);
            black_box(visited);
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            let mut visited = 0;
            hit_test_only(root.as_ref(), pos, &mut visited);
            black_box(visited);
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: hit testing (pure walk)"));
}

fn pointer_event() -> ElementEvent {
    ElementEvent::PointerMove(PointerInfo::mouse(Vec2d::default(), PointerButton::Primary))
}

fn measure_event_dispatch(count: usize) {
    let mut samples = Samples::default();
    for _ in 0..ROUNDS {
        let (root, _) = traversal_tree(count, false);
        let event = pointer_event();
        let mut dispatcher = EventDispatcher::new();
        for _ in 0..WARMUP_OPERATIONS {
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: pointer dispatch (production)"));
}

fn measure_event_delivery(count: usize, retained: bool) {
    let mut samples = Samples::default();
    let event = pointer_event();
    for _ in 0..ROUNDS {
        let (root, _) = traversal_tree(count, false);
        let mut dispatcher = EventDispatcher::new();
        let mut broadcast = || {
            if retained {
                dispatcher.broadcast(root.as_ref(), &event)
            } else {
                broadcast_event(root.as_ref(), &event)
            }
        };
        for _ in 0..WARMUP_OPERATIONS {
            let _ = black_box(broadcast());
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            let _ = black_box(broadcast());
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    let path = if retained {
        "cached broadcast"
    } else {
        "broadcast"
    };
    samples.print(&format!("{count} nodes: event delivery ({path})"));
}

fn measure_focus(count: usize) {
    let mut samples = Samples::default();
    let event = ElementEvent::KeyInput {
        key: aimer::events::element::NamedKey::Tab,
        action: aimer::events::element::KeyAction::Pressed,
        modifiers: aimer::events::element::Modifiers::default(),
    };
    for _ in 0..ROUNDS {
        let (root, first_node) = traversal_tree(count, true);
        let mut dispatcher = EventDispatcher::new();
        if let Some(node) = first_node {
            node.request_focus();
            let _ = black_box(dispatcher.dispatch(
                root.as_ref(),
                Vec2d::default(),
                &ElementEvent::Cancel,
            ));
        }
        for _ in 0..WARMUP_OPERATIONS {
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }

        let start = Instant::now();
        for _ in 0..MEASURED_OPERATIONS {
            let _ = black_box(dispatcher.dispatch(root.as_ref(), Vec2d::default(), &event));
        }
        samples
            .values
            .push(start.elapsed().as_secs_f64() * 1e6 / MEASURED_OPERATIONS as f64);
    }
    samples.print(&format!("{count} nodes: focus traversal (Tab)"));
}

fn main() {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let runtime = runtime();
    let _runtime_guard = runtime.enter();

    println!(
        "framework phase profile ({profile} profile, {ROUNDS} rounds, \
         {MEASURED_OPERATIONS} measured operations)"
    );
    println!("phase                                      p50        p95");

    for count in TREE_SIZES {
        measure_layout_invalidation(count, &runtime);
        measure_materialization(count, &runtime);
        measure_layout(count, &runtime);
        measure_draw(count, &runtime);
        measure_reconciliation(count, &runtime);
        measure_clean_reconciliation(count, &runtime);
    }

    println!("\nstructural input phases");
    for count in TREE_SIZES {
        measure_hit_testing(count);
        measure_event_delivery(count, false);
        measure_event_delivery(count, true);
        measure_event_dispatch(count);
        measure_focus(count);
    }
}
