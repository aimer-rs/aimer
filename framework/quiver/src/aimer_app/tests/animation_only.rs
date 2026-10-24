//! A frame that exists only to advance a compositor animation visits the
//! animating elements and leaves the rest of the tree alone.
//!
//! The probes count every `update` they receive, so the tests can tell an
//! element that was revisited from one that was left alone.

use super::*;

use std::time::Duration;

use aimer_animation::{
    AnimatedBuilder, AnimatedPaint, AnimationController, curves, RotationTransition,
};

const PROBES: usize = 24;

struct Probe {
    updates: Rc<Cell<usize>>,
}

impl Widget for Probe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        ProbeElement {
            updates: self.updates,
        }
        .boxed()
    }
}

impl aimer_widget::PortableWidget for Probe {}

struct ProbeElement {
    updates: Rc<Cell<usize>>,
}

impl VisitorElement for ProbeElement {
    fn debug_name(&self) -> &'static str {
        "AnimationOnlyProbe"
    }
}

impl EventElement for ProbeElement {}
impl Rebuildable for ProbeElement {}

impl LayoutElement for ProbeElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(40.0), Dimension::Px(10.0)))
    }
}

impl Drawable for ProbeElement {
    fn update(&self, _ctx: &BuildContext) {
        self.updates.set(self.updates.get() + 1);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(Rect::new(0.0, 0.0, 40.0, 10.0), [200, 200, 200, 255]);
        canvas.finish();
    }
}

fn page(controller: AnimationController, updates: Rc<Cell<usize>>) -> impl Widget + 'static {
    let spinner = RotationTransition::new(
        controller,
        SizedBox::new()
            .width(Dimension::Px(48.0))
            .height(Dimension::Px(48.0))
            .color(Color::RED),
    );
    let mut children = vec![spinner.boxed()];
    children.extend((0..PROBES).map(|_| {
        Probe {
            updates: updates.clone(),
        }
        .boxed()
    }));
    Column::new().children(children)
}

fn looping() -> AnimationController {
    let controller = AnimationController::new(Duration::from_millis(1500), curves::LINEAR);
    controller.set_repeat(true);
    controller.forward_from_first_tick();
    controller
}

/// The rotation the spinner's render node shows, as (cos, |sin|) of its angle.
fn shown_rotation<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> (f32, f32) {
    fn find<'a>(element: &'a dyn Element) -> Option<&'a dyn Element> {
        if element.debug_name() == "RotationTransitionElement" {
            return Some(element);
        }
        let mut found = None;
        element.visit_children(&mut |child| {
            if found.is_none() {
                found = find(child);
            }
        });
        found
    }
    let root = app.app.widget_root.as_ref().expect("the tree is mounted");
    let spinner = find(root.as_ref()).expect("the spinner is mounted");
    let node = app
        .app
        .render_node_for_element(spinner.id())
        .expect("the spinner has a render node");
    let (matrix, _) = app
        .app
        .render_tree()
        .compositor_animation(node)
        .expect("the node exists");
    (matrix.cols[0][0], matrix.cols[0][1].abs())
}

fn start(
    controller: AnimationController,
) -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Cell<usize>>) {
    let updates = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        page(controller, updates.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    (app, updates)
}

/// Renders one frame after the clock has moved, and reports its damage.
fn next_frame<W: Widget + 'static>(
    app: &mut HeadlessAimerApp<W>,
) -> aimer_cupid::damage_region::DamageSet {
    std::thread::sleep(Duration::from_millis(12));
    assert_eq!(
        app.pump_frames(1),
        1,
        "the animation keeps asking for frames"
    );
    app.last_frame_result.take().expect("a frame was drawn").1
}

#[test]
fn an_animation_only_frame_does_not_visit_unrelated_elements() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, updates) = start(looping());
    let settled = updates.get();
    assert!(
        settled >= PROBES,
        "the full walk visits every probe while the tree settles"
    );

    for frame in 0..10 {
        let damage = next_frame(&mut app);
        assert!(
            !damage.is_empty(),
            "frame {frame}: the rotation must still be presented"
        );
        assert!(
            !damage.is_full(),
            "frame {frame}: the rotation damages only its own bounds"
        );
    }
    assert_eq!(
        updates.get(),
        settled,
        "probes were revisited by animation-only frames"
    );
}

