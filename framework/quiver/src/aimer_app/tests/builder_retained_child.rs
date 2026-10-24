//! An animated builder that holds a retained child leaves it alone.
//!
//! A builder is asked for its whole output on every frame of its animation, but
//! most of what an animated widget shows does not depend on the animation: a tab
//! bar fading in is the same tab bar at every step. That part is handed to the
//! builder as a [`ChildBuilder`], built once, and each frame only places it again.
//! Placing it again must not run its builds, and the pass that follows must not
//! walk its elements: both used to, so a frame cost as much as the retained
//! subtree was large, which is exactly what handing it over was meant to avoid.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{AnimatedBuilder, AnimationController, curves};
use aimer_widget::{ChildBuilder, StateUpdater};

const ROWS: usize = 30;
const ROW_DEPTH: usize = 5;
const FRAMES: usize = 20;

fn looping() -> AnimationController {
    let controller = AnimationController::new(Duration::from_millis(1500), curves::LINEAR);
    controller.set_repeat(true);
    controller.forward_from_first_tick();
    controller
}

fn deep_row() -> aimer_widget::AnyWidget {
    let mut row = SizedBox::new()
        .width(Dimension::Px(120.0))
        .height(Dimension::Px(8.0))
        .color(Color::WHITE)
        .boxed();
    for _ in 0..ROW_DEPTH {
        row = aimer_container::Container::new().child(row).boxed();
    }
    row
}

/// A stateless widget that counts its builds and holds `ROWS` deep rows.
#[derive(Clone)]
struct Counted {
    builds: Rc<Cell<usize>>,
}

impl Widget for Counted {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let build = move |ctx: &BuildContext| -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            Column::new()
                .children((0..ROWS).map(|_| deep_row()).collect::<Vec<_>>())
                .to_element(ctx)
        };
        let child = build(ctx);
        aimer_widget::StatelessElement::new(child, build, None, "Counted").boxed()
    }
}

impl aimer_widget::PortableWidget for Counted {}

fn start(builds: Rc<Cell<usize>>, calls: Rc<Cell<usize>>) -> HeadlessAimerApp<impl Widget + 'static> {
    let retained = ChildBuilder::from_widget(Counted { builds });
    let animated = AnimatedBuilder::new(looping(), move |progress| {
        calls.set(calls.get() + 1);
        Column::new().children([
            SizedBox::new()
                .width(Dimension::Px(48.0))
                .height(Dimension::Px(8.0))
                .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
                .boxed(),
            retained.clone().boxed(),
        ])
    });
    let mut app = AimerApp::start_headless_with(
        aimer_container::Container::new().color(Color::WHITE).child(animated),
        HeadlessOptions {
            size: PhysicalSize::new(200, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    app
}

fn next_frame<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) {
    std::thread::sleep(Duration::from_millis(12));
    assert_eq!(app.pump_frames(1), 1, "the animation keeps asking for frames");
    app.last_frame_result.take().expect("a frame was drawn");
}

/// Builds of the retained child, runs of the builder, and the boundaries each
/// animating frame descended, over `FRAMES` frames.
fn run() -> (usize, usize, Vec<u64>) {
    let builds = Rc::new(Cell::new(0usize));
    let calls = Rc::new(Cell::new(0usize));
    let mut app = start(builds.clone(), calls.clone());
    for _ in 0..3 {
        next_frame(&mut app);
    }
    let (builds_before, calls_before) = (builds.get(), calls.get());
    let mut descents = Vec::new();
    for _ in 0..FRAMES {
        let before = aimer_widget::rebuild_descent_count();
        next_frame(&mut app);
        descents.push(aimer_widget::rebuild_descent_count() - before);
    }
    (builds.get() - builds_before, calls.get() - calls_before, descents)
}

#[test]
fn placing_a_retained_child_again_does_not_build_it_again() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (builds, calls, _) = run();

    assert!(calls >= FRAMES - 5, "the builder ran only {calls} times in {FRAMES} frames");
    assert_eq!(builds, 0, "the retained child was built {builds} times over {calls} frames");
}

#[test]
fn placing_a_retained_child_again_does_not_walk_its_elements() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (_, _, descents) = run();

    // The retained subtree alone holds `ROWS * (ROW_DEPTH + 2)` boundaries; the
    // builder's own fresh output is a handful.
    let worst = descents.iter().copied().max().unwrap_or(0);
    assert!(
        worst <= 16,
        "an animating frame descended {worst} boundaries, per frame: {descents:?}; \
         the retained child holds {}",
        ROWS * (ROW_DEPTH + 2)
    );
}

/// A stateful widget that counts its builds and can be told to change.
struct Switch {
    builds: Rc<Cell<usize>>,
    updater: Rc<RefCell<Option<StateUpdater<SwitchState>>>>,
}

struct SwitchState {
    on: bool,
    builds: Rc<Cell<usize>>,
    updater: Rc<RefCell<Option<StateUpdater<SwitchState>>>>,
}

