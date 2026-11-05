//! A floating panel and an element hosted by the overlay keep to the rules of a
//! modal: they reach what the application provides, take a share of the window
//! once, and, where they are a mode, keep the pointer and the keyboard from the
//! page beneath.

use super::modal_isolation::{
    Greeting, Page, Reader, CAPTURED, SEEN, counters, options, page, page_with_focused_field,
    move_to, type_into,
};
use super::*;

use aimer_attribute::Dimension;
use aimer_container::{Container, SizedBox};
use aimer_modal::{Floating, ModalScope, OverlayLayer};
use aimer_provider::{Provider, ProviderHandle};

/// A tree under a provider, as the dialog of an application is, whose root is a
/// container sized as half of what it is given.
fn half_of_the_window() -> impl Widget + 'static {
    Provider::new().handle(ProviderHandle::new(0_u8)).child(
        Container::new()
            .width(Dimension::Percent(50.0))
            .height(Dimension::Percent(50.0))
            .box_decoration(
                aimer_style::BoxDecoration::new()
                    .background_color(Color::BLUE)
                    .border_radius(16),
            )
            .child(Column::new().children([aimer_flex::Expanded::new().box_child(SizedBox::new())])),
    )
}

/// The sizes of what the overlay paints that is not the whole window: the
/// content, and not a barrier. What an overlay hosts is not walked with the
/// page, so it is found by what is drawn; the follower, the provider and the
/// container each have a node, and all of them are the size of the content.
fn painted_sizes_of_hosted(app: &mut HeadlessAimerApp<impl Widget + 'static>) -> Vec<(f32, f32)> {
    let _ = direct_headless_frame_packet(app);
    let sizes: Vec<_> = app
        .app
        .render_tree()
        .render_all()
        .iter()
        .filter_map(|operation| match operation {
            aimer_cupid::draw_cmd::RenderOp::Draw(item)
                if item.bounds.width > 1.0 && item.bounds.width < 399.0 =>
            {
                Some((item.bounds.width, item.bounds.height))
            }
            _ => None,
        })
        .collect();
    assert!(!sizes.is_empty(), "the hosted content is drawn");
    sizes
}

/// The size the container of the overlay is painted at.
fn painted_size(app: &mut HeadlessAimerApp<impl Widget + 'static>) -> (f32, f32) {
    let _ = direct_headless_frame_packet(app);
    let content = retained_element_id_named(app, "Container")
        .expect("the overlay content is a framework element");
    let node = app
        .app
        .render_node_for_element(content)
        .expect("the overlay content has a render node");
    let operations = app.app.render_tree().render_all();
    let item = operations
        .iter()
        .find_map(|operation| match operation {
            aimer_cupid::draw_cmd::RenderOp::Draw(item) if item.element == node => Some(item),
            _ => None,
        })
        .expect("the overlay content is drawn");
    (item.bounds.width, item.bounds.height)
}

fn assert_half_of_the_window(size: (f32, f32)) {
    assert!(
        (size.0 - 200.0).abs() < 0.5 && (size.1 - 150.0).abs() < 0.5,
        "half of a 400x300 window is 200x150, not {}x{}",
        size.0,
        size.1
    );
}

fn started_with_scope() -> (HeadlessAimerApp<impl Widget + 'static>, ProviderHandle<Greeting>, ModalScope) {
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
    (app, greeting, scope)
}

fn last_seen() -> Option<&'static str> {
    SEEN.with(|seen| seen.borrow().last().copied())
}

// ---------------------------------------------------------------------------
// Floating

#[test]
fn a_floating_panel_sized_as_a_share_of_the_window_takes_that_share_once() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(SizedBox::new(), options());
    app.pump_frames(2);

    let _handle = Floating::new().child(half_of_the_window()).show();
    app.pump_frames(3);
    assert_half_of_the_window(painted_size(&mut app));
}

