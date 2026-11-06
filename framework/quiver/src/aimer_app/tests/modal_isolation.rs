//! A modal is a mode: it reaches what the application provides, sizes itself
//! against the window, and keeps the pointer from the page beneath it.

use super::*;

use std::cell::RefCell;

use aimer_attribute::Dimension;
use aimer_container::{Container, SizedBox, ZeroSizedBox};
use aimer_input::button::Button;
use aimer_input::mouse_region::MouseRegion;
use aimer_modal::{Modal, ModalScope};
use aimer_provider::{Provider, ProviderContext, ProviderHandle};
use winit::event::{ElementState, MouseButton};

pub(super) fn options() -> HeadlessOptions {
    HeadlessOptions {
        size: PhysicalSize::new(400, 300),
        scale_factor: 1.0,
    }
}

pub(super) fn move_to<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.pump_frames(2);
}

pub(super) fn click<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, x: f64, y: f64) {
    move_to(app, x, y);
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(2);
    }
}

// ---------------------------------------------------------------------------
// Size

#[test]
fn a_modal_sized_as_a_share_of_the_window_takes_that_share_once() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(SizedBox::new(), options());
    app.pump_frames(2);

    let _handle = Modal::new()
        .child(
            Provider::new().handle(ProviderHandle::new(0_u8)).child(
            Container::new()
                .width(Dimension::Percent(50.0))
                .height(Dimension::Percent(50.0))
                .box_decoration(
                    aimer_style::BoxDecoration::new()
                        .background_color(Color::BLUE)
                        .border_radius(16),
                )
                .child(
                    Column::new().children([
                        aimer_flex::Expanded::new()
                            .box_child(SizedBox::new()),
                    ]),
                ),
        )
            )
        .show();
    app.pump_frames(3);

    let _ = direct_headless_frame_packet(&mut app);
    let content = retained_element_id_named(&app, "Container")
        .expect("the modal content is a framework element");
    let node = app
        .app
        .render_node_for_element(content)
        .expect("the modal content has a render node");
    let operations = app.app.render_tree().render_all();
    let item = operations
        .iter()
        .find_map(|operation| match operation {
            aimer_cupid::draw_cmd::RenderOp::Draw(item) if item.element == node => Some(item),
            _ => None,
        })
        .expect("the modal content is drawn");
    let (x, y) = item.origin;
    let bounds = item.bounds;
    println!("origin {:?} bounds {:?}", item.origin, bounds);
    assert!(
        (bounds.width - 200.0).abs() < 0.5 && (bounds.height - 150.0).abs() < 0.5,
        "half of a 400x300 window is 200x150, not {}x{}",
        bounds.width,
        bounds.height
    );
    assert!(
        (x - 100.0).abs() < 0.5 && (y - 75.0).abs() < 0.5,
        "and it sits in the middle, at (100, 75), not ({x}, {y})"
    );
}

// ---------------------------------------------------------------------------
// Providers

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Greeting(pub(super) &'static str);

thread_local! {
    pub(super) static CAPTURED: RefCell<Option<ModalScope>> = const { RefCell::new(None) };
    pub(super) static SEEN: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Part of the application: takes the scope it is built in.
pub(super) struct Page;

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        CAPTURED.with(|captured| *captured.borrow_mut() = Some(ModalScope::capture(ctx)));
        SizedBox::new().to_element(ctx)
    }
}

/// Content of the modal: reads and watches what the application provides.
pub(super) struct Reader;

impl Widget for Reader {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        aimer_widget::StatelessElement::from_builder(
            ctx,
            |ctx| {
                let greeting = ctx
                    .try_watch::<Greeting>()
                    .map_or("no provider in scope", |greeting| greeting.0);
                SEEN.with(|seen| seen.borrow_mut().push(greeting));
                SizedBox::new().width(40.0).height(40.0).to_element(ctx)
            },
            None,
            "Reader",
        )
        .boxed()
    }
}

#[test]
fn a_modal_given_the_scope_reads_what_the_application_provides() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    SEEN.with(|seen| seen.borrow_mut().clear());
    let greeting = ProviderHandle::new(Greeting("hello"));
    let mut app = AimerApp::start_headless_with(
        Provider::new().handle(greeting.clone()).child(Page),
        options(),
    );
    app.pump_frames(2);
    let scope = CAPTURED
        .with(|captured| captured.borrow().clone())
        .expect("the page took the scope");

    let _handle = Modal::new().scope(scope).child(Reader).show();
    app.pump_frames(3);
    assert_eq!(
        SEEN.with(|seen| seen.borrow().last().copied()),
        Some("hello"),
        "the modal should find the provider of the page it was opened from"
    );
}

