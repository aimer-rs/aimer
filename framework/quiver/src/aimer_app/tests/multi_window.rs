//! Two windows alive on one UI thread keep their frames, runtime and metrics
//! apart.
//!
//! The framework was written for one window per thread, so its frame requester,
//! its runtime and its metrics subscribers are all thread-wide. Each test here
//! keeps two headless applications alive at once, which is the closest the
//! current code comes to two windows on a shared thread, and asks for something
//! a window must be able to rely on while another one exists: a state update
//! lands on the window its widget lives in, a frame runs with its own runtime
//! current, and a resize leaves the other window's widgets alone.
//!
//! These are the requirements the per-window scope has to satisfy; none of them
//! mentions how that scope is implemented.

use super::*;

use aimer_venus::Venus;
use aimer_widget::safe_area::{SafeAreaInsets, set_safe_area_insets_for};
use aimer_widget::StateUpdater;

/// What a probe records about its builds, shared with the test.
struct Probe {
    builds: Cell<usize>,
    /// Whether the build reads the window metrics, and so follows a resize.
    watches_metrics: bool,
    /// Whether the build reads the safe area, and so follows its changes.
    watches_safe_area: bool,
    /// The insets the last build read.
    insets_during_build: Cell<SafeAreaInsets>,
    /// The runtime that was current the last time the probe built.
    runtime_during_build: RefCell<Option<Rc<Venus>>>,
    updater: RefCell<Option<StateUpdater<ProbeState>>>,
}

impl Probe {
    fn new(watches_metrics: bool) -> Rc<Self> {
        Rc::new(Self {
            builds: Cell::new(0),
            watches_metrics,
            watches_safe_area: false,
            insets_during_build: Cell::new(SafeAreaInsets::ZERO),
            runtime_during_build: RefCell::new(None),
            updater: RefCell::new(None),
        })
    }

    fn following_the_safe_area() -> Rc<Self> {
        Rc::new(Self {
            builds: Cell::new(0),
            watches_metrics: false,
            watches_safe_area: true,
            insets_during_build: Cell::new(SafeAreaInsets::ZERO),
            runtime_during_build: RefCell::new(None),
            updater: RefCell::new(None),
        })
    }

    /// Changes the probe's state from outside any event or frame, the way a
    /// timer or a finished request does.
    fn touch(&self) {
        let updater = self.updater.borrow().expect("the probe was built");
        updater.set_state(|state| state.touches += 1);
    }
}

struct ProbeWidget(Rc<Probe>);

struct ProbeState {
    probe: Rc<Probe>,
    touches: usize,
}

impl StatefulWidget for ProbeWidget {
    type State = ProbeState;

    fn create_state(self) -> ProbeState {
        ProbeState {
            probe: self.0,
            touches: 0,
        }
    }
}

impl State<ProbeWidget> for ProbeState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.probe.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        self.probe.builds.set(self.probe.builds.get() + 1);
        *self.probe.runtime_during_build.borrow_mut() = Venus::current();
        if self.probe.watches_metrics {
            let _ = ctx.watch_window_metrics();
        }
        if self.probe.watches_safe_area {
            self.probe.insets_during_build.set(ctx.watch_safe_area_insets());
        }
        SizedBox::new()
            .width(Dimension::Px(40.0))
            .height(Dimension::Px(40.0))
    }
}

impl Widget for ProbeWidget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "MultiWindowProbe", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for ProbeWidget {}

fn open(
    probe: &Rc<Probe>,
    size: PhysicalSize<u32>,
) -> HeadlessAimerApp<impl Widget + 'static + use<>> {
    let mut app = AimerApp::start_headless_with(
        ProbeWidget(probe.clone()),
        HeadlessOptions {
            size,
            scale_factor: 1.0,
        },
    );
    app.pump_frames(8);
    app
}

