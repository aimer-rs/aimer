//! A tab bar's container can be padded, given margin, sized, placed and
//! decorated, and its items and indicator stay where the pointer expects them.
//!
//! These tap through a headless window, so what they check is the layout a
//! user would get: a tap on an item's centre must select that item wherever the
//! container put it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aimer::decoration::BoxDecoration;
use aimer::indicator::{TabBar, TabItem};
use aimer::modal::ModalHost;
use aimer::quiver::aimer_app::HeadlessAimerApp;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::Alignment;
use aimer::style::LayoutSpacing;
use aimer::widget::base::Color;
use aimer::{AimerApp, HeadlessOptions};

const ITEM_WIDTH: f64 = 96.0;
const ITEM_HEIGHT: f64 = 56.0;
const WINDOW: (f64, f64) = (400.0, 200.0);

struct Window {
    app: HeadlessAimerApp<ModalHost<TabBar>>,
    selections: Rc<RefCell<Vec<usize>>>,
}

/// Shows `bar` with three items in a 400 by 200 window.
fn show(bar: TabBar) -> Window {
    let selections = Rc::new(RefCell::new(Vec::new()));
    let recorded = selections.clone();
    let widget = bar
        .on_select(move |index| recorded.borrow_mut().push(index))
        .item(TabItem::new().label("Recents"))
        .item(TabItem::new().label("Shared"))
        .item(TabItem::new().label("Browse"));
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(WINDOW.0 as u32, WINDOW.1 as u32),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    Window { app, selections }
}

impl Window {
    fn tap(&mut self, x: f64, y: f64) {
        // The recognizer merges two taps within 300 ms into a double tap.
        std::thread::sleep(Duration::from_millis(350));
        self.app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        for state in [ElementState::Pressed, ElementState::Released] {
            self.app.send_window_event(WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button: MouseButton::Left,
            });
            self.app.render_frame();
        }
    }

    /// Taps the centre of item `index` in a bar whose items start at `(left, top)`.
    fn tap_item(&mut self, left: f64, top: f64, index: usize) {
        self.tap(left + ITEM_WIDTH * (index as f64 + 0.5), top + ITEM_HEIGHT / 2.0);
    }

    fn selections(&self) -> Vec<usize> {
        self.selections.borrow().clone()
    }
}

#[test]
fn the_default_bar_keeps_its_items_six_pixels_in() {
    let mut window = show(TabBar::new());
    window.tap_item(6.0, 6.0, 2);
    assert_eq!(window.selections(), [2]);
}

#[test]
fn padding_moves_the_items_and_the_indicator_follows() {
    let mut window = show(TabBar::new().bar_padding(LayoutSpacing::new().left(30).top(14).right(4).bottom(4)));
    window.tap_item(30.0, 14.0, 1);
    window.tap_item(30.0, 14.0, 2);
    assert_eq!(window.selections(), [1, 2]);
}

#[test]
fn margin_moves_the_whole_bar() {
    let mut window = show(TabBar::new().bar_margin(LayoutSpacing::all(20)));
    // Margin, then the default padding. Item 0 is already selected, so tap 1.
    window.tap_item(26.0, 26.0, 1);
    assert_eq!(window.selections(), [1]);
}

#[test]
fn alignment_places_the_bar_within_the_window() {
    let mut window = show(TabBar::new().bar_alignment(Alignment::BotRight));
    let width = ITEM_WIDTH * 3.0 + 12.0;
    let height = ITEM_HEIGHT + 12.0;
    window.tap_item(WINDOW.0 - width + 6.0, WINDOW.1 - height + 6.0, 2);
    assert_eq!(window.selections(), [2]);
}

#[test]
fn a_wider_bar_leaves_the_extra_room_after_the_last_item() {
    let mut window = show(TabBar::new().bar_width(380));
    window.tap_item(6.0, 6.0, 2);
    // Past the last item, still inside the bar: nothing to select.
    window.tap(6.0 + ITEM_WIDTH * 3.0 + 30.0, 6.0 + ITEM_HEIGHT / 2.0);
    assert_eq!(window.selections(), [2]);
}

#[test]
fn a_custom_decoration_and_unclipped_content_keep_the_items_where_they_were() {
    let decoration = BoxDecoration::new()
        .background_color(Color::Rgba(20, 20, 24, 255))
        .border_radius(10);
    let mut window = show(TabBar::new().bar_decoration(decoration).clip_content(false));
    window.tap_item(6.0, 6.0, 1);
    assert_eq!(window.selections(), [1]);
}

#[test]
fn a_vertical_bar_stacks_its_items_and_taps_follow_the_column() {
    let mut window = show(TabBar::new().vertical());
    // Items are 96 by 56, one under the other, inside the default padding.
    for index in 1..3 {
        window.tap(6.0 + ITEM_WIDTH / 2.0, 6.0 + ITEM_HEIGHT * (index as f64 + 0.5));
    }
    assert_eq!(window.selections(), [1, 2]);
}

#[test]
fn a_vertical_bar_has_no_items_beside_the_column() {
    let mut window = show(TabBar::new().vertical());
    // Where the second item of a horizontal bar would be: past the column's width.
    window.tap(6.0 + ITEM_WIDTH * 1.5, 6.0 + ITEM_HEIGHT * 2.5);
    assert_eq!(window.selections(), Vec::<usize>::new());
}

#[test]
fn a_vertical_bar_is_placed_and_sized_like_a_horizontal_one() {
    let mut window = show(TabBar::new().vertical().item_size(56.0, 56.0).bar_alignment(Alignment::BotRight));
    let width = 56.0 + 12.0;
    let height = 56.0 * 3.0 + 12.0;
    window.tap(WINDOW.0 - width + 6.0 + 28.0, WINDOW.1 - height + 6.0 + 56.0 * 2.5);
    assert_eq!(window.selections(), [2]);
}
