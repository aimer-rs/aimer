//! Dragging a sidebar whose own state follows the resize zone.
//!
//! The sidebar highlights its border while the pointer is over the grab band, so
//! the zone callback calls `set_state`, which rebuilds the implicit animation
//! that holds the `Resizable`. The drag has to survive that rebuild.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{Animatable, curves, ImplicitAnimatedBuilder};
use aimer_container::{Direction, Resizable};
use aimer_style::{BorderSlice, BorderStyle, BoxDecoration, BoxOutline};
use winit::event::{ElementState, MouseButton};

const WIDTH: f32 = 250.0;
const MAX: f32 = 400.0;

type Updater = Rc<RefCell<Option<aimer_widget::StateUpdater<SidebarState>>>>;

struct SidebarState {
    resizable: bool,
    updater: Updater,
    hovered: Rc<std::cell::Cell<i32>>,
}

struct Sidebar {
    updater: Updater,
    hovered: Rc<std::cell::Cell<i32>>,
}

impl StatefulWidget for Sidebar {
    type State = SidebarState;

    fn create_state(self) -> SidebarState {
        SidebarState {
            resizable: false,
            updater: self.updater,
            hovered: self.hovered,
        }
    }
}

impl State<Sidebar> for SidebarState {
    fn init_state(&mut self, updater: aimer_widget::StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let zone_updater = self.updater.borrow().expect("initialised");
        let hovered = self.hovered.clone();
        ImplicitAnimatedBuilder::new(
            if self.resizable { 1.0 } else { 0.0 },
            Duration::from_millis(300),
            curves::EASE_IN_OUT,
            move |delta: &f32| {
                let zone_updater = zone_updater;
                let hovered = hovered.clone();
                let stroke = 1.0f32.lerp(&3.0, *delta);
                aimer_container::Container::new()
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(Color::Rgb(30, 22, 20))
                            .outline(BoxOutline::new().right(
                                BorderSlice::new()
                                    .color(Color::Rgb(200, 80, 60))
                                    .style(BorderStyle::Solid)
                                    .stroke(stroke),
                            )),
                    )
                    .child(
                        Resizable::new()
                            .height(f32::MAX)
                            .width(WIDTH)
                            .min_width(100.0)
                            .max_width(MAX)
                            .handle_thickness(14.0)
                            .handle_outset(12.0)
                            .direction(Direction::RIGHT)
                            .on_resize_zone(move |zone: Direction| {
                                zone_updater.set_state(move |state| {
                                    state.resizable = zone != Direction::NONE;
                                });
                            })
                            .child(
                                aimer_container::Container::new()
                                    .padding(aimer_style::LayoutSpacing::horizontal(12.into()))
                                    .child(
                                        Column::new().vertical_alignment(aimer_flex::BoxAlignment::Start).children([
                                            aimer_flex::Expanded::new()
                                                .child(Scrollable::new().vertical_scroll_bar(None).child(
                                                    Column::new().children(
                                                        (0..12)
                                                            .map(|_| {
                                                                let (enter, exit) = (hovered.clone(), hovered.clone());
                                                                aimer_input::mouse_region::MouseRegion::new()
                                                                    .on_hover_enter(move || enter.set(enter.get() + 1))
                                                                    .on_hover_exit(move || exit.set(exit.get() - 1))
                                                                    .box_child(
                                                                        SizedBox::new()
                                                                            .height(Dimension::Px(30.0))
                                                                            .color(Color::Rgb(60, 40, 40)),
                                                                    )
                                                            })
                                                            .collect::<Vec<_>>(),
                                                    ),
                                                ))
                                                .boxed(),
                                            SizedBox::new().height(12).boxed(),
                                            SizedBox::new().height(40).color(Color::Rgb(70, 50, 50)).boxed(),
                                        ]),
                                    ),
                            ),
                    )
            },
        )
    }
}

impl Widget for Sidebar {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Sidebar", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Sidebar {}

struct Shell {
    updater: Updater,
    hovered: Rc<std::cell::Cell<i32>>,
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        aimer_container::Container::new()
            .color(Color::Rgb(10, 10, 10))
            .child(
                Row::new()
                    .justify_content(aimer_flex::JustifyContent::SpaceBetween)
                    .children(vec![
                        Sidebar {
                            updater: self.updater,
                            hovered: self.hovered,
                        }
                        .boxed(),
                        aimer_flex::Expanded::new()
                            .flex(0.95)
                            .child(
                                SizedBox::new()
                                    .height(Dimension::Px(400.0))
                                    .color(Color::Rgb(20, 20, 20)),
                            )
                            .boxed(),
                    ]),
            )
            .to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Shell {}

fn find<'a>(element: &'a dyn Element, name: &str) -> Option<&'a dyn Element> {
    if element.debug_name() == name {
        return Some(element);
    }
    let mut found = None;
    element.structural_children(&mut |child| {
        if found.is_none() {
            found = find(child, name);
        }
    });
    found
}

/// The width the resizable itself reports.
fn panel_width(app: &HeadlessAimerApp<impl Widget + 'static>) -> f32 {
    let root = app.app.widget_root.as_ref().unwrap().as_ref();
    match find(root, "Resizable")
        .and_then(|resizable| resizable.size())
        .map(|size| size.width)
    {
        Some(Dimension::Px(width)) => width,
        _ => f32::NAN,
    }
}