#[test]
fn a_modal_given_the_scope_follows_the_provider_it_reads() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    SEEN.with(|seen| seen.borrow_mut().clear());
    let greeting = ProviderHandle::new(Greeting("hello"));
    let mut app = AimerApp::start_headless_with(
        Provider::new().handle(greeting.clone()).child(Page),
        options(),
    );
    app.pump_frames(2);
    let scope = CAPTURED
        .with(|captured| captured.borrow().clone())
        .expect("the page took the scope");
    let _handle = Modal::new().scope(scope).child(Reader).show();
    app.pump_frames(3);

    greeting.update(|value| *value = Greeting("goodbye"));
    app.pump_frames(3);
    assert_eq!(
        SEEN.with(|seen| seen.borrow().last().copied()),
        Some("goodbye"),
        "an update of the provider should rebuild what the modal built from it"
    );
}

// ---------------------------------------------------------------------------
// Events

pub(super) struct Counters {
    pub(super) entered: Cell<usize>,
    pub(super) exited: Cell<usize>,
    pub(super) pressed: Cell<usize>,
}

/// A page that is one big hoverable button.
pub(super) fn page(counters: &Rc<Counters>) -> impl Widget + 'static {
    let (entered, exited, pressed) = (counters.clone(), counters.clone(), counters.clone());
    MouseRegion::new()
        .on_hover_enter(move || entered.entered.set(entered.entered.get() + 1))
        .on_hover_exit(move || exited.exited.set(exited.exited.get() + 1))
        .child(
            Button::new()
                .on_press(move || pressed.pressed.set(pressed.pressed.get() + 1))
                .child(SizedBox::new().width(400.0).height(300.0)),
        )
}

pub(super) fn counters() -> Rc<Counters> {
    Rc::new(Counters {
        entered: Cell::new(0),
        exited: Cell::new(0),
        pressed: Cell::new(0),
    })
}

pub(super) fn open_small_modal() -> aimer_modal::ModalHandle {
    Modal::new()
        .child(SizedBox::new().width(100.0).height(60.0))
        .show()
}

#[test]
fn the_pointer_over_the_barrier_does_not_hover_the_page_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    move_to(&mut app, -10.0, -10.0);
    app.pump_frames(2);
    let _handle = open_small_modal();
    app.pump_frames(3);
    let before = counters.entered.get();

    // Over the barrier, and over the modal's own content.
    move_to(&mut app, 20.0, 20.0);
    move_to(&mut app, 200.0, 150.0);
    assert_eq!(
        counters.entered.get(),
        before,
        "the page was hovered through the modal"
    );
}

#[test]
fn control_the_page_is_hovered_and_pressed_without_a_modal() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    move_to(&mut app, -10.0, -10.0);
    app.pump_frames(2);
    move_to(&mut app, 20.0, 20.0);
    assert_eq!(counters.entered.get(), 1, "the page should be hovered");
    click(&mut app, 20.0, 20.0);
    assert_eq!(counters.pressed.get(), 1, "the page should be pressed");
}

#[test]
fn the_pointer_over_a_button_in_the_modal_does_not_hover_the_page_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    move_to(&mut app, -10.0, -10.0);
    let _handle = Modal::new()
        .child(
            Button::new()
                .on_press(|| {})
                .child(SizedBox::new().width(100.0).height(60.0)),
        )
        .show();
    app.pump_frames(3);

    move_to(&mut app, 20.0, 20.0);
    move_to(&mut app, 200.0, 150.0);
    move_to(&mut app, 210.0, 160.0);
    assert_eq!(counters.entered.get(), 0, "the page was hovered through the modal");
}

#[test]
fn a_modal_opened_under_the_pointer_ends_the_hover_of_the_page() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    move_to(&mut app, -10.0, -10.0);
    move_to(&mut app, 20.0, 20.0);
    assert_eq!(counters.entered.get(), 1, "the page should be hovered first");

    let _handle = open_small_modal();
    app.pump_frames(4);
    assert_eq!(
        counters.exited.get(),
        1,
        "the page is under the barrier now and must stop being hovered"
    );
}

