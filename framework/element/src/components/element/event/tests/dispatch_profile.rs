//! Where the time of routing an event goes, one phase at a time.
//!
//! The application-level harness (`app/jaime`, `event_cost_profile`) can only say
//! how long a whole frame takes. This one lives beside the dispatcher so it can
//! call the phases of routing on their own:
//!
//! * `structural rebuild`: path index plus event tree, built in full
//!   (what a changed tree costs, once per generation);
//! * `event tree build`: the event tree alone;
//! * `bounds refresh`: what the first event after a layout change pays, which
//!   rewrites every target's bounds (`synchronize_event_tree`);
//! * `index hit_test`: the spatial index query by itself;
//! * `routed`: `dispatch_routed_event`, the mark pass and the dispatch pass, with
//!   no synchronisation around them;
//! * `dispatch`: `EventDispatcher::dispatch`, which adds path/focus
//!   synchronisation, hover transitions and focus settling;
//! * `scroll frame`: what a scrolling frame asks of routing, a layout change, a
//!   `Scroll` and the `PointerMove` the scroll container sends afterwards.
//!
//! The tree is a windowed list as a scroll container presents it: a hit-test
//! boundary (like `Flex`) over `ROWS` rows, each row an event target that holds
//! one transparent wrapper and one leaf target. Row bounds are read from a shared
//! scroll offset, so a scroll is one number changed plus
//! `notify_layout_changed()`, exactly the work a real frame hands routing.
//!
//! Counters report what the walk touched per event: elements visited, handler
//! calls, how many of those handler calls ignored the event, and how many times
//! the boundary was asked for its hit children. They are the figures that say
//! whether an interest filter would save calls.
//!
//! Run unoptimised, one test at a time, nothing else running:
//!
//! ```text
//! CARGO_TARGET_DIR="agents/target" cargo nextest run -p aimer_element \
//!     dispatch_phase_profile --run-ignored only --no-capture
//! ```
//!
//! `ROWS=100,1000` sizes the lists, `ITERS=<n>` is the timed iterations per
//! round (default 300), `ROUNDS=<n>` the rounds (default 7); the figure printed is
//! the median of the per-round means. It prints and asserts only that the walk
//! did what the scenario says it does.
//!
//! Limits: the boundary here finds its row with a division, where `RawFlex` reads
//! a retained sparse index, so the boundary's own cost is a lower bound; handlers
//! do no real work; times are one thread, debug, headless.

use std::time::Instant;

use aimer_events::element::{ScrollDeltaKind, TouchPhase};
use aimer_events::pointer::PointerButton;

use super::*;
use crate::components::element::notify_layout_changed;

const ROW_HEIGHT: f32 = 40.0;
const WIDTH: f32 = 300.0;
const VIEWPORT_ROWS: usize = 12;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_rows() -> Vec<usize> {
    std::env::var("ROWS")
        .ok()
        .map(|rows| rows.split(',').filter_map(|row| row.trim().parse().ok()).collect())
        .filter(|rows: &Vec<usize>| !rows.is_empty())
        .unwrap_or_else(|| vec![100, 1000])
}

/// What the walk touched, shared by every element of one tree.
#[derive(Default)]
struct Counters {
    handler_calls: Cell<u64>,
    ignored: Cell<u64>,
    boundary_queries: Cell<u64>,
}

impl Counters {
    fn reset(&self) {
        self.handler_calls.set(0);
        self.ignored.set(0);
        self.boundary_queries.set(0);
    }

    fn note(&self, result: EventResult) -> EventResult {
        self.handler_calls.set(self.handler_calls.get() + 1);
        if !result.is_consumed() {
            self.ignored.set(self.ignored.get() + 1);
        }
        result
    }
}

/// The scroll offset every row reads its position from.
type Offset = Rc<Cell<f32>>;

fn row_bounds(index: usize, offset: f32) -> (Vec2d, Vec2d) {
    let top = index as f32 * ROW_HEIGHT + offset;
    (Vec2d { x: 0.0, y: top }, Vec2d { x: WIDTH, y: top + ROW_HEIGHT })
}

/// The scroll container's content: a hit-test boundary over the rows that
/// answers `hit_test_children_at` with the one row under the pointer.
struct ListBoundary {
    rows: Vec<AnyElement>,
    offset: Offset,
    counters: Rc<Counters>,
}

impl VisitorElement for ListBoundary {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        for row in &self.rows {
            visitor(row.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "ListBoundary"
    }

