//! How far from a resizable panel's edge the pointer still grabs it.
//!
//! A side panel sits next to a content pane, as in a sidebar layout. The grab
//! band is configured to reach `INNER` pixels into the panel and `OUTER` pixels
//! out of it, and the pointer has to find it wherever that band is.

use super::*;

use std::cell::RefCell;

use aimer_container::{Direction, Resizable};

const WIDTH: f32 = 250.0;
const INNER: f32 = 14.0;
const OUTER: f32 = 12.0;

struct Layout {
    zones: Rc<RefCell<Vec<Direction>>>,
}

impl Widget for Layout {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let zones = self.zones;
        Row::new()
            .children(vec![
                Resizable::new()
                    .height(f32::MAX)
                    .width(WIDTH)
                    .min_width(100.0)
                    .max_width(400.0)
                    .handle_thickness(INNER)
                    .handle_outset(OUTER)
                    .direction(Direction::RIGHT)
                    .on_resize_zone(move |zone: Direction| zones.borrow_mut().push(zone))
                    .child(
                        SizedBox::new()
                            .width(Dimension::Px(WIDTH))
                            .height(Dimension::Px(400.0))
                            .color(Color::Rgb(40, 30, 30)),
                    )
                    .boxed(),
                aimer_flex::Expanded::new()
                    .child(
                        // A content pane the way an application builds one: a
                        // scroll view of rows that react to the pointer.
                        Scrollable::new().child(
                            Column::new().children(
                                (0..8)
                                    .map(|_| {
                                        aimer_input::mouse_region::MouseRegion::new().box_child(
                                            SizedBox::new()
                                                .height(Dimension::Px(60.0))
                                                .color(Color::Rgb(20, 20, 20)),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ),
                        ),
                    )
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Layout {}

/// Moves the pointer to `x`, `y` from far away, and reports the last zone.
fn zone_at(x: f64, y: f64) -> Direction {
    zone_at_scale(x, y, 1.0)
}

/// The same at a display scale: `x` and `y` are logical, the pointer is physical.
fn zone_at_scale(x: f64, y: f64, scale: f64) -> Direction {
    let zones = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        Layout {
            zones: zones.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new((600.0 * scale) as u32, (400.0 * scale) as u32),
            scale_factor: scale,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(500.0 * scale, 200.0 * scale),
    });
    app.pump_frames(3);
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x * scale, y * scale),
    });
    app.pump_frames(2);
    let last = zones.borrow().last().copied().unwrap_or(Direction::NONE);
    last
}

#[test]
fn the_right_edge_is_grabbed_across_its_whole_configured_band() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut missed = Vec::new();
    // Inside the panel up to INNER from its edge, and outside it up to OUTER.
    for offset in [-13.0f32, -9.0, -5.0, -1.0, 1.0, 5.0, 9.0, 11.0] {
        let zone = zone_at(f64::from(WIDTH + offset), 200.0);
        if zone != Direction::RIGHT {
            missed.push((offset, zone));
        }
    }
    assert!(
        missed.is_empty(),
        "points inside the configured band did not grab the edge: {missed:?}"
    );
}

#[test]
fn the_right_edge_is_not_grabbed_beyond_its_band() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        zone_at(f64::from(WIDTH - INNER - 4.0), 200.0),
        Direction::NONE
    );
    assert_eq!(
        zone_at(f64::from(WIDTH + OUTER + 4.0), 200.0),
        Direction::NONE
    );
}

#[test]
fn the_zone_is_reported_empty_again_once_the_pointer_has_left_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for exit_to in [
        (500.0, 200.0),
        (WIDTH + OUTER as f32 + 30.0, 200.0),
        (100.0, 200.0),
        (WIDTH as f64 as f32 + 3.0, 380.0),
    ] {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let mut app = AimerApp::start_headless_with(
            Layout {
                zones: zones.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(600, 400),
                scale_factor: 1.0,
            },
        );
        for (x, y) in [
            (500.0, 200.0),
            (f64::from(WIDTH) - 2.0, 200.0),
            (f64::from(WIDTH) + 3.0, 200.0),
            (f64::from(exit_to.0), f64::from(exit_to.1)),
        ] {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, y),
            });
            app.pump_frames(2);
        }
        let reported = zones.borrow().clone();
        let leaves_the_band = exit_to.0 > WIDTH + OUTER as f32 || exit_to.0 < WIDTH - INNER;
        if leaves_the_band {
            assert_eq!(
                reported.last().copied(),
                Some(Direction::NONE),
                "pointer moved to {exit_to:?}; reported {reported:?}"
            );
        }
    }
}

