//! A wrapper does not forward `sync_paint_geometry` to a child that is about to
//! update itself, and the frame it produces is unchanged by that.
//!
//! The scene stacks single-child wrappers (container, hover region, padding,
//! button) so that a forward would otherwise be repeated at every level. The
//! differential tests run the same input with the skip off and on and compare
//! what the framework publishes from those syncs: every element's cached
//! interaction rectangle, the frame's render plan, and what the pointer does.

use super::*;

use aimer_container::{Container, Padding};
use aimer_input::button::Button;
use aimer_input::mouse_region::MouseRegion;
use aimer_style::LayoutSpacing;
use winit::event::{ElementState, MouseButton};

const WIDTH: u32 = 400;
const HEIGHT: u32 = 320;

#[derive(Default, Debug, PartialEq)]
struct Trace {
    /// Per step, every element's debug name and cached interaction rectangle.
    rects: Vec<Vec<(&'static str, Option<[f32; 4]>)>>,
    /// Per step, everything the frame would draw.
    plans: Vec<Vec<String>>,
    taps: usize,
    hover_enters: usize,
    hover_exits: usize,
}

struct Counters {
    taps: Cell<usize>,
    enters: Cell<usize>,
    exits: Cell<usize>,
}

fn card(index: u8, counters: &Rc<Counters>) -> aimer_widget::AnyWidget {
    let (enter, exit, tap) = (counters.clone(), counters.clone(), counters.clone());
    Container::new()
        .height(70)
        .margin(LayoutSpacing::all(4))
        .color(Color::Rgb(40 + index * 40, 60, 120))
        .child(
            MouseRegion::new()
                .on_hover_enter(move || enter.enters.set(enter.enters.get() + 1))
                .on_hover_exit(move || exit.exits.set(exit.exits.get() + 1))
                .box_child(
                    Container::new()
                        .padding(LayoutSpacing::all(6 + index as i32))
                        .color(Color::Rgb(90, 50 + index * 30, 70))
                        .child(
                            Padding::new().padding(LayoutSpacing::all(2)).child(
                                Button::new()
                                    .on_press(move || tap.taps.set(tap.taps.get() + 1))
                                    .decoration(
                                        aimer_style::BoxDecoration::new()
                                            .background_color(Color::Rgb(70, 70, 90)),
                                    )
                                    .hover_decoration(
                                        aimer_style::BoxDecoration::new()
                                            .background_color(Color::Rgb(230, 120, 60)),
                                    )
                                    .child(SizedBox::new().width(100).height(30)),
                            ),
                        ),
                ),
        )
        .boxed()
}

fn plan(app: &HeadlessAimerApp<aimer_modal::ModalHost<impl Widget + 'static>>) -> Vec<String> {
    app.app
        .render_tree()
        .render_all()
        .into_iter()
        .map(|operation| match operation {
            aimer_cupid::draw_cmd::RenderOp::Draw(item) => format!(
                "{:?} {:?} {:?} {:?} {:?}",
                item.element,
                item.bounds,
                item.origin,
                item.clip,
                item.snapshot().commands
            ),
            aimer_cupid::draw_cmd::RenderOp::BeginOpacityGroup {
                element,
                bounds,
                opacity,
                clip,
                masked,
                ..
            } => format!("begin {element:?} {bounds:?} {opacity} {clip:?} {masked}"),
            aimer_cupid::draw_cmd::RenderOp::EndOpacityGroup { element } => {
                format!("end {element:?}")
            }
        })
        .collect()
}

/// Runs one fixed pointer script against the scene and records what it shows.
fn run(skip: bool, scale: f64) -> Trace {
    aimer_widget::set_geometry_forward_skip(skip);
    let counters = Rc::new(Counters {
        taps: Cell::new(0),
        enters: Cell::new(0),
        exits: Cell::new(0),
    });
    let widget = Column::new().children(vec![
        card(0, &counters),
        card(1, &counters),
        card(2, &counters),
    ]);
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: scale,
        },
    );
    app.pump_frames(3);

    let device = DeviceId::dummy();
    let mut trace = Trace::default();
    let mut record = |app: &mut HeadlessAimerApp<_>| {
        trace.rects.push(app.interaction_rects());
        trace.plans.push(plan(app));
    };
    record(&mut app);
    for y in (10..HEIGHT).step_by(40) {
        for x in (10..WIDTH).step_by(50) {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: device,
                position: PhysicalPosition::new(x as f64, y as f64),
            });
            app.render_frame();
            app.render_frame();
            record(&mut app);
        }
    }
    for (x, y) in [(60.0, 40.0), (60.0, 130.0), (60.0, 220.0), (350.0, 300.0)] {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: device,
            position: PhysicalPosition::new(x, y),
        });
        app.render_frame();
        for state in [ElementState::Pressed, ElementState::Released] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: device,
                state,
                button: MouseButton::Left,
            });
            app.render_frame();
        }
        app.render_frame();
        record(&mut app);
    }
    trace.taps = counters.taps.get();
    trace.hover_enters = counters.enters.get();
    trace.hover_exits = counters.exits.get();
    aimer_widget::set_geometry_forward_skip(true);
    trace
}