#[test]
fn an_animation_only_frame_shows_the_rotation_the_controller_reports() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = looping();
    let (mut app, updates) = start(controller.clone());
    let settled = updates.get();

    let mut seen = Vec::new();
    for frame in 0..12 {
        next_frame(&mut app);
        // `value` is the sample this very frame took: nothing ticks it after.
        let turns = controller.value();
        let angle = turns * std::f32::consts::TAU;
        let (cos, sin) = shown_rotation(&app);
        assert!(
            (cos - angle.cos()).abs() < 1e-3 && (sin - angle.sin().abs()).abs() < 1e-3,
            "frame {frame}: node shows (cos {cos}, |sin| {sin}) but the controller is at {turns} turns"
        );
        seen.push(cos);
    }
    assert_eq!(
        updates.get(),
        settled,
        "every one of those frames took the cheap path"
    );
    assert!(
        seen.windows(2).any(|pair| pair[0] != pair[1]),
        "the rotation must actually move: {seen:?}"
    );
}

#[test]
fn any_other_frame_request_restores_the_full_walk() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, updates) = start(looping());
    for _ in 0..3 {
        next_frame(&mut app);
    }
    let before = updates.get();

    // Whoever asks without saying it is only an animation keeps the frame full.
    aimer_events::window::request_animation_frame();
    next_frame(&mut app);
    assert_eq!(
        updates.get(),
        before + PROBES,
        "an ordinary request walks every probe once"
    );

    let after_full = updates.get();
    next_frame(&mut app);
    assert_eq!(
        updates.get(),
        after_full,
        "and the next animation frame is cheap again"
    );
}

#[test]
fn pointer_input_restores_the_full_walk() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, updates) = start(looping());
    for _ in 0..3 {
        next_frame(&mut app);
    }
    let before = updates.get();

    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(20.0, 120.0),
    });
    next_frame(&mut app);
    assert!(
        updates.get() >= before + PROBES,
        "a cursor move must be seen by the whole tree"
    );
}

#[test]
fn a_finished_animation_settles_and_the_frame_loop_goes_idle() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = AnimationController::new(Duration::from_millis(60), curves::LINEAR);
    controller.forward_from_first_tick();
    let (mut app, _updates) = start(controller.clone());

    let mut frames = 0;
    while app.pump_frames(1) == 1 {
        frames += 1;
        assert!(
            frames < 100,
            "the loop must stop once the animation has finished"
        );
        std::thread::sleep(Duration::from_millis(12));
    }
    assert!(!controller.is_animating());
    assert_eq!(controller.value(), 1.0);
}

#[test]
fn an_offset_spinner_keeps_its_hit_area_across_animation_only_frames() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // The loading page centres its icon, so the icon's ancestors leave a
    // translation on the canvas that a lone revisit has to reproduce.
    let mut app = AimerApp::start_headless_with(
        loading_damage::loading_page(),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    for _ in 0..8 {
        next_frame(&mut app);
    }
    let audit = app.bounds_audit().expect("the tree is mounted");
    assert!(
        audit.interaction_adopted > 0,
        "the audit must compare something"
    );
}

/// A page whose only animation is an `AnimatedPaint` fade, which does not go
/// through the compositor, above a column of probes.
fn paint_page(controller: AnimationController, updates: Rc<Cell<usize>>) -> impl Widget + 'static {
    let fade = AnimatedPaint::new(
        controller,
        SizedBox::new()
            .width(Dimension::Px(48.0))
            .height(Dimension::Px(48.0))
            .color(Color::RED),
    )
    .opacity();
    let mut children = vec![fade.boxed()];
    children.extend((0..PROBES).map(|_| {
        Probe {
            updates: updates.clone(),
        }
        .boxed()
    }));
    Column::new().children(children)
}

/// The opacity the child of the `AnimatedPaint` is shown with.
fn shown_paint_opacity<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> f32 {
    fn find<'a>(element: &'a dyn Element) -> Option<&'a dyn Element> {
        if element.debug_name() == "AnimatedPaintElement" {
            return Some(element);
        }
        let mut found = None;
        element.visit_children(&mut |child| {
            if found.is_none() {
                found = find(child);
            }
        });
        found
    }
    let root = app.app.widget_root.as_ref().expect("the tree is mounted");
    let paint = find(root.as_ref()).expect("the AnimatedPaint is mounted");
    let mut child = None;
    paint.visit_children(&mut |c| child = Some(c.id()));
    let node = app
        .app
        .render_node_for_element(child.expect("it has a child"))
        .expect("child node");
    app.app
        .render_tree()
        .presentation(node)
        .expect("the node exists")
        .1
}