impl StatefulWidget for Switch {
    type State = SwitchState;

    fn create_state(self) -> SwitchState {
        SwitchState { on: false, builds: self.builds, updater: self.updater }
    }
}

impl State<Switch> for SwitchState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.builds.set(self.builds.get() + 1);
        SizedBox::new()
            .width(Dimension::Px(40.0))
            .height(Dimension::Px(8.0))
            .color(if self.on { Color::Rgb(0, 200, 0) } else { Color::Rgb(200, 0, 0) })
    }
}

impl Widget for Switch {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Switch", None).0.boxed()
    }
}

impl aimer_widget::PortableWidget for Switch {}

#[test]
fn a_retained_child_still_rebuilds_when_its_own_state_changes_during_the_animation() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let builds = Rc::new(Cell::new(0usize));
    let updater = Rc::new(RefCell::new(None));
    let retained = ChildBuilder::from_widget(Switch { builds: builds.clone(), updater: updater.clone() });
    let animated = AnimatedBuilder::new(looping(), move |progress| {
        Column::new().children([
            SizedBox::new()
                .width(Dimension::Px(48.0))
                .height(Dimension::Px(8.0))
                .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
                .boxed(),
            retained.clone().boxed(),
        ])
    });
    let mut app = AimerApp::start_headless_with(
        aimer_container::Container::new().color(Color::WHITE).child(animated),
        HeadlessOptions { size: PhysicalSize::new(200, 200), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    for _ in 0..6 {
        next_frame(&mut app);
    }
    let before = builds.get();

    let state = updater.borrow().expect("the switch has a state");
    state.set_state(|state| state.on = true);
    for _ in 0..3 {
        next_frame(&mut app);
    }
    let after_change = builds.get();
    for _ in 0..10 {
        next_frame(&mut app);
    }

    assert!(after_change > before, "the switch was not rebuilt after its state changed");
    assert_eq!(
        builds.get(),
        after_change,
        "the switch kept rebuilding after the change, once per frame of the animation"
    );
}

#[test]
fn an_animator_inside_a_retained_child_keeps_running_after_the_child_moves() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    // The nested animator stays in the rebuild pass by the path it registered; a
    // path that no longer describes where it is would have the pass prune it and
    // the animation would stop.
    let inner_calls = Rc::new(Cell::new(0usize));
    let counted_inner = inner_calls.clone();
    let retained = ChildBuilder::from_widget(AnimatedBuilder::new(looping(), move |progress| {
        counted_inner.set(counted_inner.get() + 1);
        SizedBox::new()
            .width(Dimension::Px(40.0))
            .height(Dimension::Px(8.0))
            .color(Color::Rgb(0, (progress * 255.0) as u8, 0))
    }));
    let calls = Rc::new(Cell::new(0usize));
    let counted = calls.clone();
    // After a few frames the builder puts the child under other parents, so the
    // chain of elements above it is no longer the one it was last descended under.
    let animated = AnimatedBuilder::new(looping(), move |progress| {
        counted.set(counted.get() + 1);
        let marker = SizedBox::new()
            .width(Dimension::Px(48.0))
            .height(Dimension::Px(8.0))
            .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
            .boxed();
        let child = if counted.get() > 8 {
            aimer_container::Container::new().child(retained.clone()).boxed()
        } else {
            retained.clone().boxed()
        };
        Column::new().children([marker, child])
    });
    let mut app = AimerApp::start_headless_with(
        aimer_container::Container::new().color(Color::WHITE).child(animated),
        HeadlessOptions { size: PhysicalSize::new(200, 200), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    while calls.get() < 14 {
        next_frame(&mut app);
    }
    for _ in 0..4 {
        next_frame(&mut app);
    }
    let before = inner_calls.get();

    for _ in 0..10 {
        next_frame(&mut app);
    }

    assert!(
        inner_calls.get() >= before + 8,
        "the animator inside the moved child ran {} times in 10 frames",
        inner_calls.get() - before
    );
}

#[test]
fn with_child_builds_the_child_once_however_many_steps_the_animation_takes() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let builds = Rc::new(Cell::new(0usize));
    let animated = AnimatedBuilder::with_child(
        looping(),
        Counted { builds: builds.clone() },
        |progress, child| {
            Column::new().children([
                SizedBox::new()
                    .width(Dimension::Px(48.0))
                    .height(Dimension::Px(8.0))
                    .color(Color::Rgb((progress * 255.0) as u8, 0, 0))
                    .boxed(),
                child.boxed(),
            ])
        },
    );
    let mut app = AimerApp::start_headless_with(
        aimer_container::Container::new().color(Color::WHITE).child(animated),
        HeadlessOptions { size: PhysicalSize::new(200, 800), scale_factor: 1.0 },
    );
    app.pump_frames(4);

    for _ in 0..FRAMES {
        next_frame(&mut app);
    }

    assert_eq!(builds.get(), 1, "the child was built {} times", builds.get());
}
