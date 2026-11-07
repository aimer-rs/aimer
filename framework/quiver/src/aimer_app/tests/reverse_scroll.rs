//! A reverse scrollable inside the real frame loop: a chat transcript opens on
//! its newest row, positions count from the end, short transcripts sit at the
//! bottom, and a windowed list's estimated heights do not move the view.
//!
//! Which row is under a point is read the way a user would find out: by tapping
//! it. Nothing here looks at the engine's internals.

use super::*;

use aimer_input::gesture::gesture_detector::{GestureDetector, GestureDetectorBehavior};
use aimer_scroll::ScrollController;
use winit::event::Touch;

const VIEWPORT: f32 = 400.0;

/// A transcript of `count` rows, row `i` as tall as `height(i)`.
struct Transcript {
    count: usize,
    height: fn(usize) -> f32,
    controller: ScrollController,
    tapped: Rc<Cell<Option<usize>>>,
}

impl Widget for Transcript {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let height = self.height;
        let tapped = self.tapped.clone();
        let rows = Column::new()
            .list(0..self.count)
            .builder(move |&index| {
                let tapped = tapped.clone();
                GestureDetector::new()
                    .behavior(GestureDetectorBehavior::BlockChild)
                    .on_tap(move || tapped.set(Some(index)))
                    .child(
                        SizedBox::new()
                            .width(Dimension::Px(200.0))
                            .height(Dimension::Px(height(index))),
                    )
            });
        SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(VIEWPORT))
            .child(
                Scrollable::new()
                    .reverse(true)
                    .vertical_scroll_bar(None)
                    .horizontal_scroll_bar(None)
                    .controller(self.controller)
                    .child(rows),
            )
            .to_element(ctx)
    }
}

fn uniform(_: usize) -> f32 {
    60.0
}

/// 40, 60 or 80 px, so a single probed row predicts the total wrongly.
fn varying(index: usize) -> f32 {
    40.0 + 20.0 * (index % 3) as f32
}

struct Chat {
    app: HeadlessAimerApp<ModalHost<Transcript>>,
    controller: ScrollController,
    tapped: Rc<Cell<Option<usize>>>,
}

fn start(count: usize, height: fn(usize) -> f32) -> Chat {
    let controller = ScrollController::new();
    let tapped = Rc::new(Cell::new(None));
    let mut app = AimerApp::start_headless_with(
        Transcript {
            count,
            height,
            controller: controller.clone(),
            tapped: tapped.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, VIEWPORT as u32),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(5);
    Chat {
        app,
        controller,
        tapped,
    }
}

impl Chat {
    fn touch(&mut self, phase: TouchPhase, y: f64) {
        self.app.send_window_event(WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(100.0, y),
            force: None,
            id: 1,
        }));
        self.app.pump_frames(1);
    }

    /// The row a tap at `y` activates, if any.
    fn row_at(&mut self, y: f64) -> Option<usize> {
        self.tapped.set(None);
        self.touch(TouchPhase::Started, y);
        self.touch(TouchPhase::Ended, y);
        self.tapped.get()
    }

    fn settle(&mut self) {
        self.app.pump_frames(10);
    }
}

#[test]
fn a_transcript_opens_on_its_newest_row() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut chat = start(200, uniform);

    assert_eq!(chat.controller.offset().y, 0.0, "the end is position zero");
    assert_eq!(chat.controller.max_extent().y, 200.0 * 60.0 - VIEWPORT);
    assert_eq!(chat.row_at(390.0), Some(199), "the last row is at the bottom");
    assert_eq!(chat.row_at(10.0), Some(193), "and the six rows above it");
}

#[test]
fn a_position_is_a_distance_from_the_end() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut chat = start(200, uniform);

    chat.controller.jump_to(Vec2d { x: 0.0, y: 1200.0 });
    chat.settle();

    assert_eq!(chat.controller.offset().y, 1200.0);
    // 1200 px back, a point 10 px above the viewport bottom is 1210 px from the
    // end: 20 whole rows (1200 px) plus 10 px into the 21st.
    assert_eq!(chat.row_at(390.0), Some(199 - 20));

    chat.controller.jump_to(Vec2d::ZERO);
    chat.settle();
    assert_eq!(chat.row_at(390.0), Some(199), "zero is the newest again");
}

#[test]
fn a_short_transcript_sits_at_the_bottom() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut chat = start(3, uniform);

    // Three 60 px rows occupy y 220..400.
    assert_eq!(chat.row_at(390.0), Some(2));
    assert_eq!(chat.row_at(230.0), Some(0));
    assert_eq!(chat.row_at(100.0), None, "the empty space above is empty");
}

#[test]
fn estimated_heights_above_the_view_do_not_move_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut chat = start(500, varying);

    chat.controller.jump_to(Vec2d { x: 0.0, y: 3000.0 });
    chat.settle();
    let before = chat.row_at(390.0).expect("a row is under the point");

    // Rows above are measured as they are painted and the total is corrected.
    chat.settle();
    chat.settle();

    assert_eq!(
        chat.controller.offset().y,
        3000.0,
        "the distance from the end survived the corrections"
    );
    assert_eq!(chat.row_at(390.0), Some(before), "the same row is on screen");
}

/// A parent that rebuilds its transcript on demand, the way any `set_state`
/// above a scrollable does.
struct Rebuilding {
    count: usize,
    reverse: bool,
    controller: Option<ScrollController>,
    tapped: Rc<Cell<Option<usize>>>,
    poke: Rc<RefCell<Option<StateUpdater<RebuildingState>>>>,
}