#[test]
fn an_animated_paint_frame_revisits_only_the_animated_paint() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = looping();
    let updates = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        paint_page(controller.clone(), updates.clone()),
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let settled = updates.get();
    assert!(
        settled >= PROBES,
        "the full walk visits every probe while the tree settles"
    );

    let mut seen = Vec::new();
    for frame in 0..10 {
        let damage = next_frame(&mut app);
        assert!(
            !damage.is_empty(),
            "frame {frame}: the fade must be presented"
        );
        assert!(
            !damage.is_full(),
            "frame {frame}: the fade damages only its own bounds"
        );
        let shown = shown_paint_opacity(&app);
        assert!(
            (shown - controller.value()).abs() < 1e-3,
            "frame {frame}: the child is shown at {shown} but the controller is at {}",
            controller.value()
        );
        seen.push(shown);
    }
    assert_eq!(
        updates.get(),
        settled,
        "probes were revisited by AnimatedPaint frames"
    );
    assert!(
        seen.windows(2).any(|pair| pair[0] != pair[1]),
        "the fade must actually move: {seen:?}"
    );
}

/// A repeating `AnimatedBuilder` in the middle of a page of static rows, counted
/// by how often its closure runs over a number of frames.
fn builder_runs_over_frames(frames: usize) -> usize {
    let calls = Rc::new(Cell::new(0usize));
    let animated = {
        let calls = calls.clone();
        AnimatedBuilder::new(looping(), move |progress| {
            calls.set(calls.get() + 1);
            SizedBox::new()
                .width(Dimension::Px(48.0))
                .height(Dimension::Px(48.0))
                .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
        })
    };
    let mut rows = vec![animated.boxed()];
    rows.extend((0..40).map(|_| {
        SizedBox::new()
            .width(Dimension::Px(120.0))
            .height(Dimension::Px(8.0))
            .color(Color::WHITE)
            .boxed()
    }));
    let page = aimer_container::Container::new()
        .color(Color::WHITE)
        .child(Column::new().children(rows));
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let before = calls.get();
    for _ in 0..frames {
        next_frame(&mut app);
    }
    calls.get() - before
}

#[test]
fn an_animated_builder_keeps_rebuilding_for_as_long_as_it_animates() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let runs = builder_runs_over_frames(20);
    assert!(
        runs >= 15,
        "the builder ran only {runs} times in 20 animating frames"
    );
}

fn find_element<'a>(element: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
    if element.debug_name() == name {
        return Some(element);
    }
    let mut found = None;
    element.visit_children(&mut |child| {
        if found.is_none() {
            found = find_element(child, name);
        }
    });
    found
}

/// The render node of the first child of the element called `name`.
fn child_node<W: Widget + 'static>(
    app: &HeadlessAimerApp<W>,
    name: &str,
) -> aimer_cupid::draw_cmd_v2::RenderNodeId {
    let root = app.app.widget_root.as_ref().expect("the tree is mounted");
    let parent = find_element(root.as_ref(), name).expect("the element is mounted");
    let mut child = None;
    parent.visit_children(&mut |c| child = Some(c.id()));
    app.app
        .render_node_for_element(child.expect("it has a child"))
        .expect("child node")
}

#[test]
fn an_animated_builder_frame_rerecords_its_own_subtree_and_nothing_else() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = looping();
    let updates = Rc::new(Cell::new(0));
    let page = {
        let updates = updates.clone();
        let builder = AnimatedBuilder::new(controller.clone(), |progress| {
            SizedBox::new()
                .width(Dimension::Px(48.0))
                .height(Dimension::Px(48.0))
                .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
        });
        let mut children = vec![builder.boxed()];
        children.extend((0..PROBES).map(|_| {
            Probe {
                updates: updates.clone(),
            }
            .boxed()
        }));
        Column::new().children(children)
    };
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let settled = updates.get();
    assert!(settled >= PROBES);
    let (full_syncs, _) = app.sync_counts();

    let mut revisions = Vec::new();
    for frame in 0..10 {
        let damage = next_frame(&mut app);
        assert!(
            !damage.is_empty(),
            "frame {frame}: the new colour must be presented"
        );
        assert!(
            !damage.is_full(),
            "frame {frame}: only the builder's bounds are damaged"
        );
        let node = child_node(&app, "AnimatedBuilderElement");
        revisions.push(app.app.render_tree().draw_list_revision(node).unwrap());
    }
    assert_eq!(
        updates.get(),
        settled,
        "probes were revisited by AnimatedBuilder frames"
    );
    assert_eq!(
        app.sync_counts().0,
        full_syncs,
        "no frame needed a full synchronization"
    );
    assert!(
        revisions.windows(2).all(|pair| pair[1] > pair[0]),
        "the rebuilt child must be re-recorded every frame: {revisions:?}"
    );
}

