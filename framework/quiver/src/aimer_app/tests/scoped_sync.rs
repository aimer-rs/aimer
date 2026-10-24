//! Re-synchronizing only the subtree a rebuild replaced leaves the render tree
//! exactly as a full synchronization would, and takes the cheap path only when
//! that is true.
//!
//! Every test runs one page in two applications, one with scoped
//! synchronization turned off, applies the same state changes to both, and
//! compares the bounds of every element after each step.

use super::*;

use std::cell::RefCell;

use aimer_widget::StateUpdater;

type Updaters = Rc<RefCell<Vec<StateUpdater<BoxState>>>>;

/// A box that rebuilds into one of several shapes when its mode changes.
struct StatefulBox {
    updaters: Updaters,
    nested: bool,
}

struct BoxState {
    mode: u8,
    updaters: Updaters,
    nested: bool,
}

impl StatefulWidget for StatefulBox {
    type State = BoxState;

    fn create_state(self) -> BoxState {
        BoxState {
            mode: 0,
            updaters: self.updaters,
            nested: self.nested,
        }
    }
}

impl State<StatefulBox> for BoxState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updaters.borrow_mut().push(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let swatch = |width: f32, shade: u8| {
            SizedBox::new()
                .width(Dimension::Px(width))
                .height(Dimension::Px(20.0))
                .color(Color::Rgb(shade, 90, 90))
                .boxed()
        };
        match self.mode {
            // The same shape with different paint.
            0 => swatch(40.0, 20),
            1 => swatch(40.0, 200),
            // The same elements, a different size.
            2 => swatch(80.0, 20),
            // More elements than before.
            3 => Column::new()
                .children(vec![swatch(40.0, 20), swatch(40.0, 60)])
                .boxed(),
            // Two columns of the same shape whose first swatch differs in colour:
            // the change is below the rebuilt root, not in it.
            5 => Column::new()
                .children(vec![swatch(40.0, 30), swatch(40.0, 60)])
                .boxed(),
            6 => Column::new()
                .children(vec![swatch(40.0, 220), swatch(40.0, 60)])
                .boxed(),
            // A stateful element inside the stateful element.
            _ if self.nested => StatefulBox {
                updaters: self.updaters.clone(),
                nested: false,
            }
            .boxed(),
            _ => swatch(40.0, 120),
        }
    }
}

impl Widget for StatefulBox {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ScopedBox", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for StatefulBox {}

struct Page {
    updaters: Updaters,
}

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let boxed = |nested| {
            StatefulBox {
                updaters: self.updaters.clone(),
                nested,
            }
            .boxed()
        };
        let blue = || {
            SizedBox::new()
                .width(Dimension::Px(30.0))
                .height(Dimension::Px(20.0))
                .color(Color::BLUE)
                .boxed()
        };
        let mut rows = vec![
            boxed(false),
            Row::new()
                .children(vec![blue(), boxed(false), blue()])
                .boxed(),
            boxed(true),
        ];
        rows.extend((0..20).map(|_| {
            SizedBox::new()
                .width(Dimension::Px(120.0))
                .height(Dimension::Px(8.0))
                .color(Color::WHITE)
                .boxed()
        }));
        Column::new().children(rows).to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Page {}

/// One application showing the page.
struct Scene {
    app: HeadlessAimerApp<aimer_modal::ModalHost<Page>>,
    updaters: Updaters,
}

impl Scene {
    fn new(scoped_sync_disabled: bool) -> Self {
        let updaters: Updaters = Rc::default();
        let mut app = AimerApp::start_headless_with(
            Page {
                updaters: updaters.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(240, 300),
                scale_factor: 1.0,
            },
        );
        app.app.render_tree.scoped_sync_disabled = scoped_sync_disabled;
        app.pump_frames(3);
        Self { app, updaters }
    }

    /// Changes the mode of the boxes at `indices` (creation order) and renders.
    ///
    /// The updates are made by a task, as a timer or a finished request would
    /// make them. A synchronous `set_state` keeps its ordinary full frame.
    fn set_modes(&mut self, changes: &[(usize, u8)]) {
        let venus = self.app.venus().clone();
        for &(index, mode) in changes {
            let updater = self.updaters.borrow()[index];
            venus.spawn(async move { updater.set_state(move |state| state.mode = mode) });
        }
        // The scheduler's wake: a task is ready, and nothing else is asking.
        aimer_events::window::request_scoped_frame();
        self.app.pump_frames(3);
    }