/// A leaf that cannot paint, so no resolve ever runs its own paint-state sync:
/// the forward from its wrapper is the only geometry sync it ever receives.
struct GeometryCounter {
    calls: Rc<Cell<usize>>,
}

impl Widget for GeometryCounter {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        GeometryCounterElement { calls: self.calls }.boxed()
    }
}

struct GeometryCounterElement {
    calls: Rc<Cell<usize>>,
}

impl VisitorElement for GeometryCounterElement {
    fn debug_name(&self) -> &'static str {
        "GeometryCounter"
    }
}

impl EventElement for GeometryCounterElement {}
impl Rebuildable for GeometryCounterElement {}

impl LayoutElement for GeometryCounterElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(40.0), Dimension::Px(20.0)))
    }
}

impl Drawable for GeometryCounterElement {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        false
    }

    fn sync_paint_geometry(&self, _ctx: &BuildContext) {
        self.calls.set(self.calls.get() + 1);
    }
}

/// How many geometry syncs the counter leaf receives over four settled frames.
fn counter_syncs(skip: bool) -> usize {
    aimer_widget::set_geometry_forward_skip(skip);
    let calls = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        Container::new().height(80).color(Color::Rgb(30, 30, 60)).child(
            MouseRegion::new().box_child(
                Container::new()
                    .padding(LayoutSpacing::all(4))
                    .child(GeometryCounter { calls: calls.clone() }),
            ),
        ),
        HeadlessOptions {
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    calls.set(0);
    for _ in 0..4 {
        app.render_frame();
    }
    aimer_widget::set_geometry_forward_skip(true);
    calls.get()
}

/// The skip is only for children that will sync themselves. A child that never
/// does, because it cannot paint, is still forwarded to by its own parent in
/// every frame. What it no longer gets is the repeats from the wrappers further
/// up, which forwarded through the whole chain to reach it.
#[test]
fn a_child_that_cannot_sync_itself_still_receives_the_forward() {
    const FRAMES: usize = 4;
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let without = counter_syncs(false);
    let with = counter_syncs(true);
    assert!(without > FRAMES, "the whole chain forwards to the leaf without the skip");
    assert!(
        with >= FRAMES,
        "the leaf was starved of geometry: {with} syncs over {FRAMES} frames"
    );
    assert!(with < without, "the repeats from further up should be gone: {with} vs {without}");
}

/// A leaf that paints nothing but owns a render node, like every wrapper layer:
/// its own update syncs its geometry once, so a forward before it is a repeat.
struct TransparentCounter {
    calls: Rc<Cell<usize>>,
}

impl Widget for TransparentCounter {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        TransparentCounterElement { calls: self.calls }.boxed()
    }
}

struct TransparentCounterElement {
    calls: Rc<Cell<usize>>,
}

impl VisitorElement for TransparentCounterElement {
    fn debug_name(&self) -> &'static str {
        "TransparentCounter"
    }
}

impl EventElement for TransparentCounterElement {}
impl Rebuildable for TransparentCounterElement {}

impl LayoutElement for TransparentCounterElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(40.0), Dimension::Px(20.0)))
    }
}

impl Drawable for TransparentCounterElement {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
    }

    fn paints_nothing(&self) -> bool {
        true
    }

    fn sync_paint_geometry(&self, _ctx: &BuildContext) {
        self.calls.set(self.calls.get() + 1);
    }
}

