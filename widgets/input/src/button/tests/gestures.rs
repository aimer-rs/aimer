//! How a button reads a click: every press that means "press" has to reach
//! `on_press`, and a gesture the caller never asked for must not take one away.

use std::time::Duration;

use super::retained::button_context;
use super::*;

/// Long enough to pass the recognizer's long-press threshold.
const HELD: Duration = Duration::from_millis(550);

struct Mounted {
    root: AnyElement,
    dispatcher: aimer_widget::EventDispatcher,
}

impl Mounted {
    fn new(button: Button<Probe>) -> Self {
        let ctx = button_context();
        let root = button.to_element(&ctx);
        root.update(&ctx);
        let mut mounted = Self {
            root,
            dispatcher: aimer_widget::EventDispatcher::new(),
        };
        // A mouse arrives over a button before it presses it.
        mounted.send(at(1.0, 1.0), ElementEvent::PointerMove);
        mounted
    }

    fn send(&mut self, pointer: PointerInfo, event: impl FnOnce(PointerInfo) -> ElementEvent) {
        let _ = self
            .dispatcher
            .dispatch(self.root.as_ref(), pointer.pos, &event(pointer));
    }

    /// A press and release at one spot, with nothing between them.
    fn click(&mut self) {
        let pointer = at(1.0, 1.0);
        self.send(pointer, ElementEvent::PointerDown);
        self.send(pointer, ElementEvent::PointerUp);
    }

    /// A press held for `duration` before the release.
    fn hold(&mut self, duration: Duration) {
        let pointer = at(1.0, 1.0);
        self.send(pointer, ElementEvent::PointerDown);
        std::thread::sleep(duration);
        self.send(pointer, ElementEvent::PointerUp);
    }
}

fn at(x: f32, y: f32) -> PointerInfo {
    PointerInfo::mouse((x, y).into(), PointerButton::Primary)
}

fn counter() -> (Rc<Cell<usize>>, impl Fn() + 'static) {
    let count = Rc::new(Cell::new(0));
    let counted = count.clone();
    (count, move || counted.set(counted.get() + 1))
}

fn button_with(configure: impl FnOnce(Button) -> Button) -> Button<Probe> {
    configure(Button::new()).child(Probe { builds: Rc::default() })
}

#[tokio::test]
async fn a_button_with_only_a_press_handler_presses_on_every_quick_click() {
    let (presses, on_press) = counter();
    let mut button = Mounted::new(button_with(|button| button.on_press(on_press)));

    button.click();
    button.click();
    button.click();

    assert_eq!(
        presses.get(),
        3,
        "a second click soon after the first is another press, not a double-click nobody wanted"
    );
}

#[tokio::test]
async fn a_button_with_only_a_press_handler_presses_when_the_click_is_held_a_while() {
    let (presses, on_press) = counter();
    let mut button = Mounted::new(button_with(|button| button.on_press(on_press)));

    button.hold(HELD);

    assert_eq!(presses.get(), 1, "a slow click is not a long press nobody wanted");
}

#[tokio::test]
async fn a_button_presses_when_the_mouse_drifts_a_few_pixels_during_the_click() {
    let (presses, on_press) = counter();
    let mut button = Mounted::new(button_with(|button| button.on_press(on_press)));

    button.send(at(1.0, 1.0), ElementEvent::PointerDown);
    button.send(at(4.0, 5.0), ElementEvent::PointerMove);
    button.send(at(4.0, 5.0), ElementEvent::PointerUp);

    assert_eq!(presses.get(), 1);
}

#[tokio::test]
async fn a_button_that_asked_for_a_double_press_still_gets_one() {
    let (presses, on_press) = counter();
    let (doubles, on_double_press) = counter();
    let mut button = Mounted::new(button_with(|button| {
        button.on_press(on_press).on_double_press(on_double_press)
    }));

    button.click();
    button.click();

    assert_eq!(presses.get(), 1, "the first click is a press");
    assert_eq!(doubles.get(), 1, "the second completes the double press");
}

#[tokio::test]
async fn a_button_that_asked_for_a_long_press_still_gets_one() {
    let (presses, on_press) = counter();
    let (longs, on_long_press) = counter();
    let mut button = Mounted::new(button_with(|button| {
        button.on_press(on_press).on_long_press(on_long_press)
    }));

    button.hold(HELD);

    assert_eq!(longs.get(), 1);
    assert_eq!(presses.get(), 0, "a long press is not also a press");
}