    /// Every element of the mounted tree, in order, with the bounds of its node.
    fn layout(&self) -> Vec<(&'static str, Option<(f32, f32, f32, f32)>)> {
        fn walk(
            element: &dyn Element,
            app: &HeadlessAimerApp<aimer_modal::ModalHost<Page>>,
            out: &mut Vec<(&'static str, Option<(f32, f32, f32, f32)>)>,
        ) {
            let bounds = app
                .app
                .render_node_for_element(element.id())
                .and_then(|node| app.app.render_tree().element_bounds(node).ok())
                .map(|r| (r.x, r.y, r.width, r.height));
            out.push((element.debug_name(), bounds));
            element.visit_retained_v2_children(&mut |_, child| walk(child, app, out));
        }
        let mut out = Vec::new();
        walk(
            self.app.app.widget_root.as_ref().expect("mounted").as_ref(),
            &self.app,
            &mut out,
        );
        out
    }

    fn syncs(&self) -> (u64, u64) {
        let counts = self.app.app.render_tree.sync_counts;
        (counts.full, counts.scoped)
    }
}

type Layout = Vec<(&'static str, Option<(f32, f32, f32, f32)>)>;

/// Applies `steps` to one application and records its layout before the first
/// step and after each, with its (full, scoped) synchronization counts.
///
/// Applications must run one after the other: they share the thread's frame
/// requester, so a second one alive at the same time would receive the first
/// one's frame requests.
fn run(steps: &[&[(usize, u8)]], scoped_sync_disabled: bool) -> (Vec<Layout>, (u64, u64)) {
    let mut scene = Scene::new(scoped_sync_disabled);
    let before = scene.syncs();
    let mut layouts = vec![scene.layout()];
    for step in steps {
        scene.set_modes(step);
        layouts.push(scene.layout());
    }
    let after = scene.syncs();
    (layouts, (after.0 - before.0, after.1 - before.1))
}

/// Applies `steps` to a scoped and a full-only application, asserting that
/// they show the same layout after every step. Returns the scoped application's
/// (full, scoped) synchronization counts gained over the steps.
fn compare(steps: &[&[(usize, u8)]]) -> (u64, u64) {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (scoped_layouts, counts) = run(steps, false);
    let (full_layouts, reference) = run(steps, true);
    for (number, (scoped, full)) in scoped_layouts.iter().zip(&full_layouts).enumerate() {
        assert_eq!(
            scoped, full,
            "after step {number} the scoped application diverged"
        );
    }
    assert_eq!(
        reference.1, 0,
        "the reference application never takes the scoped path"
    );
    counts
}

#[test]
fn a_repaint_only_rebuild_is_synchronized_in_place() {
    // Mode 0 and 1 differ only in colour.
    let (full, scoped) = compare(&[&[(0, 1)], &[(0, 0)], &[(1, 1)], &[(1, 0)]]);
    assert!(
        scoped >= 4,
        "each rebuild takes the scoped path, saw {scoped} (full {full})"
    );
    assert_eq!(full, 0, "and none needs a full synchronization");
}

#[test]
fn a_rebuild_that_changes_size_falls_back_to_the_full_synchronization() {
    // Mode 2 is wider, so everything beside it moves.
    let (full, _) = compare(&[&[(1, 2)], &[(1, 0)]]);
    assert!(
        full >= 2,
        "the layout change must be fully synchronized, saw {full}"
    );
}

#[test]
fn a_rebuild_that_adds_elements_falls_back_to_the_full_synchronization() {
    let (full, _) = compare(&[&[(0, 3)], &[(0, 0)]]);
    assert!(
        full >= 2,
        "new nodes must be fully synchronized, saw {full}"
    );
}

#[test]
fn rebuilds_that_coincide_stay_correct() {
    compare(&[
        // Two boxes in one frame.
        &[(0, 1), (1, 1)],
        // Mixing shapes in one frame.
        &[(0, 2), (1, 0), (2, 3)],
        &[(0, 0), (2, 0)],
    ]);
}

#[test]
fn a_stateful_element_inside_a_rebuilt_one_stays_correct() {
    compare(&[
        // Box 2 grows a stateful element of its own, which gets box index 3.
        &[(2, 4)],
        // The outer box rebuilds again, keeping its inner one, in the same frame
        // as the inner one rebuilds.
        &[(2, 4), (3, 1)],
        &[(3, 0)],
        &[(3, 1), (2, 4)],
        // Leaving the nested mode removes the inner element.
        &[(2, 0)],
    ]);
}

/// A box whose child is a button, so the replaced subtree contains something
/// an event has to reach through the freshly patched index.
struct PressableBox {
    presses: Rc<Cell<usize>>,
    handle: Rc<RefCell<Option<StateUpdater<PressableState>>>>,
}

struct PressableState {
    shade: u8,
    presses: Rc<Cell<usize>>,
    handle: Rc<RefCell<Option<StateUpdater<PressableState>>>>,
}

impl StatefulWidget for PressableBox {
    type State = PressableState;

    fn create_state(self) -> PressableState {
        PressableState {
            shade: 0,
            presses: self.presses,
            handle: self.handle,
        }
    }
}

impl State<PressableBox> for PressableState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.handle.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let presses = self.presses.clone();
        aimer_input::button::Button::new()
            .on_press(move || presses.set(presses.get() + 1))
            .decoration(
                aimer_style::BoxDecoration::new().background_color(Color::Rgb(self.shade, 80, 80)),
            )
            .child(
                SizedBox::new()
                    .width(Dimension::Px(60.0))
                    .height(Dimension::Px(30.0)),
            )
    }
}

impl Widget for PressableBox {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "PressableBox", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for PressableBox {}

/// Presses after each of three rebuilds, plus how the path index was kept.
fn presses_after_rebuilds(index_patching: bool) -> (Vec<usize>, (u64, u64)) {
    let presses = Rc::new(Cell::new(0));
    let handle = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        PressableBox {
            presses: presses.clone(),
            handle: handle.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, 100),
            scale_factor: 1.0,
        },
    );
    app.app.event_dispatcher.set_index_patching(index_patching);
    app.pump_frames(3);
    let updater = handle.borrow().expect("the state mounted");
    let click = |app: &mut HeadlessAimerApp<_>| {
        // A rebuilt button starts unhovered, so the pointer arrives from outside
        // each time, as it would for a user.
        for position in [(150.0, 80.0), (20.0, 15.0)] {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(position.0, position.1),
            });
            app.pump_frames(1);
        }
        for state in [
            winit::event::ElementState::Pressed,
            winit::event::ElementState::Released,
        ] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: winit::event::MouseButton::Left,
            });
        }
        app.pump_frames(2);
    };
    click(&mut app);
    let mut counts = vec![presses.get()];
    for shade in [60u8, 120, 180, 240, 30, 90] {
        updater.set_state(move |state| state.shade = shade);
        app.pump_frames(3);
        click(&mut app);
        counts.push(presses.get());
    }
    (counts, app.app.event_dispatcher.path_index_work())
}

