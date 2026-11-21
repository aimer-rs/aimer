//! A force press that opens something under the pointer is not a force press of what it opened.
//!
//! A force click in a code editor goes to a definition, and the text it opens is under the same spot of the pad with the
//! finger still down. The pad goes on reporting the deep press, and an element that did not exist when it began must not
//! take it for its own: the window knows the press began, and tells it.

use super::*;

use std::cell::{Cell, RefCell};

use aimer_input::button::Button;
use aimer_widget::StateUpdater;
use winit::event::DeviceId;

type Updater = Rc<RefCell<Option<StateUpdater<HandoverState>>>>;

#[derive(Clone)]
struct Handover {
    updater: Updater,
    first: Rc<Cell<usize>>,
    second: Rc<Cell<usize>>,
}

struct HandoverState {
    swapped: bool,
    widget: Handover,
}

impl State<Handover> for HandoverState {
    fn create(widget: Handover) -> Self {
        HandoverState { swapped: false, widget }
    }

    fn on_mount(&mut self, updater: StateUpdater<Self>) {
        *self.widget.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(this: StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        let body = || SizedBox::new().width(Dimension::Px(400.0)).height(Dimension::Px(300.0));
        if this.swapped {
            // Another element, as a file that is opened is: not the first one, rebuilt.
            let second = Rc::clone(&this.widget.second);
            aimer_container::Container::new()
                .child(Button::new().on_force_press(move || second.set(second.get() + 1)).child(body()))
                .boxed()
        } else {
            let (first, updater) = (Rc::clone(&this.widget.first), this.widget.updater.borrow().expect("mounted"));
            Button::new()
                .on_force_press(move || {
                    first.set(first.get() + 1);
                    updater.set_state(|state| state.swapped = true);
                })
                .child(body())
                .boxed()
        }
    }
}

impl Widget for Handover {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, HandoverState>(self, ctx, "Handover", None).0.boxed()
    }
}

fn pressure(app: &mut HeadlessAimerApp<impl Widget + 'static>, pressure: f32, stage: i64) {
    app.send_window_event(WindowEvent::TouchpadPressure { device_id: DeviceId::dummy(), pressure, stage });
    app.pump_frames(2);
}

#[test]
fn what_a_force_press_opened_does_not_take_the_press_that_is_still_held() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (first, second) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let mut app = AimerApp::start_headless_with(
        Handover { updater: Rc::new(RefCell::new(None)), first: Rc::clone(&first), second: Rc::clone(&second) },
        HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor: 1.0 },
    );
    app.pump_frames(3);
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(100.0, 100.0),
    });

    pressure(&mut app, 0.4, 1);
    pressure(&mut app, 0.9, 2);
    assert_eq!((first.get(), second.get()), (1, 0), "the force press is the first element's, which opens the second");
    // The finger is held, and the pad goes on saying so.
    pressure(&mut app, 1.0, 2);
    pressure(&mut app, 1.0, 2);
    assert_eq!((first.get(), second.get()), (1, 0), "the press that opened it is not its force press");

    // Let go, and press deep again: that is the second element's own.
    pressure(&mut app, 0.0, 0);
    pressure(&mut app, 0.4, 1);
    pressure(&mut app, 0.9, 2);
    assert_eq!((first.get(), second.get()), (1, 1));
    pressure(&mut app, 0.0, 0);
}