/// How many geometry syncs the transparent leaf gets over `frames` frames drawn
/// from the first one on, with the wrappers above it promising its update.
fn transparent_syncs_from_the_first_frame(skip: bool, frames: usize) -> usize {
    aimer_widget::set_geometry_forward_skip(skip);
    let calls = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        Container::new().height(80).color(Color::Rgb(30, 30, 60)).child(
            MouseRegion::new().box_child(
                Container::new()
                    .padding(LayoutSpacing::all(4))
                    .child(TransparentCounter { calls: calls.clone() }),
            ),
        ),
        HeadlessOptions {
            size: PhysicalSize::new(WIDTH, HEIGHT),
            scale_factor: 1.0,
        },
    );
    for _ in 0..frames {
        app.render_frame();
    }
    aimer_widget::set_geometry_forward_skip(true);
    calls.get()
}

/// A wrapper does not forward geometry to a transparent child that has a render
/// node: the child's own update, which the wrapper promised, syncs it. That holds
/// from the first frame, not only once the node has recorded its paint.
#[test]
fn a_transparent_child_is_synced_once_per_frame_from_the_first_frame() {
    const FRAMES: usize = 4;
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let without = transparent_syncs_from_the_first_frame(false, FRAMES);
    let with = transparent_syncs_from_the_first_frame(true, FRAMES);
    assert!(with >= 1, "the leaf was never synced: {with}");
    assert!(
        with <= without,
        "the skip must never add syncs: {with} vs {without}"
    );
    assert!(
        with <= FRAMES + 1,
        "repeats were forwarded: {with} syncs over {FRAMES} frames (without the skip: {without})"
    );
}

fn assert_same_trace(scale: f64) {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let without = run(false, scale);
    let with = run(true, scale);
    assert!(
        without.rects.iter().flatten().any(|(_, rect)| rect.is_some()),
        "the scene must publish interaction rectangles for the comparison to mean anything"
    );
    assert!(without.taps > 0 && without.hover_enters > 0, "the script must exercise the pointer");
    for (step, (a, b)) in without.rects.iter().zip(&with.rects).enumerate() {
        assert_eq!(a, b, "interaction rectangles differ at step {step}");
    }
    for (step, (a, b)) in without.plans.iter().zip(&with.plans).enumerate() {
        assert_eq!(a, b, "plans differ at step {step}");
    }
    assert_eq!(
        (without.taps, without.hover_enters, without.hover_exits),
        (with.taps, with.hover_enters, with.hover_exits),
        "the pointer behaves differently"
    );
}

#[test]
fn skipping_forwards_publishes_the_same_geometry_at_scale_one() {
    assert_same_trace(1.0);
}

#[test]
fn skipping_forwards_publishes_the_same_geometry_at_scale_two() {
    assert_same_trace(2.0);
}

/// With the skip on, the forwarded visits fall and none is left unpaid: every
/// child that was spared a forward went on to sync itself.
#[cfg(feature = "frame-stats")]
#[test]
fn skipping_removes_forwarded_visits_and_every_skipped_child_syncs_itself() {
    use aimer_widget::geometry_walk_probe::{GeometryWalkReport, take_geometry_walk_report};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let measure = |skip: bool| {
        aimer_widget::set_geometry_forward_skip(skip);
        let counters = Rc::new(Counters {
            taps: Cell::new(0),
            enters: Cell::new(0),
            exits: Cell::new(0),
        });
        let mut app = AimerApp::start_headless_with(
            Column::new().children(vec![card(0, &counters), card(1, &counters)]),
            HeadlessOptions {
                size: PhysicalSize::new(WIDTH, HEIGHT),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(3);
        let _ = take_geometry_walk_report();
        let mut total = GeometryWalkReport::default();
        for _ in 0..4 {
            app.render_frame();
            total.merge(&take_geometry_walk_report());
        }
        total
    };
    let without = measure(false);
    let with = measure(true);
    aimer_widget::set_geometry_forward_skip(true);

    assert_eq!(without.suppressed_forwards, 0, "nothing is skipped with the switch off");
    assert!(with.suppressed_forwards > 0, "the skip never applied: {with:?}");
    assert!(
        with.forwarded_visits < without.forwarded_visits,
        "forwarded visits did not fall: {} with the skip, {} without",
        with.forwarded_visits,
        without.forwarded_visits
    );
    assert_eq!(with.unpaid_suppressions, 0, "a skipped child never synced itself: {with:?}");
    assert_eq!(with.own_syncs, without.own_syncs, "every element still syncs itself");
}