struct ZoneState {
    active: bool,
    log: Rc<RefCell<Vec<bool>>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ZoneState>>>>,
}

struct ZoneScreen {
    log: Rc<RefCell<Vec<bool>>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ZoneState>>>>,
}

impl StatefulWidget for ZoneScreen {
    type State = ZoneState;

    fn create_state(self) -> ZoneState {
        ZoneState {
            active: false,
            log: self.log,
            updater: self.updater,
        }
    }
}

impl State<ZoneScreen> for ZoneState {
    fn init_state(&mut self, updater: aimer_widget::StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.log.borrow_mut().push(self.active);
        let updater = self.updater.borrow().expect("initialised");
        Row::new().children(vec![
            Resizable::new()
                .height(f32::MAX)
                .width(WIDTH)
                .handle_thickness(INNER)
                .handle_outset(OUTER)
                .direction(Direction::RIGHT)
                .on_resize_zone(move |zone: Direction| {
                    updater.set_state(move |state| state.active = zone != Direction::NONE);
                })
                .child(
                    SizedBox::new()
                        .width(Dimension::Px(WIDTH))
                        .height(Dimension::Px(400.0))
                        .color(Color::Rgb(40, 30, 30)),
                )
                .boxed(),
            aimer_flex::Expanded::new()
                .child(
                    SizedBox::new()
                        .height(Dimension::Px(400.0))
                        .color(Color::Rgb(20, 20, 20)),
                )
                .boxed(),
        ])
    }
}

impl Widget for ZoneScreen {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ZoneScreen", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for ZoneScreen {}

fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.pump_frames(2);
}