#[test]
fn an_animated_builder_that_changes_size_moves_its_neighbours() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let controller = looping();
    let updates = Rc::new(Cell::new(0));
    let page = {
        let builder = AnimatedBuilder::new(controller.clone(), |progress| {
            SizedBox::new()
                .width(Dimension::Px(40.0 + progress * 100.0))
                .height(Dimension::Px(20.0))
                .color(Color::RED)
        });
        Row::new().children(vec![
            builder.boxed(),
            Probe {
                updates: updates.clone(),
            }
            .boxed(),
        ])
    };
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(300, 100),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let (full_before, _) = app.sync_counts();

    for frame in 0..10 {
        next_frame(&mut app);
        let probe_x = {
            let root = app.app.widget_root.as_ref().unwrap();
            let probe = find_element(root.as_ref(), "AnimationOnlyProbe").expect("probe");
            let node = app
                .app
                .render_node_for_element(probe.id())
                .expect("probe node");
            app.app.render_tree().element_bounds(node).unwrap().x
        };
        let expected = 40.0 + controller.value() * 100.0;
        assert!(
            (probe_x - expected).abs() < 1.0,
            "frame {frame}: the neighbour sits at x={probe_x} but the builder is {expected} wide"
        );
    }
    assert!(
        app.sync_counts().0 > full_before,
        "a size change is a full synchronization"
    );
}

#[test]
fn an_animated_builder_in_a_page_of_rows_walks_only_itself() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let updates = Rc::new(Cell::new(0));
    let animated = AnimatedBuilder::new(looping(), |progress| {
        SizedBox::new()
            .width(Dimension::Px(48.0))
            .height(Dimension::Px(48.0))
            .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
    });
    let mut rows = vec![animated.boxed()];
    rows.extend((0..40).map(|_| {
        Probe {
            updates: updates.clone(),
        }
        .boxed()
    }));
    let page = aimer_container::Container::new()
        .color(Color::WHITE)
        .child(Column::new().children(rows));
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let settled = updates.get();
    let (walks, passes) = aimer_widget::traversal_counts();

    for _ in 0..20 {
        next_frame(&mut app);
    }

    let (walks_after, passes_after) = aimer_widget::traversal_counts();
    assert_eq!(updates.get(), settled, "the rows were revisited");
    assert_eq!(walks_after, walks, "no frame walked the whole tree");
    assert_eq!(
        passes_after - passes,
        20,
        "every frame was an animation-only pass"
    );
}

#[test]
fn a_neighbour_of_a_resizing_builder_keeps_a_hit_area_that_follows_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let builder = AnimatedBuilder::new(looping(), |progress| {
        SizedBox::new()
            .width(Dimension::Px(40.0 + progress * 100.0))
            .height(Dimension::Px(96.0))
            .color(Color::RED)
    });
    // An `Svg` measures its hit area from the canvas transform while it is
    // updated, so it only learns that it moved by being walked.
    let icon = aimer_svg::Svg::new(
        aimer_svg::SvgDocument::from_svg(include_bytes!(
            "../../../../jaime/assets/loading-1-svgrepo-com.svg"
        ))
        .expect("the bundled loading icon SVG should be valid"),
    )
    .bounded()
    .width(Dimension::Px(96.0))
    .height(Dimension::Px(96.0));
    let page = Row::new().children(vec![builder.boxed(), icon.boxed()]);
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);

    for _ in 0..8 {
        next_frame(&mut app);
    }

    let audit = app.bounds_audit().expect("the tree is mounted");
    assert!(
        audit.interaction_adopted > 0,
        "the audit must compare something"
    );
    assert!(
        audit.interaction_disagreements.is_empty(),
        "the icon's hit area was left where the builder used to push it: {:?}",
        audit.interaction_disagreements
    );
}

#[test]
fn an_animation_only_frame_rebuilds_a_builder_once() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let runs = builder_runs_over_frames(20);
    let (walks, passes) = aimer_widget::traversal_counts();
    assert!(
        passes >= 15,
        "the frames should be animation-only passes (walks {walks}, passes {passes})"
    );
    assert!(
        (18..=22).contains(&runs),
        "one builder run per frame is expected over 20 frames, saw {runs}"
    );
}
