//! Measures what a windowed list costs the render-tree synchronization when a
//! row scrolls in or out.
//!
//! Run with
//! `cargo nextest run -p aimer_quiver row_sync_profile --run-ignored only --no-capture`.
//! Debug build, on purpose: that is the build the framework is tuned for.

use std::time::Instant;

use super::*;

/// Leaves per row. With the row itself this is about the node count of a
/// RustForum topic card (55 render nodes per row in the device traces).
const LEAVES_PER_ROW: usize = 50;
const ROW_HEIGHT: f32 = 80.0;
const VIEWPORT: (f32, f32) = (400.0, 800.0);

fn row(width: f32) -> impl Widget {
    aimer_flex::Row::new().children((0..LEAVES_PER_ROW).map(move |index| V2PaintLeaf {
        position: Vec2d::ZERO,
        width: width / LEAVES_PER_ROW as f32,
        height: ROW_HEIGHT,
        color: if index % 2 == 0 { Color::RED } else { Color::BLUE },
        paints: Rc::new(Cell::new(0)),
    }))
}

/// One frame the way `draw_scoped` runs it: sync, update walk, sync again.
/// Returns the time spent in each of the two syncs and what the second added.
fn frame(
    root: &AnyElement,
    context: &BuildContext<'_>,
    window_tree: &mut WindowRenderTree,
) -> (f64, f64, usize, u64) {
    let root_id = root.id();
    root.rebuild_if_dirty(context);
    let _ = aimer_widget::take_unmapped_draws();
    let full_before = window_tree.sync_counts.full;
    let started = Instant::now();
    window_tree
        .sync(Some(root), context, (400, 800), context.scale)
        .expect("synchronize before drawing");
    let before_draw = started.elapsed().as_secs_f64() * 1e3;

    context.canvas.begin_frame();
    aimer_widget::begin_paint_frame(400, 800);
    let tree = window_tree.tree.clone();
    let element_nodes = window_tree.element_nodes.clone();
    aimer_widget::with_v2_render_tree_context(tree, element_nodes, Some(root_id), || {
        context.canvas.save();
        root.update(context);
        context.canvas.restore();
    });

    let started = Instant::now();
    let sync = window_tree
        .sync(Some(root), context, (400, 800), context.scale)
        .expect("synchronize after drawing");
    let after_draw = started.elapsed().as_secs_f64() * 1e3;
    let added = sync.added_render_nodes.len();
    if sync.refreshed && sync.added_nodes {
        record_new_local_v2_paints(&window_tree.tree, sync.added_render_nodes);
    }
    window_tree.settle_unmapped_draws();
    (
        before_draw,
        after_draw,
        added,
        window_tree.sync_counts.full - full_before,
    )
}

