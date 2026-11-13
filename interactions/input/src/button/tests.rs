use aimer_widget::BuildElement;
use aimer_widget::CreateState;
use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::size::ResolvedSize;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_style::BoxDecoration;
use aimer_widget::base::{Color, WindowHandle};
use super::ButtonState;
use aimer_widget::{
    AnyElement, Drawable, Element, ErrorWidget, EventElement, Key, LayoutElement, Rebuildable,
    State, VisitorElement, Widget, dispatch_event,
};

use super::{BuildContext, Button};

/// A child that reports how often it was asked for an element.
struct Probe {
    builds: Rc<Cell<usize>>,
}

impl Widget for Probe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.builds.set(self.builds.get() + 1);
        ErrorWidget::new("probe").to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "Probe"
    }
}

struct EventProbe {
    events: Rc<Cell<usize>>,
}

impl Widget for EventProbe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        EventProbeElement {
            events: self.events,
        }
        .boxed()
    }
}

struct EventProbeElement {
    events: Rc<Cell<usize>>,
}

impl VisitorElement for EventProbeElement {
    fn debug_name(&self) -> &'static str {
        "ButtonEventProbe"
    }
}
impl EventElement for EventProbeElement {
    fn on_event(&self, _event: &ElementEvent) -> aimer_widget::EventResult {
        self.events.set(self.events.get() + 1);
        aimer_widget::EventResult::ignored()
    }
}
impl LayoutElement for EventProbeElement {}
impl Drawable for EventProbeElement {
    fn update(&self, _ctx: &BuildContext<'_>) {}
}
impl Rebuildable for EventProbeElement {}

fn context() -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize::default(),
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        tokio::runtime::Handle::current(),
    )
}

/// Rebuilding a button's configuration must reach the same retained child
/// again, preserving the child's own state rather than constructing it anew.
#[tokio::test]
async fn every_rebuild_reaches_the_same_child() {
    let builds = Rc::new(Cell::new(0));
    let state = Button::new()
        .child(Probe {
            builds: Rc::clone(&builds),
        })
        .create_state::<ButtonState<_>>();
    let ctx = context();

    state.build_element(&ctx);
    state.build_element(&ctx);

    assert_eq!(
        builds.get(),
        1,
        "a hover or a press must reuse the child, not rebuild it"
    );
}

#[tokio::test]
async fn button_blocks_events_from_its_child_by_default() {
    let events = Rc::new(Cell::new(0));
    let state = Button::new()
        .child(EventProbe {
            events: events.clone(),
        })
        .create_state::<ButtonState<_>>();
    let ctx = context();
    let element = state.build_element(&ctx);

    let pointer = PointerInfo::mouse(
        aimer_widget::base::Vec2d { x: 1.0, y: 1.0 },
        PointerButton::Primary,
    );
    let _ = dispatch_event(
        element.as_ref(),
        pointer.pos,
        &ElementEvent::PointerDown(pointer),
    );

    assert_eq!(events.get(), 0);
}

/// An asynchronous press handler may keep an [`Rc`] from the element tree
/// across an `await`, and its effect lands on the runtime that owns the
/// frame.
///
/// This is the whole reason the async path moved off a thread pool: a
/// handler that cannot capture a `StateUpdater` cannot change anything a
/// user would see. The bound is easy to re-tighten by accident, so it is
/// pinned here rather than left to a compile that happens to still work.
#[tokio::test]
async fn an_async_press_handler_may_hold_tree_state_across_an_await() {
    use aimer_utils::callback::CallbackExecutor;
    use aimer_venus::Venus;

    let venus = Venus::new();
    venus.install();

    let presses = Rc::new(Cell::new(0));
    let counted = Rc::clone(&presses);
    let button = Button::new()
        .child(aimer_widget::ErrorWidget::new("button"))
        .on_press_async(move || async move {
            aimer_venus::yield_now().await;
            counted.set(counted.get() + 1);
        });

    button.on_press.execute(());
    while venus.task_count() > 0 {
        venus.run_microtasks();
    }

    assert_eq!(presses.get(), 1);
}

#[test]
fn explicit_key_sets_reconciliation_identity() {
    let button = Button::new()
        .child(aimer_widget::ErrorWidget::new("button"))
        .key("platform-button");

    assert_eq!(
        Widget::key(&button),
        Some(Key::Value("platform-button".to_owned()))
    );
}

#[test]
fn state_specific_decorations_are_transferred_to_button_state() {
    let normal = BoxDecoration::new().background_color(Color::WHITE);
    let hover = BoxDecoration::new().background_color(Color::BLUE);
    let press = BoxDecoration::new().background_color(Color::RED);
    let disabled = BoxDecoration::new().background_color(Color::GRAY);

    let state = Button::new()
        .decoration(normal.clone())
        .hover_decoration(hover.clone())
        .press_decoration(press.clone())
        .disable_decoration(disabled.clone())
        .child(aimer_widget::ErrorWidget::new("button"))
        .create_state::<ButtonState<_>>();

    assert_eq!(state.decoration, normal);
    assert_eq!(state.hover_decoration, Some(hover));
    assert_eq!(state.press_decoration, Some(press));
    assert_eq!(state.disable_decoration, Some(disabled));
}

#[test]
fn state_specific_decorations_follow_disabled_pressed_hover_precedence() {
    let normal = BoxDecoration::new().background_color(Color::WHITE);
    let hover = BoxDecoration::new().background_color(Color::BLUE);
    let press = BoxDecoration::new().background_color(Color::RED);
    let disabled = BoxDecoration::new().background_color(Color::GRAY);
    let mut state = Button::new()
        .decoration(normal.clone())
        .hover_decoration(hover.clone())
        .press_decoration(press.clone())
        .disable_decoration(disabled.clone())
        .child(aimer_widget::ErrorWidget::new("button"))
        .create_state::<ButtonState<_>>();

    state.is_disabled = true;
    state.is_hover.set(true);
    state.is_pressed.set(true);
    assert_eq!(state.active_decoration(), disabled);

    state.is_disabled = false;
    assert_eq!(state.active_decoration(), press);

    state.is_pressed.set(false);
    assert_eq!(state.active_decoration(), hover);

    state.is_hover.set(false);
    assert_eq!(state.active_decoration(), normal);
}

mod retained;
mod custom;
mod no_decor;
mod gestures;
mod semantics;