#[test]
fn a_screen_that_rebuilds_for_the_zone_is_told_when_the_pointer_leaves_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        ZoneScreen {
            log: log.clone(),
            updater: Rc::new(RefCell::new(None)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    move_to(&mut app, 500.0, 200.0);
    move_to(&mut app, f64::from(WIDTH) - 3.0, 200.0);
    assert_eq!(
        log.borrow().last().copied(),
        Some(true),
        "entering the zone must be reported: {:?}",
        log.borrow()
    );

    move_to(&mut app, 100.0, 200.0);

    assert_eq!(
        log.borrow().last().copied(),
        Some(false),
        "leaving the zone left it reported active: {:?}",
        log.borrow()
    );
}

#[test]
fn a_screen_that_rebuilds_for_the_zone_keeps_the_zone_while_the_pointer_moves_inside_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut app = AimerApp::start_headless_with(
        ZoneScreen {
            log: log.clone(),
            updater: Rc::new(RefCell::new(None)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    move_to(&mut app, 500.0, 200.0);
    for x in [
        WIDTH - 5.0,
        WIDTH - 3.0,
        WIDTH - 1.0,
        WIDTH + 1.0,
        WIDTH + 4.0,
    ] {
        move_to(&mut app, f64::from(x), 200.0);
    }
    let builds = log.borrow().len();
    assert_eq!(log.borrow().last().copied(), Some(true));
    assert!(
        builds <= 4,
        "moving inside one zone rebuilt the screen {builds} times: {:?}",
        log.borrow()
    );
}

#[test]
fn the_right_edge_is_grabbed_across_its_whole_band_on_a_2x_display() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut missed = Vec::new();
    for offset in [-13.0f32, -9.0, -5.0, -1.0, 1.0, 5.0, 9.0, 11.0] {
        let zone = zone_at_scale(f64::from(WIDTH + offset), 200.0, 2.0);
        if zone != Direction::RIGHT {
            missed.push((offset, zone));
        }
    }
    assert!(
        missed.is_empty(),
        "points inside the configured band did not grab the edge at 2x: {missed:?}"
    );
    assert_eq!(
        zone_at_scale(f64::from(WIDTH + OUTER + 4.0), 200.0, 2.0),
        Direction::NONE
    );
}

/// A panel next to a pane whose rows claim the pointer, as selectable text does
/// by asking for its own cursor.
struct ClaimingPane {
    zones: Rc<RefCell<Vec<Direction>>>,
}

impl Widget for ClaimingPane {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let zones = self.zones;
        Row::new()
            .children(vec![
                Resizable::new()
                    .height(f32::MAX)
                    .width(WIDTH)
                    .handle_thickness(INNER)
                    .handle_outset(OUTER)
                    .direction(Direction::RIGHT)
                    .on_resize_zone(move |zone: Direction| zones.borrow_mut().push(zone))
                    .child(
                        SizedBox::new()
                            .width(Dimension::Px(WIDTH))
                            .height(Dimension::Px(400.0))
                            .color(Color::Rgb(40, 30, 30)),
                    )
                    .boxed(),
                aimer_flex::Expanded::new()
                    .child(
                        aimer_input::mouse_region::MouseRegion::new()
                            .cursor(winit::window::CursorIcon::Text)
                            .box_child(
                                SizedBox::new()
                                    .height(Dimension::Px(400.0))
                                    .color(Color::Rgb(20, 20, 20)),
                            ),
                    )
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for ClaimingPane {}

#[test]
fn the_outer_band_is_grabbed_even_when_the_neighbouring_pane_claims_the_pointer() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut missed = Vec::new();
    for offset in [1.0f32, 3.0, 5.0, 9.0, 11.0] {
        let zones = Rc::new(RefCell::new(Vec::new()));
        let mut app = AimerApp::start_headless_with(
            ClaimingPane {
                zones: zones.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(600, 400),
                scale_factor: 1.0,
            },
        );
        for x in [500.0, 400.0, 300.0, f64::from(WIDTH + offset)] {
            app.send_window_event(WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(x, 200.0),
            });
            app.pump_frames(2);
        }
        if zones.borrow().last().copied() != Some(Direction::RIGHT) {
            missed.push((offset, zones.borrow().clone()));
        }
    }
    assert!(
        missed.is_empty(),
        "the pane next to the panel took the pointer in its outer band: {missed:?}"
    );
}

/// Rows inside the resizable panel that count how many of them are hovered.
struct HoverPanel {
    hovered: Rc<std::cell::Cell<i32>>,
}

impl Widget for HoverPanel {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let rows: Vec<aimer_widget::AnyWidget> = (0..6)
            .map(|_| {
                let (enter, exit) = (self.hovered.clone(), self.hovered.clone());
                aimer_input::mouse_region::MouseRegion::new()
                    .on_hover_enter(move || enter.set(enter.get() + 1))
                    .on_hover_exit(move || exit.set(exit.get() - 1))
                    .box_child(
                        SizedBox::new()
                            .width(Dimension::Px(WIDTH - 40.0))
                            .height(Dimension::Px(40.0))
                            .color(Color::Rgb(50, 40, 40)),
                    )
            })
            .collect();
        Row::new()
            .children(vec![
                Resizable::new()
                    .height(f32::MAX)
                    .width(WIDTH)
                    .handle_thickness(INNER)
                    .handle_outset(OUTER)
                    .direction(Direction::RIGHT)
                    .child(Column::new().children(rows))
                    .boxed(),
                aimer_flex::Expanded::new()
                    .child(
                        SizedBox::new()
                            .height(Dimension::Px(400.0))
                            .color(Color::Rgb(20, 20, 20)),
                    )
                    .boxed(),
            ])
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for HoverPanel {}

#[test]
fn a_row_stops_being_hovered_when_the_pointer_slides_into_the_resize_band() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let hovered = Rc::new(std::cell::Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        HoverPanel {
            hovered: hovered.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // Over the second row, then out along the row into the resize band, then
    // across it into the pane.
    let mut history = Vec::new();
    for x in [
        100.0, 200.0, 215.0, 240.0, 248.0, 252.0, 258.0, 300.0, 400.0,
    ] {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, 60.0),
        });
        app.pump_frames(2);
        history.push((x, hovered.get()));
    }

    assert_eq!(
        hovered.get(),
        0,
        "a row is still hovered after the pointer left the panel: {history:?}"
    );
    // The rows are 210 wide (WIDTH - 40) from x = 0, so the pointer has left the
    // row by x = 215 and must not be counted as over it from then on.
    let lingering: Vec<_> = history
        .iter()
        .filter(|(x, count)| *x > 215.0 && *count != 0)
        .collect();
    assert!(
        lingering.is_empty(),
        "the row stayed hovered after the pointer left it: {history:?}"
    );
}

struct ClickState {
    selected: usize,
    hovered: Rc<std::cell::Cell<i32>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ClickState>>>>,
}

struct ClickPanel {
    hovered: Rc<std::cell::Cell<i32>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ClickState>>>>,
}

impl StatefulWidget for ClickPanel {
    type State = ClickState;

    fn create_state(self) -> ClickState {
        ClickState {
            selected: 0,
            hovered: self.hovered,
            updater: self.updater,
        }
    }
}

impl State<ClickPanel> for ClickState {
    fn init_state(&mut self, updater: aimer_widget::StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let updater = self.updater.borrow().expect("initialised");
        let rows: Vec<aimer_widget::AnyWidget> = (0..6)
            .map(|index| {
                let (enter, exit) = (self.hovered.clone(), self.hovered.clone());
                let selected = self.selected == index;
                // A row like a category button: a hover region around a gesture
                // detector whose tap changes the screen's state.
                aimer_input::mouse_region::MouseRegion::new()
                    .on_hover_enter(move || enter.set(enter.get() + 1))
                    .on_hover_exit(move || exit.set(exit.get() - 1))
                    .box_child(
                        aimer_input::gesture::gesture_detector::GestureDetector::new()
                            .behavior(aimer_input::gesture::gesture_detector::GestureDetectorBehavior::BlockChild)
                            .on_tap(move || updater.set_state(move |state| state.selected = index))
                            .child(SizedBox::new().width(Dimension::Px(WIDTH - 40.0)).height(Dimension::Px(40.0)).color(
                                if selected { Color::Rgb(120, 60, 50) } else { Color::Rgb(50, 40, 40) },
                            )),
                    )
            })
            .collect();
        Row::new().children(vec![
            Resizable::new()
                .height(f32::MAX)
                .width(WIDTH)
                .handle_thickness(INNER)
                .handle_outset(OUTER)
                .direction(Direction::RIGHT)
                .child(Column::new().children(rows))
                .boxed(),
            aimer_flex::Expanded::new()
                .child(
                    SizedBox::new()
                        .height(Dimension::Px(400.0))
                        .color(Color::Rgb(20, 20, 20)),
                )
                .boxed(),
        ])
    }
}

impl Widget for ClickPanel {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ClickPanel", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for ClickPanel {}

#[test]
fn hover_keeps_following_the_pointer_after_a_click_that_rebuilds_the_panel() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let hovered = Rc::new(std::cell::Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        ClickPanel {
            hovered: hovered.clone(),
            updater: Rc::new(RefCell::new(None)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(600, 400),
            scale_factor: 1.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(500.0, 200.0),
    });
    app.pump_frames(3);

    let mut history = Vec::new();
    for round in 0..4 {
        let y = 20.0 + 40.0 * f64::from(round);
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(100.0, y),
        });
        app.pump_frames(2);
        history.push(("over row", round, hovered.get()));
        for state in [
            winit::event::ElementState::Pressed,
            winit::event::ElementState::Released,
        ] {
            app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: winit::event::MouseButton::Left,
            });
            app.pump_frames(2);
        }
        history.push(("after click", round, hovered.get()));
    }
    for x in [230.0, 252.0, 300.0, 450.0] {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, 100.0),
        });
        app.pump_frames(2);
    }
    history.push(("pointer in the pane", 0, hovered.get()));

    assert_eq!(
        hovered.get(),
        0,
        "a row is still hovered with the pointer in the pane: {history:?}"
    );
    assert!(
        history
            .iter()
            .filter(|(label, _, _)| *label == "over row")
            .all(|(_, _, count)| *count == 1),
        "exactly the row under the pointer is hovered after each move: {history:?}"
    );
}

#[test]
fn a_screen_that_rebuilds_for_the_zone_is_told_when_the_pointer_jumps_out_to_the_pane() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for target in [(300.0, 200.0), (500.0, 200.0), (500.0, 20.0)] {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut app = AimerApp::start_headless_with(
            ZoneScreen {
                log: log.clone(),
                updater: Rc::new(RefCell::new(None)),
            },
            HeadlessOptions {
                size: PhysicalSize::new(600, 400),
                scale_factor: 1.0,
            },
        );
        move_to(&mut app, 500.0, 200.0);
        move_to(&mut app, f64::from(WIDTH) - 3.0, 200.0);
        assert_eq!(
            log.borrow().last().copied(),
            Some(true),
            "entering the zone must be reported"
        );

        move_to(&mut app, target.0, target.1);

        assert_eq!(
            log.borrow().last().copied(),
            Some(false),
            "the zone stayed reported after the pointer jumped to {target:?}: {:?}",
            log.borrow()
        );
    }
}