fn move_to(app: &mut HeadlessAimerApp<impl Widget + 'static>, x: f64, frames: usize) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, 200.0),
    });
    app.pump_frames(frames);
}

fn button(app: &mut HeadlessAimerApp<impl Widget + 'static>, state: ElementState) {
    app.send_window_event(WindowEvent::MouseInput {
        device_id: DeviceId::dummy(),
        state,
        button: MouseButton::Left,
    });
    app.pump_frames(1);
}

/// Hovers the edge, presses and drags `steps` steps of `5` pixels, `frames`
/// frames apart, and lists the steps where the panel was not under the pointer.
fn drag(frames: usize, steps: u32) -> Vec<(u32, f64, f32, f32)> {
    drag_from(-2.0, frames, steps)
}

/// The same drag, pressing `offset` pixels from the sidebar's right edge with
/// nothing hovered first, the way a click arrives.
fn drag_from(offset: f64, frames: usize, steps: u32) -> Vec<(u32, f64, f32, f32)> {
    let mut app = AimerApp::start_headless_with(
        Shell {
            updater: Rc::new(RefCell::new(None)),
            hovered: Rc::new(std::cell::Cell::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(800, 400),
            scale_factor: 1.0,
        },
    );
    move_to(&mut app, 600.0, 1);
    app.pump_frames(3);
    move_to(&mut app, f64::from(WIDTH) + offset, frames);
    button(&mut app, ElementState::Pressed);

    let mut wrong = Vec::new();
    for step in 1..=steps {
        let x = f64::from(WIDTH) + offset + f64::from(step) * 5.0;
        move_to(&mut app, x, frames);
        let expected = (x - offset).min(f64::from(MAX)) as f32;
        let panel = panel_width(&app);
        if (panel - expected).abs() > 2.0 {
            wrong.push((step, x, panel, expected));
        }
    }
    button(&mut app, ElementState::Released);
    wrong
}

#[test]
fn a_sidebar_that_rebuilds_for_the_zone_still_follows_the_drag() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for frames in [1, 2, 4] {
        let wrong = drag(frames, 24);
        assert!(
            wrong.is_empty(),
            "{frames} frame(s) per move; (step, pointer, panel's width, expected): the panel did not follow the dragged edge: {wrong:?}"
        );
    }
}

#[test]
fn a_press_anywhere_in_the_grab_band_starts_the_drag() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut dead = Vec::new();
    for offset in [-13.0, -10.0, -6.0, -2.0, 2.0, 6.0, 10.0] {
        // No frame between the move and the press, as with a click.
        for frames in [0, 1, 3] {
            let wrong = drag_from(offset, frames, 6);
            if !wrong.is_empty() {
                dead.push((offset, frames, wrong.len()));
            }
        }
    }
    assert!(
        dead.is_empty(),
        "presses at these offsets from the edge did not drag it: {dead:?}"
    );
}

#[test]
fn rows_in_a_sidebar_that_follows_its_zone_are_not_left_hovered_in_the_pane() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // The pointer crosses the rows, rests in the grab band, and leaves in one
    // jump or by sliding out, by every route a mouse takes to the pane.
    let routes: [&[f64]; 4] = [
        &[600.0, 100.0, 200.0, 245.0, 252.0, 258.0, 300.0, 600.0],
        &[600.0, 100.0, 200.0, 600.0],
        &[600.0, 100.0, 248.0, 600.0],
        &[600.0, 255.0, 100.0, 240.0, 270.0, 500.0],
    ];
    let mut lingering = Vec::new();
    for (route, xs) in routes.iter().enumerate() {
        let hovered = Rc::new(std::cell::Cell::new(0));
        let mut app = AimerApp::start_headless_with(
            Shell {
                updater: Rc::new(RefCell::new(None)),
                hovered: hovered.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(800, 400),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(3);
        for &x in *xs {
            for y in [60.0, 61.0] {
                app.send_window_event(WindowEvent::CursorMoved {
                    device_id: DeviceId::dummy(),
                    position: PhysicalPosition::new(x, y),
                });
                app.pump_frames(2);
            }
        }
        if hovered.get() != 0 {
            lingering.push((route, hovered.get()));
        }
    }
    assert!(
        lingering.is_empty(),
        "(route, rows still hovered) after the pointer reached the pane: {lingering:?}"
    );
}

// The paint-time hover check consults this flag: a list the dragged edge sweeps
// across must not start hovering, because the moves that leave it go to the edge.
#[test]
fn the_pointer_counts_as_captured_for_exactly_the_length_of_the_drag() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        Shell {
            updater: Rc::new(RefCell::new(None)),
            hovered: Rc::new(std::cell::Cell::new(0)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(800, 400),
            scale_factor: 1.0,
        },
    );
    move_to(&mut app, 600.0, 1);
    app.pump_frames(3);
    move_to(&mut app, f64::from(WIDTH) - 3.0, 2);
    assert!(
        !aimer_widget::pointer_capture_active(),
        "hovering the edge captures nothing"
    );

    button(&mut app, ElementState::Pressed);
    assert!(
        aimer_widget::pointer_capture_active(),
        "a press on the edge owns the pointer"
    );
    move_to(&mut app, 200.0, 2);
    assert!(
        aimer_widget::pointer_capture_active(),
        "and keeps it while the drag moves"
    );

    button(&mut app, ElementState::Released);
    assert!(
        !aimer_widget::pointer_capture_active(),
        "the release gives it back"
    );
}
