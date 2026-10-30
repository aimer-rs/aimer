//! The scroll bar is something a pointer can hold: a mouse drags its thumb, a
//! finger holds it, and it shows itself when reached and fades out by itself.
//!
//! The page is 400 x 600 with 60 rows of 50, so the content is 3000 tall. The
//! thumb is `600 * 600 / 3000 = 120` long on a track of 600, which leaves 480 of
//! travel for 2400 of scroll: one pixel of thumb is five of content.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
};
use aimer::input::button::Button;
use aimer::{
    AimerApp, Column, HeadlessOptions, ScrollController, Scrollable, SizedBox, Widget,
};

const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 600.0;
/// The thumb sits in the last 12 pixels of the viewport.
const THUMB_X: f64 = 394.0;
/// How far content moves for each pixel the thumb does.
const CONTENT_PER_THUMB_PIXEL: f32 = 5.0;

struct Page<W: Widget + 'static> {
    app: aimer::HeadlessAimerApp<W>,
    controller: ScrollController,
    /// Device pixels per logical pixel.
    scale: f64,
}

fn page() -> Page<impl Widget + 'static> {
    page_at(1.0)
}

fn page_at(scale: f64) -> Page<impl Widget + 'static> {
    page_of_at(
        scale,
        (0..60)
            .map(|_| SizedBox::new().width(400).height(50).boxed())
            .collect(),
    )
}

/// The same page with a button on every row, as a list of cards has: the thumb
/// is drawn over buttons, and a press on it must not reach them.
fn pressable_page(presses: Rc<Cell<usize>>) -> Page<impl Widget + 'static> {
    page_of(
        (0..60)
            .map(|_| {
                let presses = presses.clone();
                Button::new()
                    .on_press(move || presses.set(presses.get() + 1))
                    .child(SizedBox::new().width(400).height(50))
                    .boxed()
            })
            .collect(),
    )
}

fn page_of(rows: Vec<aimer::AnyWidget>) -> Page<impl Widget + 'static> {
    page_of_at(1.0, rows)
}

fn page_of_at(scale: f64, rows: Vec<aimer::AnyWidget>) -> Page<impl Widget + 'static> {
    let controller = ScrollController::new();
    let mut app = AimerApp::start_headless_with(
        Scrollable::new()
            .controller(controller.clone())
            .child(Column::new().children(rows)),
        HeadlessOptions {
            size: PhysicalSize::new((WIDTH * scale) as u32, (HEIGHT * scale) as u32),
            scale_factor: scale,
        },
    );
    app.pump_frames(4);
    Page { app, controller, scale }
}

impl<W: Widget + 'static> Page<W> {
    fn move_cursor(&mut self, x: f64, y: f64) {
        self.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x * self.scale, y * self.scale),
        });
        self.app.pump_frames(2);
    }

    fn button(&mut self, state: ElementState) {
        self.app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        self.app.pump_frames(2);
    }

    /// Moves the cursor in small steps, as a hand does.
    fn drag_cursor(&mut self, x: f64, from: f64, to: f64) {
        for step in 1..=10 {
            self.move_cursor(x, from + (to - from) * f64::from(step) / 10.0);
        }
    }

    fn touch(&mut self, phase: TouchPhase, x: f64, y: f64) {
        self.app.send_window_event(WindowEvent::Touch(Touch {
            device_id: DeviceId::dummy(),
            phase,
            location: PhysicalPosition::new(x * self.scale, y * self.scale),
            force: None,
            id: 1,
        }));
        self.app.pump_frames(2);
    }

    fn drag_finger(&mut self, x: f64, from: f64, to: f64) {
        for step in 1..=10 {
            self.touch(
                TouchPhase::Moved,
                x,
                from + (to - from) * f64::from(step) / 10.0,
            );
        }
    }

    /// One wheel tick, which is what makes a bar appear on a desktop.
    fn wheel(&mut self, dy: f64) {
        self.move_cursor(200.0, 300.0);
        self.app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, dy * self.scale)),
            phase: TouchPhase::Moved,
        });
        self.app.pump_frames(6);
    }

    /// Lets the bar finish showing, which takes real time: a finger can hold
    /// only a thumb that is on screen.
    fn wait_until_shown(&mut self) {
        for _ in 0..100 {
            if self.controller.scroll_bar_opacity() >= 1.0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(16));
            self.app.pump_frames(2);
        }
        panic!("the bar never finished showing");
    }

    fn offset(&self) -> f32 {
        self.controller.offset().y
    }
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!(
        (actual - expected).abs() < 2.0,
        "{what}: expected {expected}, got {actual}"
    );
}