#[test]
fn presses_reach_a_button_in_a_patched_subtree_exactly_as_they_do_after_a_rebuild() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (rebuilt_counts, rebuilt_work) = presses_after_rebuilds(false);
    let (patched_counts, patched_work) = presses_after_rebuilds(true);

    assert_eq!(
        patched_counts, rebuilt_counts,
        "patching must not change which presses land"
    );
    assert!(
        patched_counts.last() > Some(&1),
        "the button must receive presses at all: {patched_counts:?}"
    );
    assert_eq!(rebuilt_work.1, 0, "the reference never patches");
    assert!(
        patched_work.1 >= 6,
        "each rebuild patched the index, saw {patched_work:?}"
    );
    assert_eq!(patched_work.0, 1, "and only the first build created it");
}

/// The (whole-tree walks, animation-only passes) this thread has run so far.
fn walks() -> (u64, u64) {
    aimer_widget::traversal_counts()
}

/// Like [`run`], with the walks and passes each step took.
fn run_counted(steps: &[&[(usize, u8)]], incremental: bool) -> (Vec<Layout>, Vec<(u64, u64)>) {
    let mut scene = Scene::new(!incremental);
    scene.app.set_incremental_sync(incremental);
    let mut layouts = vec![scene.layout()];
    let mut counts = Vec::new();
    for step in steps {
        let before = walks();
        scene.set_modes(step);
        let after = walks();
        counts.push((after.0 - before.0, after.1 - before.1));
        layouts.push(scene.layout());
    }
    (layouts, counts)
}