/// Two windows, `a` opened first and `b` after it, both settled: neither has
/// a frame pending.
fn two_windows() -> (
    (Rc<Probe>, HeadlessAimerApp<impl Widget + 'static + use<>>),
    (Rc<Probe>, HeadlessAimerApp<impl Widget + 'static + use<>>),
) {
    let probe_a = Probe::new(false);
    let a = open(&probe_a, PhysicalSize::new(400, 300));
    let probe_b = Probe::new(false);
    let b = open(&probe_b, PhysicalSize::new(800, 600));
    assert!(!a.take_redraw_request() && !b.take_redraw_request());
    ((probe_a, a), (probe_b, b))
}

#[test]
fn a_state_update_requests_a_frame_for_the_window_it_lives_in() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ((probe_a, a), (_probe_b, _b)) = two_windows();

    probe_a.touch();

    assert!(
        a.take_redraw_request(),
        "the update was made in window A and A was never asked to draw it"
    );
}

#[test]
fn a_state_update_does_not_wake_another_window() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ((probe_a, _a), (_probe_b, b)) = two_windows();

    probe_a.touch();

    assert!(
        !b.take_redraw_request(),
        "window B was asked to draw a change that happened in window A"
    );
}

#[test]
fn a_state_update_is_built_by_its_own_window_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ((probe_a, mut a), (probe_b, mut b)) = two_windows();
    let (builds_a, builds_b) = (probe_a.builds.get(), probe_b.builds.get());

    probe_a.touch();
    a.pump_frames(8);
    b.pump_frames(8);

    assert_eq!(
        probe_a.builds.get(),
        builds_a + 1,
        "window A did not rebuild its own widget exactly once"
    );
    assert_eq!(
        probe_b.builds.get(),
        builds_b,
        "window B rebuilt for a change that was not its own"
    );
}

#[test]
fn a_frame_is_built_with_the_runtime_of_its_own_window() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ((probe_a, mut a), (_probe_b, _b)) = two_windows();

    // The first build happened while A was the only window, so what it saw
    // proves nothing; forget it and watch the rebuild instead. The frame is
    // drawn directly because where the update's request went is the business
    // of the tests above, not of this one.
    *probe_a.runtime_during_build.borrow_mut() = None;
    probe_a.touch();
    a.render_frame();

    let during_build = probe_a
        .runtime_during_build
        .borrow()
        .clone()
        .expect("a runtime was current while the widget built");
    assert!(
        Rc::ptr_eq(&during_build, a.venus()),
        "window A built with the runtime that window B installed last"
    );
}

#[test]
fn resizing_one_window_leaves_the_widgets_of_another_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let probe_a = Probe::new(true);
    let mut a = open(&probe_a, PhysicalSize::new(400, 300));
    let probe_b = Probe::new(true);
    let mut b = open(&probe_b, PhysicalSize::new(800, 600));
    let (builds_a, builds_b) = (probe_a.builds.get(), probe_b.builds.get());

    let resized = PhysicalSize::new(450, 300);
    a.window.update_headless_metrics(resized, 1.0);
    a.send_window_event(WindowEvent::Resized(resized));
    // Both frames are drawn outright, so a rebuild that was only marked, and
    // would otherwise wait for B's next frame, shows up too.
    a.render_frame();
    b.render_frame();

    assert!(
        probe_a.builds.get() > builds_a,
        "the resized window did not rebuild the widget that reads its size"
    );
    assert_eq!(
        probe_b.builds.get(),
        builds_b,
        "window B rebuilt because window A was resized"
    );
}

#[test]
fn each_window_follows_the_safe_area_reported_for_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let probe_a = Probe::following_the_safe_area();
    let mut a = open(&probe_a, PhysicalSize::new(400, 300));
    let probe_b = Probe::following_the_safe_area();
    let mut b = open(&probe_b, PhysicalSize::new(800, 600));
    let (builds_a, builds_b) = (probe_a.builds.get(), probe_b.builds.get());
    let status_bar = SafeAreaInsets::new(0.0, 24.0, 0.0, 0.0);

    let notified = set_safe_area_insets_for(&a.window, status_bar);
    a.render_frame();
    b.render_frame();

    assert_eq!(notified, 1, "the report reached a widget outside window A");
    assert!(
        probe_a.builds.get() > builds_a,
        "window A did not rebuild for its own insets"
    );
    assert_eq!(probe_a.insets_during_build.get(), status_bar);
    assert_eq!(
        probe_b.builds.get(),
        builds_b,
        "window B rebuilt for insets reported for window A"
    );
    assert_eq!(
        probe_b.insets_during_build.get(),
        SafeAreaInsets::ZERO,
        "window B was given window A's insets"
    );
}