    fn hit_test_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.counters.boundary_queries.set(self.counters.boundary_queries.get() + 1);
        let row = ((pos.y - self.offset.get()) / ROW_HEIGHT).floor();
        if row >= 0.0
            && let Some(row) = self.rows.get(row as usize)
        {
            visitor(row.as_ref());
        }
    }
}

impl EventElement for ListBoundary {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedHitTestBoundary
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        // The scroll container takes the wheel; everything else passes through.
        self.counters.note(match event {
            ElementEvent::Scroll { .. } => EventResult::consumed(),
            _ => EventResult::ignored(),
        })
    }
}

impl LayoutElement for ListBoundary {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some((
            Vec2d { x: 0.0, y: 0.0 },
            Vec2d { x: WIDTH, y: VIEWPORT_ROWS as f32 * ROW_HEIGHT },
        ))
    }
}

impl Drawable for ListBoundary {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for ListBoundary {}

/// A row: an event target that follows the scroll offset and, like a hover
/// region, can be made to take the pointer moves that reach it.
struct Row {
    index: usize,
    offset: Offset,
    child: AnyElement,
    consume_moves: bool,
    bounded: bool,
    counters: Rc<Counters>,
}

impl VisitorElement for Row {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Row"
    }
}

impl EventElement for Row {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.counters.note(match event {
            ElementEvent::PointerMove(_) if self.consume_moves => EventResult::consumed(),
            _ => EventResult::ignored(),
        })
    }
}

impl LayoutElement for Row {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(row_bounds(self.index, self.offset.get()))
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        // Content of a scroll container moves while painting without a layout
        // change, so it publishes no bounds (`RawScrollableContainer` contract).
        self.bounded.then(|| row_bounds(self.index, self.offset.get()))
    }
}

impl Drawable for Row {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for Row {}

/// A wrapper that handles nothing, as `Padding` or `Align` do.
struct Wrapper {
    child: AnyElement,
}

impl VisitorElement for Wrapper {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Wrapper"
    }
}

impl EventElement for Wrapper {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl LayoutElement for Wrapper {}

impl Drawable for Wrapper {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for Wrapper {}

/// The innermost target of a row, the label or icon a handler may own.
struct Leaf {
    index: usize,
    offset: Offset,
    consume_moves: bool,
    bounded: bool,
    counters: Rc<Counters>,
}

impl VisitorElement for Leaf {
    fn visit_children<'a>(&'a self, _visitor: &mut dyn FnMut(&'a dyn Element)) {}

    fn debug_name(&self) -> &'static str {
        "Leaf"
    }
}

impl EventElement for Leaf {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.counters.note(match event {
            ElementEvent::PointerMove(_) if self.consume_moves => EventResult::consumed(),
            _ => EventResult::ignored(),
        })
    }
}

impl LayoutElement for Leaf {
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        Some(row_bounds(self.index, self.offset.get()))
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        // Content of a scroll container moves while painting without a layout
        // change, so it publishes no bounds (`RawScrollableContainer` contract).
        self.bounded.then(|| row_bounds(self.index, self.offset.get()))
    }
}

impl Drawable for Leaf {
    fn update(&self, _ctx: &BuildContext) {}
}

impl Rebuildable for Leaf {}

struct Fixture {
    root: AnyElement,
    offset: Offset,
    counters: Rc<Counters>,
    rows: usize,
}

/// `consume_moves` makes the leaf of every row take pointer moves, as the
/// cursor-icon regions of the sidebar do; otherwise nothing consumes a move.
/// `bounded` publishes row bounds to the event tree; scroll content does not, so
/// `false` is the shape of a real scrolling list and `true` that of a static one.
fn fixture(rows: usize, consume_moves: bool, bounded: bool) -> Fixture {
    let offset: Offset = Rc::new(Cell::new(0.0));
    let counters = Rc::new(Counters::default());
    let children = (0..rows)
        .map(|index| {
            let leaf = Leaf {
                index,
                offset: offset.clone(),
                consume_moves,
                bounded,
                counters: counters.clone(),
            }
            .boxed();
            Row {
                index,
                offset: offset.clone(),
                child: Wrapper { child: leaf }.boxed(),
                consume_moves: false,
                bounded,
                counters: counters.clone(),
            }
            .boxed()
        })
        .collect();
    let root = ListBoundary { rows: children, offset: offset.clone(), counters: counters.clone() }
        .boxed();
    Fixture { root, offset, counters, rows }
}