#[test]
#[ignore = "profile harness: run explicitly with --run-ignored only --no-capture"]
fn row_sync_profile_scrolling_a_windowed_list() {
    let canvas = aimer_canvas::InnerCanvas::new();
    let (_runtime, mut context) = build_context(&canvas, 1.0);
    context.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: VIEWPORT.0,
        max_height: VIEWPORT.1,
    };
    let controller = ScrollController::new();
    let rows = Column::new()
        .list(0..1_000_u32)
        .item_extent(Dimension::Px(ROW_HEIGHT))
        .builder(move |_| row(VIEWPORT.0));
    let root = Scrollable::new()
        .controller(controller.clone())
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(rows)
        .to_element(&context);
    let mut window_tree = WindowRenderTree::default();
    for _ in 0..4 {
        frame(&root, &context, &mut window_tree);
    }
    let nodes = window_tree.element_nodes.len();

    // (sync ms, nodes added) per frame, grouped by what the frame had to do.
    let mut added_rows = Vec::new();
    let mut full_without_adds = Vec::new();
    let mut no_full_sync = Vec::new();
    let mut moved = [0usize; 3];
    let mut sizes = Vec::new();
    for step in 1..=240 {
        controller.jump_to(Vec2d { x: 0.0, y: step as f32 * 10.0 });
        let generations = || {
            [
                aimer_widget::unscoped_element_tree_generation(),
                aimer_widget::retained_render_structure_generation(),
                aimer_widget::layout_invalidation_generation(),
            ]
        };
        let before_generations = generations();
        let live_before = aimer_widget::live_children_generation();
        let (before, after, added, full_syncs) = frame(&root, &context, &mut window_tree);
        for (slot, (a, b)) in moved.iter_mut().zip(before_generations.iter().zip(generations())) {
            *slot += usize::from(*a != b);
        }
        let cost = before + after;
        if full_syncs > 0 {
            sizes.push((
                step,
                window_tree.element_nodes.len(),
                added,
                aimer_widget::live_children_generation() - live_before,
            ));
        }
        match (full_syncs > 0, added > 0) {
            (_, true) => added_rows.push(cost),
            (true, false) => full_without_adds.push(cost),
            (false, false) => no_full_sync.push(cost),
        }
    }
    let median = |values: &[f64]| {
        let mut values = values.to_vec();
        values.sort_by(f64::total_cmp);
        values.get(values.len() / 2).copied().unwrap_or(0.0)
    };
    println!("frames in which [unscoped element tree, render structure, layout] generation moved: {moved:?}");
    println!("(step, tree nodes, added, live-children notifications) at every full sync: {sizes:?}");
    println!("tree {nodes} nodes, {LEAVES_PER_ROW}+1 per row, 240 frames of 10px");
    for (label, costs) in [
        ("frames that add nodes", &added_rows),
        ("full sync, nothing added (rows dropped or layout moved)", &full_without_adds),
        ("no full sync", &no_full_sync),
    ] {
        println!("{label:>58}: {:3} frames, median sync {:.3} ms", costs.len(), median(costs));
    }
}

/// Scrolling a windowed list reports a change to its live rows only when a row
/// really enters or leaves, and never twice in one frame.
///
/// The pass that prepares layout and the pass that paints used to disagree by
/// a row about which rows should be alive. Each frame the first dropped a row
/// and the second built it back, each reported it, and every report cost a
/// full render-tree synchronization although the tree ended the frame as it
/// began.
#[test]
fn scrolling_a_windowed_list_reports_each_row_change_once() {
    let canvas = aimer_canvas::InnerCanvas::new();
    let (_runtime, mut context) = build_context(&canvas, 1.0);
    context.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: VIEWPORT.0,
        max_height: VIEWPORT.1,
    };
    let controller = ScrollController::new();
    let rows = Column::new()
        .list(0..1_000_u32)
        .item_extent(Dimension::Px(ROW_HEIGHT))
        .builder(move |_| row(VIEWPORT.0));
    let root = Scrollable::new()
        .controller(controller.clone())
        .vertical_scroll_bar(None)
        .horizontal_scroll_bar(None)
        .child(rows)
        .to_element(&context);
    let mut window_tree = WindowRenderTree::default();
    for _ in 0..4 {
        frame(&root, &context, &mut window_tree);
    }

    // 240 frames of 10 px pass 30 rows, so rows enter and leave about 60 times.
    let mut twice_in_one_frame = 0;
    let mut frames_with_a_report = 0;
    let mut full_syncs_without_a_report = 0;
    for step in 1..=240 {
        controller.jump_to(Vec2d { x: 0.0, y: step as f32 * 10.0 });
        let before = aimer_widget::live_children_generation();
        let (_, _, _, full_syncs) = frame(&root, &context, &mut window_tree);
        let reports = aimer_widget::live_children_generation() - before;
        twice_in_one_frame += usize::from(reports > 1);
        frames_with_a_report += usize::from(reports > 0);
        full_syncs_without_a_report += usize::from(reports == 0 && full_syncs > 0);
    }

    assert_eq!(twice_in_one_frame, 0, "a frame reported the live rows twice");
    assert_eq!(
        full_syncs_without_a_report, 0,
        "a frame synchronized the whole tree although no row changed"
    );
    assert!(
        frames_with_a_report <= 70,
        "{frames_with_a_report} of 240 frames reported a change for about 60 row moves"
    );
}