/// A tree with a probe at the root of a column and another two levels down, so
/// a state update has a path through the tree to travel and not just a root to
/// mark.
struct NestedProbes {
    shallow: Rc<Probe>,
    deep: Rc<Probe>,
}

impl Widget for NestedProbes {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Column::new()
            .children(vec![
                ProbeWidget(self.shallow).boxed(),
                Column::new()
                    .children(vec![
                        SizedBox::new()
                            .width(Dimension::Px(10.0))
                            .height(Dimension::Px(10.0))
                            .boxed(),
                        Row::new()
                            .children(vec![ProbeWidget(self.deep).boxed()])
                            .boxed(),
                    ])
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for NestedProbes {}

fn open_nested(
    size: PhysicalSize<u32>,
) -> (
    Rc<Probe>,
    Rc<Probe>,
    HeadlessAimerApp<impl Widget + 'static + use<>>,
) {
    let (shallow, deep) = (Probe::new(false), Probe::new(false));
    let mut app = AimerApp::start_headless_with(
        NestedProbes {
            shallow: shallow.clone(),
            deep: deep.clone(),
        },
        HeadlessOptions {
            size,
            scale_factor: 1.0,
        },
    );
    app.pump_frames(8);
    (shallow, deep, app)
}

fn builds(probes: &[&Rc<Probe>]) -> Vec<usize> {
    probes.iter().map(|probe| probe.builds.get()).collect()
}

#[test]
fn updates_pending_in_two_windows_are_each_applied_by_their_own_frames() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (a_shallow, a_deep, mut a) = open_nested(PhysicalSize::new(400, 300));
    let (b_shallow, b_deep, mut b) = open_nested(PhysicalSize::new(800, 600));
    let all = [&a_shallow, &a_deep, &b_shallow, &b_deep];
    let before = builds(&all);

    // Both windows have something to rebuild when only A draws. B's update
    // must still be waiting afterwards, not consumed by A's frame.
    a_deep.touch();
    b_shallow.touch();
    a.pump_frames(8);
    assert_eq!(
        builds(&all),
        [before[0], before[1] + 1, before[2], before[3]],
        "A's frame did not rebuild exactly A's updated widget"
    );

    b.pump_frames(8);
    assert_eq!(
        builds(&all),
        [before[0], before[1] + 1, before[2] + 1, before[3]],
        "B's update was lost or applied more than once"
    );

    // And the other way round, with the deeper widget on B's side.
    b_deep.touch();
    a_shallow.touch();
    b.pump_frames(8);
    a.pump_frames(8);
    assert_eq!(
        builds(&all),
        [before[0] + 1, before[1] + 1, before[2] + 1, before[3] + 1],
        "an update made while the other window drew was lost"
    );
}

#[test]
fn windows_drawing_in_turn_each_settle() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (a_shallow, a_deep, mut a) = open_nested(PhysicalSize::new(400, 300));
    let (b_shallow, b_deep, mut b) = open_nested(PhysicalSize::new(800, 600));
    let all = [&a_shallow, &a_deep, &b_shallow, &b_deep];

    for round in 0..5 {
        let before = builds(&all);
        let (touched, expected) = match round % 4 {
            0 => (&a_deep, 1),
            1 => (&b_shallow, 2),
            2 => (&a_shallow, 0),
            _ => (&b_deep, 3),
        };
        touched.touch();
        // Frames alternate between the windows, as a shared loop delivers them.
        for _ in 0..3 {
            a.pump_frames(1);
            b.pump_frames(1);
        }
        let mut want = before.clone();
        want[expected] += 1;
        assert_eq!(builds(&all), want, "round {round}");
        assert!(
            !a.take_redraw_request() && !b.take_redraw_request(),
            "round {round}: a window kept asking for frames with nothing to draw"
        );
    }
}