#[test]
fn dragging_the_thumb_keeps_it_under_the_cursor() {
    let mut page = page();
    page.wheel(-10.0);
    let start = page.offset();
    // The thumb is wherever the scroll left it; hold it in the middle.
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;

    page.move_cursor(THUMB_X, thumb_middle);
    page.button(ElementState::Pressed);
    page.drag_cursor(THUMB_X, thumb_middle, thumb_middle + 100.0);
    page.button(ElementState::Released);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "content follows the thumb one to one",
    );
}

#[test]
fn a_bar_that_is_not_showing_is_revealed_by_reaching_for_it() {
    let mut page = page();
    assert_eq!(page.controller.scroll_bar_opacity(), 0.0);

    // No scroll has happened: the bar is hidden, and the thumb is at the top.
    page.move_cursor(THUMB_X, 60.0);
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(16));
        page.app.pump_frames(2);
    }
    assert_eq!(
        page.controller.scroll_bar_opacity(),
        1.0,
        "hovering the bar shows it"
    );

    page.button(ElementState::Pressed);
    page.drag_cursor(THUMB_X, 60.0, 160.0);
    page.button(ElementState::Released);

    assert_close(page.offset(), 100.0 * CONTENT_PER_THUMB_PIXEL, "dragged");
}

#[test]
fn a_bar_that_is_held_stays_while_the_cursor_is_on_it() {
    let mut page = page();
    page.move_cursor(THUMB_X, 60.0);
    std::thread::sleep(Duration::from_millis(2500));
    page.app.pump_frames(60);

    assert_eq!(page.controller.scroll_bar_opacity(), 1.0);
}

#[test]
fn the_bar_fades_out_by_itself_once_the_scroll_is_over() {
    let mut page = page();
    page.wheel(-10.0);
    assert!(page.controller.scroll_bar_opacity() > 0.0, "the bar shows while scrolling");
    // The pointer leaves, so nothing keeps it.
    page.move_cursor(100.0, 300.0);

    // Frames are delivered only when something asks for one, exactly as in a
    // window: the bar has to ask for the frames that fade it, with no input.
    for _ in 0..400 {
        std::thread::sleep(Duration::from_millis(16));
        if page.app.pump_frames(1) == 0 {
            break;
        }
    }

    assert_eq!(page.controller.scroll_bar_opacity(), 0.0, "hidden with no further input");
    assert!(!page.app.take_redraw_request(), "and the frames stop once it is");
}

#[test]
fn a_finger_holding_the_thumb_drags_it() {
    let mut page = page();
    page.wheel(-10.0);
    page.wait_until_shown();
    let start = page.offset();
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;
    // Well left of the 7-pixel thumb, where a fingertip lands.
    let finger_x = 380.0;

    page.touch(TouchPhase::Started, finger_x, thumb_middle);
    page.drag_finger(finger_x, thumb_middle, thumb_middle + 100.0);
    page.touch(TouchPhase::Ended, finger_x, thumb_middle + 100.0);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "the thumb carries the content",
    );
}

#[test]
fn a_finger_on_content_still_scrolls_it_by_the_finger() {
    let mut page = page();
    page.wheel(-10.0);
    let start = page.offset();

    // Far from the bar: an ordinary drag of the content.
    page.touch(TouchPhase::Started, 100.0, 400.0);
    page.drag_finger(100.0, 400.0, 300.0);
    page.touch(TouchPhase::Ended, 100.0, 300.0);

    // About 20 of the 100 pixels are the distance a finger has to travel before
    // a touch is a drag at all; the thumb would have moved the content by 500.
    assert!(
        page.offset() > start + 60.0 && page.offset() < start + 110.0,
        "content moved by the finger, not the thumb: {} -> {}",
        start,
        page.offset()
    );
}

#[test]
fn a_finger_cannot_take_hold_of_a_thumb_that_is_not_showing() {
    let mut page = page();
    assert_eq!(page.controller.scroll_bar_opacity(), 0.0);

    // Where the thumb would be, with the bar hidden: the content is swiped, not
    // the thumb held.
    page.touch(TouchPhase::Started, 380.0, 300.0);
    page.drag_finger(380.0, 300.0, 200.0);
    page.touch(TouchPhase::Ended, 380.0, 200.0);

    assert!(
        page.offset() > 60.0 && page.offset() < 110.0,
        "an ordinary swipe of the content, got {}",
        page.offset()
    );
}

#[test]
fn the_bar_stays_for_a_moment_after_the_scroll_and_then_goes_by_itself() {
    let mut page = page();
    page.wheel(-10.0);
    page.wait_until_shown();
    page.move_cursor(100.0, 300.0);

    // Long enough to have started fading under a hold of a fraction of a second,
    // and well short of the idle delay.
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(16));
        page.app.pump_frames(2);
    }
    assert_eq!(page.controller.scroll_bar_opacity(), 1.0, "still there half a second on");

    for _ in 0..400 {
        std::thread::sleep(Duration::from_millis(16));
        if page.app.pump_frames(1) == 0 {
            break;
        }
    }
    assert_eq!(page.controller.scroll_bar_opacity(), 0.0, "gone with no further input");
}