struct RebuildingState {
    count: usize,
    reverse: bool,
    controller: Option<ScrollController>,
    generation: u32,
    tapped: Rc<Cell<Option<usize>>>,
    poke: Rc<RefCell<Option<StateUpdater<RebuildingState>>>>,
}

impl StatefulWidget for Rebuilding {
    type State = RebuildingState;

    fn create_state(self) -> RebuildingState {
        RebuildingState {
            count: self.count,
            reverse: self.reverse,
            controller: self.controller,
            generation: 0,
            tapped: self.tapped,
            poke: self.poke,
        }
    }
}

impl State<Rebuilding> for RebuildingState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.poke.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let tapped = self.tapped.clone();
        let rows = Column::new().list(0..self.count).builder(move |&index| {
            let tapped = tapped.clone();
            GestureDetector::new()
                .behavior(GestureDetectorBehavior::BlockChild)
                .on_tap(move || tapped.set(Some(index)))
                .child(SizedBox::new().width(Dimension::Px(200.0)).height(Dimension::Px(60.0)))
        });
        let scrollable = Scrollable::new()
            .reverse(self.reverse)
            .vertical_scroll_bar(None)
            .horizontal_scroll_bar(None);
        let scrollable = match &self.controller {
            Some(controller) => scrollable.controller(controller.clone()),
            None => scrollable,
        };
        // `generation` changes nothing visible; it only makes this a new build.
        let _ = self.generation;
        SizedBox::new()
            .width(Dimension::Px(200.0))
            .height(Dimension::Px(VIEWPORT))
            .child(scrollable.child(rows))
    }
}

impl Widget for Rebuilding {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Rebuilding", None)
            .0
            .boxed()
    }
}

/// Scrolls 1200 px back, rebuilds the parent, and returns the controller's
/// position before and after.
fn offset_across_a_parent_rebuild(reverse: bool, with_controller: bool) -> (f32, f32) {
    let controller = ScrollController::new();
    let poke = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Rebuilding {
            count: 200,
            reverse,
            controller: with_controller.then(|| controller.clone()),
            tapped: Rc::new(Cell::new(None)),
            poke: poke.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, VIEWPORT as u32),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(5);
    // A forward list is 1200 px from its start, a reverse one from its end.
    // Without the app's controller the same distance is reached with the wheel.
    let engine = ScrollController::new();
    let _ = engine;
    controller.jump_to(Vec2d { x: 0.0, y: 1200.0 });
    app.pump_frames(10);
    let before = controller.offset().y;

    let updater = poke.borrow().expect("the parent exposes its updater");
    updater.set_state(|state| state.generation += 1);
    app.pump_frames(10);
    (before, controller.offset().y)
}

#[test]
fn a_parent_rebuild_keeps_a_forward_offset() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (before, after) = offset_across_a_parent_rebuild(false, true);
    assert_eq!(before, 1200.0);
    assert_eq!(after, before);
}

#[test]
fn a_parent_rebuild_keeps_a_reverse_offset() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (before, after) = offset_across_a_parent_rebuild(true, true);
    assert_eq!(before, 1200.0);
    assert_eq!(after, before, "the rebuild moved the view");
}

/// Drags the list away from its anchor with no controller anywhere, rebuilds the parent, and
/// returns the row under a fixed point before and after.
fn row_across_a_parent_rebuild(reverse: bool) -> (Option<usize>, Option<usize>) {
    let poke = Rc::new(RefCell::new(None));
    let tapped = Rc::new(Cell::new(None));
    let mut app = AimerApp::start_headless_with(
        Rebuilding {
            count: 200,
            reverse,
            controller: None,
            tapped: tapped.clone(),
            poke: poke.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, VIEWPORT as u32),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(5);
    // Drag the content toward the start of the list: down for a reverse list,
    // which opens at its end, and up for a forward one, which opens at its
    // start and has to move the other way to scroll at all.
    let (from, to) = if reverse { (60.0, 360.0) } else { (360.0, 60.0) };
    for _ in 0..4 {
        let mut finger = |phase, y: f64| {
            app.send_window_event(WindowEvent::Touch(Touch {
                device_id: DeviceId::dummy(),
                phase,
                location: PhysicalPosition::new(100.0, y),
                force: None,
                id: 2,
            }));
            app.pump_frames(1);
        };
        finger(TouchPhase::Started, from);
        for step in 1..=8 {
            finger(TouchPhase::Moved, from + (to - from) * f64::from(step) / 8.0);
        }
        finger(TouchPhase::Ended, to);
        app.pump_frames(60);
    }

    let mut tap = |app: &mut HeadlessAimerApp<_>| {
        tapped.set(None);
        for phase in [TouchPhase::Started, TouchPhase::Ended] {
            app.send_window_event(WindowEvent::Touch(Touch {
                device_id: DeviceId::dummy(),
                phase,
                location: PhysicalPosition::new(100.0, 390.0),
                force: None,
                id: 1,
            }));
            app.pump_frames(1);
        }
        tapped.get()
    };
    let before = tap(&mut app);
    let updater = poke.borrow().expect("the parent exposes its updater");
    updater.set_state(|state| state.generation += 1);
    app.pump_frames(30);
    (before, tap(&mut app))
}

#[test]
fn a_parent_rebuild_keeps_the_row_on_screen_without_a_controller() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for reverse in [false, true] {
        let (before, after) = row_across_a_parent_rebuild(reverse);
        let row = before.expect("a row is under the point");
        assert!(
            if reverse { row < 199 } else { row > 0 },
            "the wheel did not scroll (reverse={reverse}, row {row})"
        );
        assert_eq!(after, before, "the rebuild moved the view (reverse={reverse})");
    }
}