fn mouse_move(pos: Vec2d) -> ElementEvent {
    ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary))
}

fn wheel() -> ElementEvent {
    ElementEvent::Scroll {
        delta: Vec2d { x: 0.0, y: -3.0 },
        phase: TouchPhase::Moved,
        kind: ScrollDeltaKind::Line,
        is_direct_manipulation: false,
    }
}

/// A point over the middle of visible row `visible`.
fn over_row(visible: usize) -> Vec2d {
    Vec2d { x: WIDTH * 0.5, y: (visible % VIEWPORT_ROWS) as f32 * ROW_HEIGHT + ROW_HEIGHT * 0.5 }
}

/// The median of the per-round mean times of `body`, in microseconds.
fn measure(rounds: usize, iters: usize, mut body: impl FnMut(usize)) -> f64 {
    for warm in 0..iters.min(50) {
        body(warm);
    }
    let mut means: Vec<f64> = (0..rounds)
        .map(|round| {
            let start = Instant::now();
            for iteration in 0..iters {
                body(round * iters + iteration);
            }
            start.elapsed().as_secs_f64() * 1e6 / iters as f64
        })
        .collect();
    means.sort_by(|left, right| left.total_cmp(right));
    means[means.len() / 2]
}

fn row_line(label: &str, micros: f64) {
    eprintln!("  {label:<34} {micros:>10.2} us");
}

/// What one event of a kind touched, from a single run after the tree is warm.
fn touched(
    fixture: &Fixture,
    dispatcher: &mut EventDispatcher,
    event: &ElementEvent,
    pos: Vec2d,
) -> (u64, u64, u64) {
    fixture.counters.reset();
    let _ = dispatcher.dispatch(fixture.root.as_ref(), pos, event);
    (
        fixture.counters.handler_calls.get(),
        fixture.counters.ignored.get(),
        fixture.counters.boundary_queries.get(),
    )
}