#[test]
fn a_floating_panel_given_the_scope_reads_and_follows_what_the_application_provides() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut app, greeting, scope) = started_with_scope();

    let _handle = Floating::new().scope(scope).child(Reader).show();
    app.pump_frames(3);
    assert_eq!(last_seen(), Some("hello"), "the panel should find the provider of its page");

    greeting.update(|value| *value = Greeting("goodbye"));
    app.pump_frames(3);
    assert_eq!(last_seen(), Some("goodbye"), "and follow it");
}

#[test]
fn the_page_stops_being_hovered_under_a_floating_panel() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let counters = counters();
    let mut app = AimerApp::start_headless_with(page(&counters), options());
    move_to(&mut app, -10.0, -10.0);
    move_to(&mut app, 20.0, 20.0);
    assert_eq!(counters.entered.get(), 1, "the page should be hovered first");

    let _handle = Floating::new()
        .child(SizedBox::new().width(100.0).height(60.0))
        .show();
    app.pump_frames(4);
    assert_eq!(counters.exited.get(), 1, "the page is under the panel's barrier");
    let entered = counters.entered.get();

    move_to(&mut app, 30.0, 30.0);
    move_to(&mut app, 200.0, 150.0);
    assert_eq!(counters.entered.get(), entered, "the page was hovered through the panel");
}

#[test]
fn typing_with_a_floating_panel_open_does_not_reach_the_focused_field_of_the_page() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (mut app, controller) = page_with_focused_field();
    let handle = Floating::new()
        .child(SizedBox::new().width(100.0).height(60.0))
        .show();
    app.pump_frames(3);

    type_into(&mut app, "cd");
    assert_eq!(controller.value().text(), "ab", "text was typed into the page through the panel");

    handle.dismiss();
    app.pump_frames(4);
    type_into(&mut app, "e");
    assert_eq!(controller.value().text(), "abe", "and the page has the keyboard back after it");
}

// ---------------------------------------------------------------------------
// Elements hosted by the overlay

/// Hosts what `make` builds, once, from inside a build, as drag feedback is.
struct Hoster<F: Fn(&BuildContext) -> AnyElement + 'static> {
    make: F,
    scope: bool,
    hosted: Rc<Cell<bool>>,
}

impl<F: Fn(&BuildContext) -> AnyElement + 'static> Widget for Hoster<F> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        if !self.hosted.replace(true) {
            let element = (self.make)(ctx);
            let position = Rc::new(|| Vec2d { x: 0.0, y: 0.0 });
            if self.scope {
                OverlayLayer::host_element_in(element, position, ModalScope::capture(ctx));
            } else {
                OverlayLayer::host_element(element, position);
            }
        }
        SizedBox::new().to_element(ctx)
    }
}

#[test]
fn an_element_hosted_by_the_overlay_sized_as_a_share_of_the_window_takes_that_share_once() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut app = AimerApp::start_headless_with(
        Hoster {
            make: |ctx| half_of_the_window().to_element(ctx),
            scope: false,
            hosted: Rc::new(Cell::new(false)),
        },
        options(),
    );
    app.pump_frames(4);
    for size in painted_sizes_of_hosted(&mut app) {
        assert_half_of_the_window(size);
    }
}

#[test]
fn an_element_hosted_with_the_scope_follows_what_the_application_provides() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    SEEN.with(|seen| seen.borrow_mut().clear());
    let greeting = ProviderHandle::new(Greeting("hello"));
    let mut app = AimerApp::start_headless_with(
        Provider::new().handle(greeting.clone()).child(Hoster {
            make: |ctx| Reader.to_element(ctx),
            scope: true,
            hosted: Rc::new(Cell::new(false)),
        }),
        options(),
    );
    app.pump_frames(4);
    assert_eq!(last_seen(), Some("hello"));

    greeting.update(|value| *value = Greeting("goodbye"));
    app.pump_frames(4);
    assert_eq!(
        last_seen(),
        Some("goodbye"),
        "what the overlay hosts should follow the provider of the scope it was given"
    );
}
