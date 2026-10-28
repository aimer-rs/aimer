//! A vertical tab bar's indicator follows the pointer up and down the column
//! when it is dragged, and lands on the nearest item when let go.
//!
//! These drive real mouse events through a headless window, as
//! `tab_bar_drag.rs` does for the horizontal bar.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aimer::indicator::{IndicatorController, TabBar, TabItem};
use aimer::modal::ModalHost;
use aimer::quiver::aimer_app::HeadlessAimerApp;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{AimerApp, HeadlessOptions};

const ITEM: f64 = 56.0;
/// The bar pads its items by this much on every side.
const PADDING: f64 = 6.0;
const COLUMN_X: f64 = PADDING + ITEM / 2.0;

/// The vertical centre of item `index`.
fn centre(index: f64) -> f64 {
    PADDING + ITEM * index + ITEM / 2.0
}

struct Bar {
    app: HeadlessAimerApp<ModalHost<TabBar>>,
    controller: IndicatorController,
    selections: Rc<RefCell<Vec<usize>>>,
}

fn bar(selected: usize) -> Bar {
    let controller = IndicatorController::new(selected as f32);
    let selections = Rc::new(RefCell::new(Vec::new()));
    let recorded = selections.clone();
    let widget = TabBar::new()
        .vertical()
        .item_size(ITEM as f32, ITEM as f32)
        .controller(controller.clone())
        .selected(selected)
        .on_select(move |index| recorded.borrow_mut().push(index))
        .item(TabItem::new().label("A"))
        .item(TabItem::new().label("B"))
        .item(TabItem::new().label("C"));
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions { size: PhysicalSize::new(200, 300), scale_factor: 1.0 },
    );
    app.pump_frames(3);
    Bar { app, controller, selections }
}

impl Bar {
    fn move_to(&mut self, y: f64) {
        self.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(COLUMN_X, y),
        });
    }

    fn button(&mut self, state: ElementState) {
        self.app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        self.app.render_frame();
    }

    fn run_for(&mut self, millis: u64) {
        for _ in 0..millis / 16 {
            self.app.render_frame();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn press(&mut self, y: f64) {
        // The recognizer merges two taps within 300 ms into a double tap.
        std::thread::sleep(Duration::from_millis(350));
        self.move_to(y);
        self.button(ElementState::Pressed);
    }

    /// Moves the pointer down or up to `to` in small steps, as a hand does.
    fn drag_to(&mut self, from: f64, to: f64) {
        let steps = 12;
        for step in 1..=steps {
            self.move_to(from + (to - from) * f64::from(step) / f64::from(steps));
            self.app.render_frame();
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    fn selections(&self) -> Vec<usize> {
        self.selections.borrow().clone()
    }
}

#[test]
fn dragging_down_the_column_carries_the_indicator_and_selects_where_it_was_let_go() {
    let mut bar = bar(0);

    bar.press(centre(0.0));
    bar.drag_to(centre(0.0), centre(2.0));

    assert!(bar.controller.is_dragging());
    assert!((bar.controller.target() - 2.0).abs() < 0.05, "target {}", bar.controller.target());
    assert_eq!(bar.selections(), [] as [usize; 0]);
    bar.run_for(400);
    assert!(bar.controller.lift() > 0.9, "the lens did not lift: {}", bar.controller.lift());

    bar.button(ElementState::Released);
    assert!(!bar.controller.is_dragging());
    assert_eq!(bar.selections(), [2]);
}

#[test]
fn dragging_up_the_column_goes_back_to_an_earlier_item() {
    let mut bar = bar(2);

    bar.press(centre(2.0));
    bar.drag_to(centre(2.0), centre(0.0));
    bar.button(ElementState::Released);

    assert_eq!(bar.selections(), [0]);
}