#[test]
fn a_press_on_the_modal_does_not_reach_the_page_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    app.pump_frames(2);
    let handle = open_small_modal();
    app.pump_frames(3);

    click(&mut app, 200.0, 150.0);
    assert_eq!(counters.pressed.get(), 0, "pressing the modal's content pressed the page");

    click(&mut app, 20.0, 20.0);
    assert_eq!(counters.pressed.get(), 0, "pressing the barrier pressed the page");
    assert!(!handle.is_showing(), "pressing the barrier should dismiss the modal");
}

#[test]
fn the_wheel_over_a_modal_does_not_scroll_the_page_beneath() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut app = AimerApp::start_headless(ScrollRecordingWidget {
        events: events.clone(),
    });
    app.render_frame();
    app.app.cursor_pos = Vec2d { x: 20.0, y: 20.0 };

    // Control: with no modal the page gets the wheel.
    app.send_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -200.0)),
        phase: TouchPhase::Moved,
    });
    // The wheel glides for as many frames as the smoother needs, and a frame
    // that is quicker than its floor step does not shorten that: run the glide
    // out, or its tail lands after the log is cleared and looks like a leak.
    let mut frames = 0;
    while !events.lock().unwrap().iter().any(|(_, _, phase)| {
        matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
    }) {
        frames += 1;
        assert!(frames <= 500, "the wheel glide never ended");
        app.render_frame();
    }
    assert!(!events.lock().unwrap().is_empty(), "the page should be scrolled without a modal");
    events.lock().unwrap().clear();

    let _handle = open_small_modal();
    app.pump_frames(3);
    for position in [Vec2d { x: 20.0, y: 20.0 }, Vec2d { x: 200.0, y: 150.0 }] {
        app.app.cursor_pos = position;
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -200.0)),
            phase: TouchPhase::Moved,
        });
        for _ in 0..20 {
            app.render_frame();
        }
    }
    assert!(
        events.lock().unwrap().is_empty(),
        "the page was scrolled through the modal: {:?}",
        events.lock().unwrap().len()
    );
}

// ---------------------------------------------------------------------------
// Keyboard

use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_input::TextEditingController;
use aimer_input::input::TextField;

pub(super) fn press(app: &mut HeadlessAimerApp<impl Widget + 'static>, key: NamedKey) {
    app.send_user_event(AimerNativePlatformEvent::NativeInput {
        event: ElementEvent::KeyInput {
            key,
            action: KeyAction::Pressed,
            modifiers: Modifiers::default(),
        },
        modifiers: Modifiers::default(),
    });
    app.pump_frames(2);
}

pub(super) fn type_into(app: &mut HeadlessAimerApp<impl Widget + 'static>, text: &str) {
    super::text_field_caret::type_text(app, text);
    app.pump_frames(2);
}

/// A page that is a text field with the keyboard, holding "ab".
pub(super) fn page_with_focused_field() -> (HeadlessAimerApp<impl Widget + 'static>, TextEditingController) {
    let controller = TextEditingController::with_text("");
    let mut app = AimerApp::start_headless_with(
        Column::new().children([TextField::new()
            .controller(controller.clone())
            .max_length(None)
            .boxed()]),
        options(),
    );
    app.pump_frames(3);
    super::text_field_caret::click(&mut app, 100.0, 12.0);
    app.pump_frames(3);
    type_into(&mut app, "ab");
    assert_eq!(controller.value().text(), "ab", "the field should take the keyboard before any modal");
    (app, controller)
}

#[test]
fn typing_with_a_modal_open_does_not_reach_the_focused_field_of_the_page() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut app, controller) = page_with_focused_field();
    let _handle = open_small_modal();
    app.pump_frames(3);

    type_into(&mut app, "cd");
    assert_eq!(controller.value().text(), "ab", "text was typed into the page through the modal");
    press(&mut app, NamedKey::Backspace);
    assert_eq!(controller.value().text(), "ab", "a key edited the page through the modal");
    app.send_user_event(AimerNativePlatformEvent::ForceBackspace);
    app.pump_frames(2);
    assert_eq!(controller.value().text(), "ab", "a backspace edited the page through the modal");
}

#[test]
fn the_focused_field_of_the_page_gets_the_keyboard_back_when_the_modal_closes() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut app, controller) = page_with_focused_field();
    let handle = open_small_modal();
    app.pump_frames(3);
    type_into(&mut app, "cd");

    handle.dismiss();
    app.pump_frames(4);
    type_into(&mut app, "e");
    assert_eq!(
        controller.value().text(),
        "abe",
        "the field should have the keyboard again once the modal is gone"
    );
}