/// Runs `steps` scoped and unscoped, asserts the layouts agree, and returns the
/// scoped run's per-step (walks, passes).
fn compare_counted(steps: &[&[(usize, u8)]]) -> Vec<(u64, u64)> {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (scoped_layouts, counts) = run_counted(steps, true);
    let (full_layouts, reference) = run_counted(steps, false);
    for (number, (scoped, full)) in scoped_layouts.iter().zip(&full_layouts).enumerate() {
        assert_eq!(
            scoped, full,
            "after step {number} the scoped application diverged"
        );
    }
    // The reference walks the tree for every rebuild; the frame that follows a
    // state update with nothing left to do may still be an empty pass.
    assert!(
        reference.iter().all(|&(walks, _)| walks >= 1),
        "the reference walks every time: {reference:?}"
    );
    counts
}

#[test]
fn state_updates_after_the_first_revisit_only_the_rebuilt_subtree() {
    // Mode 0 and 1 differ only in colour, so the shape and size never change.
    let counts = compare_counted(&[&[(0, 1)], &[(0, 0)], &[(0, 1)], &[(0, 0)]]);

    assert!(
        counts[0].0 >= 1,
        "the first rebuild of an owner walks the tree: {counts:?}"
    );
    for (step, &(walks, passes)) in counts.iter().enumerate().skip(1) {
        assert!(
            walks == 0 && passes >= 1,
            "step {step} should be passes only, got {counts:?}"
        );
    }
}

#[test]
fn each_owner_is_captured_by_its_own_first_rebuild() {
    let counts = compare_counted(&[&[(0, 1)], &[(1, 1)], &[(0, 0)], &[(1, 0)]]);

    assert!(
        counts[0].0 >= 1 && counts[1].0 >= 1,
        "both first rebuilds are full walks: {counts:?}"
    );
    for step in [2, 3] {
        assert!(
            counts[step].0 == 0 && counts[step].1 >= 1,
            "then each owner is cheap: {counts:?}"
        );
    }
}

#[test]
fn a_frame_that_rebuilds_an_uncaptured_owner_walks_the_whole_tree() {
    // Box 0 is captured; box 1 has never rebuilt, so the frame cannot be scoped.
    let counts = compare_counted(&[&[(0, 1)], &[(0, 0)], &[(0, 1), (1, 1)]]);

    assert!(counts[1].0 == 0 && counts[1].1 >= 1, "{counts:?}");
    assert!(
        counts[2].0 >= 1,
        "an owner with no captured context needs the full walk: {counts:?}"
    );
}

#[test]
fn a_rebuild_that_changes_size_in_a_scoped_frame_falls_back() {
    let counts = compare_counted(&[&[(1, 1)], &[(1, 0)], &[(1, 2)], &[(1, 0)]]);

    assert!(counts[1].0 == 0 && counts[1].1 >= 1, "{counts:?}");
    assert!(
        counts[2].0 >= 1,
        "growing the box moves its neighbours: {counts:?}"
    );
}

#[test]
fn a_pointer_move_costs_one_full_walk_and_then_updates_are_cheap_again() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut scene = Scene::new(false);
    scene.set_modes(&[(0, 1)]);
    let before = walks();
    scene.set_modes(&[(0, 0)]);
    assert_eq!(walks().0, before.0, "the second rebuild is scoped");

    // Elements read the pointer while they are updated, so a pointer that moved
    // needs the whole tree, once. The walk refreshes what was captured.
    scene.app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(30.0, 30.0),
    });
    scene.app.pump_frames(2);
    let before = walks();
    scene.set_modes(&[(0, 1)]);
    assert!(
        walks().0 > before.0,
        "the first update after a pointer move walks the tree"
    );

    let before = walks();
    scene.set_modes(&[(0, 0)]);
    assert_eq!(
        walks().0,
        before.0,
        "and the next is scoped again, with nothing recaptured"
    );
    assert!(walks().1 > before.1);
}

#[test]
fn owners_that_take_turns_rebuilding_all_stay_cheap() {
    // Two clocks ticking at different rates: each tick must not cost the other
    // its captured context.
    let counts = compare_counted(&[
        &[(0, 1)],
        &[(1, 1)],
        &[(0, 0)],
        &[(1, 0)],
        &[(0, 1)],
        &[(1, 1)],
        &[(0, 0)],
        &[(1, 0)],
    ]);

    // Each owner's first rebuild captures it; after both have, every tick is a pass.
    for (step, &(walks, passes)) in counts.iter().enumerate().skip(2) {
        assert!(
            walks == 0 && passes >= 1,
            "step {step} should be passes only, got {counts:?}"
        );
    }
}

