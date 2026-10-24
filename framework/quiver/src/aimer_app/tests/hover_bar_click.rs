//! A bar that reacts to being hovered stays idle and clickable.
//!
//! The post screen of a forum client wraps its action bar in a hover region
//! whose enter and exit callbacks change the screen's state, which fades the
//! bar. The screen rebuilds on every such change and builds the bar again, and a
//! region that is built again must not announce an enter the cursor already
//! made. Otherwise the screen rebuilds forever while the cursor rests on the
//! bar, the window never goes idle, and the button under the cursor is replaced
//! before the click that began on it can end.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{curves, ImplicitAnimatedBuilder};
use aimer_input::button::Button;
use aimer_input::mouse_region::MouseRegion;
use aimer_widget::StateUpdater;
use winit::event::{ElementState, MouseButton};

struct Counters {
    builds: Cell<usize>,
    taps: Cell<usize>,
}

struct ScreenState {
    hovered: bool,
    counters: Rc<Counters>,
    updater: Rc<RefCell<Option<StateUpdater<ScreenState>>>>,
}

struct Screen {
    counters: Rc<Counters>,
    updater: Rc<RefCell<Option<StateUpdater<ScreenState>>>>,
}

impl StatefulWidget for Screen {
    type State = ScreenState;

    fn create_state(self) -> ScreenState {
        ScreenState {
            hovered: false,
            counters: self.counters,
            updater: self.updater,
        }
    }
}

impl State<Screen> for ScreenState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        self.counters.builds.set(self.counters.builds.get() + 1);
        let updater = self.updater.borrow().expect("initialised");
        let counters = self.counters.clone();
        let opacity = if self.hovered { 1.0f32 } else { 0.4 };

        let bar = MouseRegion::new()
            .on_hover_enter(move || updater.set_state(|state| state.hovered = true))
            .on_hover_exit(move || updater.set_state(|state| state.hovered = false))
            .box_child(ImplicitAnimatedBuilder::new(
                opacity,
                Duration::from_millis(220),
                curves::EASE_IN_OUT,
                move |_opacity| {
                    let counters = counters.clone();
                    aimer_container::Container::new()
                        .height(60)
                        .color(Color::Rgb(60, 40, 40))
                        .child(
                            Row::new().children(vec![
                                Button::new()
                                    .on_press(move || counters.taps.set(counters.taps.get() + 1))
                                    .decoration(
                                        aimer_style::BoxDecoration::new()
                                            .background_color(Color::Rgb(90, 60, 60)),
                                    )
                                    .hover_decoration(
                                        aimer_style::BoxDecoration::new()
                                            .background_color(Color::Rgb(240, 100, 70)),
                                    )
                                    .child(
                                        SizedBox::new()
                                            .width(Dimension::Px(60.0))
                                            .height(Dimension::Px(40.0)),
                                    )
                                    .boxed(),
                                SizedBox::new()
                                    .width(Dimension::Px(100.0))
                                    .height(Dimension::Px(10.0))
                                    .boxed(),
                            ]),
                        )
                },
            ));

        let rows: Vec<aimer_widget::AnyWidget> = (0..6u8)
            .map(|i| {
                SizedBox::new()
                    .width(Dimension::Px(200.0))
                    .height(Dimension::Px(60.0))
                    .color(Color::Rgb(40 + i * 30, 80, 200 - i * 30))
                    .boxed()
            })
            .collect();
        aimer_space::Stack::new().children(vec![
            aimer_space::Positioned::new()
                .layer(3)
                .top(0)
                .box_child(bar),
            Scrollable::new()
                .child(Column::new().children(rows))
                .boxed(),
        ])
    }
}

impl Widget for Screen {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "HoverBarScreen", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Screen {}

fn start() -> (HeadlessAimerApp<impl Widget + 'static>, Rc<Counters>) {
    let counters = Rc::new(Counters {
        builds: Cell::new(0),
        taps: Cell::new(0),
    });
    let mut app = AimerApp::start_headless_with(
        Screen {
            counters: counters.clone(),
            updater: Rc::new(RefCell::new(None)),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, 240),
            scale_factor: 1.0,
        },
    );
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(-10.0, -10.0),
    });
    app.pump_frames(3);
    (app, counters)
}

fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
}

fn click<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>) {
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(1);
    }
}

/// Runs frames over `millis` of wall-clock time, as a window would.
fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = std::time::Instant::now() + Duration::from_millis(millis);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

#[test]
fn resting_on_the_back_button_lets_the_window_go_idle() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();

    move_to(&mut app, 30.0, 25.0);
    run_for(&mut app, 600);

    // The fade is over. A window with nothing to do asks for no more frames.
    let builds = counters.builds.get();
    let frames = app.pump_frames(10);
    assert_eq!(
        frames, 0,
        "the window is still drawing frames with nothing changing"
    );
    assert_eq!(
        counters.builds.get(),
        builds,
        "the screen is still rebuilding"
    );
    assert!(
        builds < 10,
        "the screen was built {builds} times for one hover"
    );
}

#[test]
fn the_back_button_can_be_clicked_while_the_bar_fades_in() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();

    move_to(&mut app, 30.0, 25.0);
    app.pump_frames(1);
    click(&mut app);

    assert_eq!(
        counters.taps.get(),
        1,
        "a click on the button right after hovering it did nothing"
    );
}

#[test]
fn the_back_button_can_be_clicked_once_the_bar_has_settled() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let (mut app, counters) = start();

    move_to(&mut app, 30.0, 25.0);
    run_for(&mut app, 600);
    click(&mut app);

    assert_eq!(
        counters.taps.get(),
        1,
        "a click on the settled button did nothing"
    );
}