fn profile_rows(rows: usize, rounds: usize, iters: usize) {
    eprintln!("\n=== {rows} rows, {} event targets ===", rows * 2 + 1);

    // A static list publishes its row bounds. What a changed tree or a resized
    // window costs routing is read off this one.
    let bounded = fixture(rows, false, true);
    let mut dispatcher = EventDispatcher::new();
    dispatcher.set_index_patching(false);
    // The first dispatch builds the indexes; every figure below is steady state.
    let _ = dispatcher.dispatch(bounded.root.as_ref(), over_row(0), &wheel());
    eprintln!("-- static list (row bounds published)");

    // Forgetting the root address is what makes `synchronize_path_index` rebuild:
    // setting `paths_dirty` alone is overwritten by the generation comparison.
    let rebuild = measure(rounds, iters / 4 + 1, |_| {
        dispatcher.indexed_root_address = None;
        dispatcher.synchronize_paths(bounded.root.as_ref());
        assert!(!dispatcher.event_tree.elements().is_empty());
    });
    let (full_rebuilds, _) = dispatcher.path_index_work();
    assert!(full_rebuilds > 1, "the rebuild measurement must rebuild");
    row_line("structural rebuild (index + tree)", rebuild);

    let build = measure(rounds, iters / 4 + 1, |_| {
        dispatcher.event_tree_stale = true;
        dispatcher.synchronize_event_tree(bounded.root.as_ref());
    });
    row_line("event tree build only", build);

    // What the first event after any layout change pays. A scroll is not one:
    // see the scroll frame below.
    let refresh = measure(rounds, iters, |_| {
        notify_layout_changed();
        dispatcher.synchronize_event_tree(bounded.root.as_ref());
    });
    row_line("bounds refresh (layout changed)", refresh);

    let hit = measure(rounds, iters * 10, |step| {
        let pos = over_row(step);
        let count = dispatcher.event_tree.hit_test(pos.x, pos.y).count();
        assert!(count > 0, "the index must find the row under the pointer");
    });
    row_line("index hit_test (bounded targets)", hit);

    // A scrolling list: its content publishes no bounds, so every row is a
    // candidate the spatial index cannot reject and the boundary does the pruning.
    eprintln!("-- scrolling list (row bounds not published)");
    let plain = fixture(rows, false, false);
    let hovering = fixture(rows, true, false);
    let mut dispatcher = EventDispatcher::new();
    dispatcher.set_index_patching(false);
    let _ = dispatcher.dispatch(plain.root.as_ref(), over_row(0), &wheel());

    let hit = measure(rounds, iters * 10, |step| {
        let pos = over_row(step);
        let count = dispatcher.event_tree.hit_test(pos.x, pos.y).count();
        assert!(count >= rows, "unbounded targets cannot be rejected by the index");
    });
    row_line("index hit_test (unbounded targets)", hit);

    let scroll = wheel();
    let routed_scroll = measure(rounds, iters, |step| {
        let outcome = dispatch_routed_event(
            &mut dispatcher,
            plain.root.as_ref(),
            over_row(step),
            &scroll,
        );
        assert!(outcome.result.is_consumed(), "the boundary takes the wheel");
    });
    row_line("routed Scroll", routed_scroll);
    let full_scroll = measure(rounds, iters, |step| {
        let _ = dispatcher.dispatch(plain.root.as_ref(), over_row(step), &scroll);
    });
    row_line("dispatch Scroll", full_scroll);
    row_line("  route overhead around it", full_scroll - routed_scroll);

    for (name, fixture) in [("ignored moves", &plain), ("consumed moves", &hovering)] {
        let mut dispatcher = EventDispatcher::new();
        dispatcher.set_index_patching(false);
        let _ = dispatcher.dispatch(fixture.root.as_ref(), over_row(0), &mouse_move(over_row(0)));
        // The pointer alternates between two rows, so the hover chain changes
        // every event, as it does while rows pass under a resting cursor.
        let moved = measure(rounds, iters, |step| {
            let pos = over_row(step % 2);
            let _ = dispatcher.dispatch(fixture.root.as_ref(), pos, &mouse_move(pos));
        });
        row_line(&format!("dispatch PointerMove ({name})"), moved);
        let routed = measure(rounds, iters, |step| {
            let pos = over_row(step % 2);
            let _ = dispatch_routed_event(
                &mut dispatcher,
                fixture.root.as_ref(),
                pos,
                &mouse_move(pos),
            );
        });
        row_line(&format!("  routed only ({name})"), routed);
    }

    // What a scrolling frame asks of routing: the offset moved, the wheel is
    // delivered, then the scroll container re-sends the pointer position. No
    // layout generation advances, which is checked: the real app does not
    // advance it either (`CLEAN_SKIP_PROBE` in app/jaime's event_cost_profile).
    let mut dispatcher = EventDispatcher::new();
    dispatcher.set_index_patching(false);
    let _ = dispatcher.dispatch(hovering.root.as_ref(), over_row(0), &wheel());
    let layout_before = crate::components::element::layout_invalidation_generation();
    let rebuilds_before = dispatcher.path_index_work();
    let frame = measure(rounds, iters, |step| {
        hovering.offset.set(-((step % 200) as f32) * 3.0);
        let pos = over_row(3);
        let _ = dispatcher.dispatch(hovering.root.as_ref(), pos, &wheel());
        let _ = dispatcher.dispatch(hovering.root.as_ref(), pos, &mouse_move(pos));
    });
    assert_eq!(
        layout_before,
        crate::components::element::layout_invalidation_generation(),
        "a scroll must not advance the layout generation",
    );
    assert_eq!(rebuilds_before, dispatcher.path_index_work(), "a scroll must not rebuild");
    row_line("scroll frame (Scroll + PointerMove)", frame);

    // What a walk touches per event.
    hovering.offset.set(0.0);
    let pos = over_row(3);
    let (calls, ignored, queries) = touched(&hovering, &mut dispatcher, &wheel(), pos);
    eprintln!("  Scroll:      {calls} handler calls, {ignored} ignored, {queries} boundary queries");
    let (calls, ignored, queries) =
        touched(&hovering, &mut dispatcher, &mouse_move(pos), pos);
    eprintln!("  PointerMove: {calls} handler calls, {ignored} ignored, {queries} boundary queries");
    assert!(calls > 0, "a move over a row must reach a handler");
    assert_eq!(hovering.rows, rows);
}


/// Prints the cost of each phase of routing for lists of increasing size.
#[test]
#[ignore = "profiling harness; run alone with --run-ignored only --no-capture"]
fn dispatch_phase_profile() {
    let rounds = env_usize("ROUNDS", 7);
    let iters = env_usize("ITERS", 300);
    for rows in env_rows() {
        profile_rows(rows, rounds, iters);
    }
}