/// The revision of the draw list of the first element called `name`'s child.
fn child_revision(scene: &Scene, name: &str, nth: usize) -> u64 {
    fn collect<'a>(element: &'a dyn Element, name: &str, out: &mut Vec<&'a dyn Element>) {
        if element.debug_name() == name {
            out.push(element);
        }
        element.visit_children(&mut |child| collect(child, name, out));
    }
    let mut owners = Vec::new();
    collect(
        scene.app.app.widget_root.as_ref().unwrap().as_ref(),
        name,
        &mut owners,
    );
    let mut child = None;
    owners[nth].visit_children(&mut |c| child = Some(c.id()));
    let node = scene
        .app
        .app
        .render_node_for_element(child.unwrap())
        .expect("child node");
    scene
        .app
        .app
        .render_tree()
        .draw_list_revision(node)
        .unwrap()
}

#[test]
fn a_scoped_state_frame_re_records_the_rebuilt_paint() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut scene = Scene::new(false);
    scene.set_modes(&[(0, 1)]);
    let (walks_before, passes_before) = walks();
    let mut revision = child_revision(&scene, "ScopedBox", 0);

    for mode in [0u8, 1, 0, 1] {
        scene.set_modes(&[(0, mode)]);
        let now = child_revision(&scene, "ScopedBox", 0);
        assert!(
            now > revision,
            "mode {mode}: the rebuilt box was not re-recorded ({revision} -> {now})"
        );
        revision = now;
    }

    let (walks_after, passes_after) = walks();
    assert_eq!(walks_after, walks_before, "all of those were scoped");
    assert!(passes_after - passes_before >= 4);
}

#[test]
fn a_scoped_state_frame_re_records_paint_below_the_rebuilt_root() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // The root of the rebuild is the column; its first swatch is what changes.
    fn swatch_revision(scene: &Scene) -> u64 {
        fn collect<'a>(element: &'a dyn Element, out: &mut Vec<&'a dyn Element>) {
            if element.debug_name() == "ScopedBox" {
                out.push(element);
            }
            element.visit_children(&mut |child| collect(child, out));
        }
        let mut owners = Vec::new();
        collect(
            scene.app.app.widget_root.as_ref().unwrap().as_ref(),
            &mut owners,
        );
        let mut column = None;
        owners[0].visit_children(&mut |c| column = Some(c));
        let mut swatch = None;
        column.unwrap().visit_children(&mut |c| {
            if swatch.is_none() {
                swatch = Some(c.id());
            }
        });
        let node = scene
            .app
            .app
            .render_node_for_element(swatch.unwrap())
            .expect("swatch node");
        scene
            .app
            .app
            .render_tree()
            .draw_list_revision(node)
            .unwrap()
    }

    let mut scene = Scene::new(false);
    scene.set_modes(&[(0, 5)]);
    scene.set_modes(&[(0, 6)]);
    let (walks_before, passes_before) = walks();
    let mut revision = swatch_revision(&scene);

    for mode in [5u8, 6, 5, 6] {
        scene.set_modes(&[(0, mode)]);
        let now = swatch_revision(&scene);
        assert!(
            now > revision,
            "mode {mode}: the swatch below the root was not re-recorded"
        );
        revision = now;
    }

    let (walks_after, passes_after) = walks();
    assert_eq!(walks_after, walks_before, "all of those were scoped");
    assert!(passes_after - passes_before >= 4);
}

#[test]
fn a_synchronous_state_update_keeps_its_full_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut scene = Scene::new(false);
    scene.set_modes(&[(0, 1)]);
    scene.set_modes(&[(0, 0)]);
    let (walks_before, passes_before) = walks();

    // Called straight from the test, as an event handler would: no guarantee
    // about what else the frame has to do.
    let updater = scene.updaters.borrow()[0];
    updater.set_state(|state| state.mode = 1);
    scene.app.pump_frames(3);

    let (walks_after, passes_after) = walks();
    assert!(
        walks_after > walks_before,
        "a direct set_state walks the whole tree"
    );
    assert_eq!(passes_after, passes_before, "and takes no pass");
}