#[test]
fn a_thumb_drawn_over_buttons_is_dragged_and_presses_none_of_them() {
    let presses = Rc::new(Cell::new(0));
    let mut page = pressable_page(presses.clone());
    page.wheel(-10.0);
    page.wait_until_shown();
    let start = page.offset();
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;

    page.move_cursor(THUMB_X, thumb_middle);
    page.button(ElementState::Pressed);
    page.drag_cursor(THUMB_X, thumb_middle, thumb_middle + 100.0);
    page.button(ElementState::Released);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "content follows the thumb",
    );
    assert_eq!(presses.get(), 0, "the buttons under the thumb were pressed");
}

#[test]
fn a_finger_holding_a_thumb_over_buttons_drags_it_and_presses_none_of_them() {
    let presses = Rc::new(Cell::new(0));
    let mut page = pressable_page(presses.clone());
    page.wheel(-10.0);
    page.wait_until_shown();
    let start = page.offset();
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;

    page.touch(TouchPhase::Started, 380.0, thumb_middle);
    page.drag_finger(380.0, thumb_middle, thumb_middle + 100.0);
    page.touch(TouchPhase::Ended, 380.0, thumb_middle + 100.0);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "the thumb carries the content",
    );
    assert_eq!(presses.get(), 0, "the buttons under the thumb were pressed");
}

/// On a high-density display the pointer arrives in device pixels while the bar
/// is laid out in logical ones, and the two have to meet at the thumb.
#[test]
fn the_thumb_can_be_dragged_on_a_high_density_display() {
    let mut page = page_at(2.0);
    page.wheel(-10.0);
    page.wait_until_shown();
    let start = page.offset();
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;

    page.move_cursor(THUMB_X, thumb_middle);
    page.button(ElementState::Pressed);
    page.drag_cursor(THUMB_X, thumb_middle, thumb_middle + 100.0);
    page.button(ElementState::Released);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "content follows the thumb",
    );
}

#[test]
fn a_finger_holds_the_thumb_on_a_high_density_display() {
    let mut page = page_at(2.0);
    page.wheel(-10.0);
    page.wait_until_shown();
    let start = page.offset();
    let thumb_middle = f64::from(start / CONTENT_PER_THUMB_PIXEL) + 60.0;

    page.touch(TouchPhase::Started, 380.0, thumb_middle);
    page.drag_finger(380.0, thumb_middle, thumb_middle + 100.0);
    page.touch(TouchPhase::Ended, 380.0, thumb_middle + 100.0);

    assert_close(
        page.offset(),
        start + 100.0 * CONTENT_PER_THUMB_PIXEL,
        "the thumb carries the content",
    );
}

fn assert_bar_shows_when_the_mouse_moves_over_the_content<W: Widget + 'static>(mut page: Page<W>) {
    assert_eq!(page.controller.scroll_bar_opacity(), 0.0);

    // Nowhere near the bar: over the middle of the content.
    page.move_cursor(150.0, 200.0);
    page.move_cursor(160.0, 210.0);
    page.wait_until_shown();

    assert_eq!(page.controller.scroll_bar_opacity(), 1.0);
}

#[test]
fn the_bar_shows_when_the_mouse_moves_over_the_content() {
    assert_bar_shows_when_the_mouse_moves_over_the_content(page());
}

#[test]
fn the_bar_shows_when_the_mouse_moves_over_buttons() {
    assert_bar_shows_when_the_mouse_moves_over_the_content(pressable_page(Rc::new(Cell::new(0))));
}

#[test]
fn a_mouse_resting_over_the_content_lets_the_bar_fade() {
    let mut page = page();
    page.move_cursor(150.0, 200.0);
    page.move_cursor(160.0, 210.0);
    page.wait_until_shown();

    // The mouse stays where it is: nothing is holding the bar.
    for _ in 0..400 {
        std::thread::sleep(Duration::from_millis(16));
        if page.app.pump_frames(1) == 0 {
            break;
        }
    }
    assert_eq!(page.controller.scroll_bar_opacity(), 0.0);
}

#[test]
fn a_finger_does_not_show_the_bar_by_touching_the_content() {
    let mut page = page();
    page.touch(TouchPhase::Started, 150.0, 200.0);
    page.touch(TouchPhase::Ended, 150.0, 200.0);
    std::thread::sleep(Duration::from_millis(300));
    page.app.pump_frames(10);

    assert_eq!(page.controller.scroll_bar_opacity(), 0.0, "a tap is not a scroll");
}
