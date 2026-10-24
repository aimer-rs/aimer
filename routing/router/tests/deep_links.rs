use aimer_events::link::deliver_link;
use aimer_router::*;
use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
use aimer_widget::{AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement,
    PortableWidget, Rebuildable, VisitorElement, Widget};

#[derive(Clone, Debug, PartialEq)]
enum Screen { Home, Topic(u32), Settings }

impl Route for Screen {
    fn parse(path: &str) -> Option<Self> {
        let (path, _query) = split_path_query(path);
        match path.trim_matches('/').split('/').collect::<Vec<_>>().as_slice() {
            [""] => Some(Self::Home),
            ["settings"] => Some(Self::Settings),
            ["topic", id] => id.parse().ok().map(Self::Topic),
            _ => None,
        }
    }
    fn format(&self) -> String {
        match self {
            Self::Home => "/".into(),
            Self::Settings => "/settings".into(),
            Self::Topic(id) => format!("/topic/{id}"),
        }
    }
}

impl Router for Screen {
    fn build(&self, _: &BuildContext) -> AnyWidget { Widget::boxed(Leaf(self.clone())) }
}

struct Leaf(Screen);
impl PortableWidget for Leaf {}
impl Widget for Leaf {
    fn to_element(self, _: &BuildContext) -> AnyElement { Element::boxed(self) }
}
impl VisitorElement for Leaf { fn debug_name(&self) -> &'static str { "ScreenPage" } }
impl Drawable for Leaf { fn can_paint_local_v2(&self, _: &BuildContext) -> bool { true } }
impl LayoutElement for Leaf {}
impl Rebuildable for Leaf {}
impl EventElement for Leaf {}

fn routes(screen: Screen) -> AnyWidget { Widget::boxed(Leaf(screen)) }

fn context() -> BuildContext<'static> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
    let canvas = aimer_canvas::FrameCanvas::new(Box::leak(Box::new(aimer_canvas::InnerCanvas::new())));
    BuildContext::new(canvas, ResolvedSize { width: 200.0, height: 100.0 }, 1.0,
        Default::default(), Default::default(), WindowHandle::headless(Default::default(), 1.0), runtime.handle().clone())
}

fn app(ctx: &BuildContext<'static>) -> (AnyElement, NavigatorController<Screen>) {
    let navigator = Navigator::new(Screen::Home, routes).to_element(ctx);
    let controller = NavigatorController::<Screen>::of(ctx);
    (navigator, controller)
}

#[test]
fn a_link_opens_the_linked_page_above_home() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    controller.push(Screen::Settings);
    navigator.rebuild_if_dirty(&ctx);

    assert!(deliver_link("myapp://topic/42"));
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(controller.routes(), [Screen::Home, Screen::Topic(42)], "back from a deep link goes home");
    assert_eq!(controller.current_route(), Screen::Topic(42));
}

#[test]
fn a_web_link_is_matched_on_its_path_and_a_query_survives() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    assert!(deliver_link("https://example.com/settings?tab=2#top"));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.routes(), [Screen::Home, Screen::Settings]);
}

#[test]
fn a_link_to_the_first_page_leaves_just_that_page() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    controller.push(Screen::Settings);
    navigator.rebuild_if_dirty(&ctx);
    assert!(deliver_link("myapp:///"));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.routes(), [Screen::Home]);
}

#[test]
fn a_link_no_route_recognises_changes_nothing() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    controller.push(Screen::Settings);
    navigator.rebuild_if_dirty(&ctx);
    assert!(!deliver_link("myapp://nowhere/at/all"));
    assert!(!controller.open_url("not a link"));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.routes(), [Screen::Home, Screen::Settings]);
}

#[test]
fn an_app_can_open_a_link_itself_through_the_controller() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    assert!(controller.open_url("/topic/9"));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Screen::Topic(9));
}

#[test]
fn the_link_that_launched_the_app_is_the_first_thing_a_navigator_shows() {
    // It arrived before any navigator existed.
    assert!(!deliver_link("myapp://topic/7"));

    let ctx = context();
    let (_navigator, controller) = app(&ctx);
    assert_eq!(controller.routes(), [Screen::Home, Screen::Topic(7)]);

    // It was claimed, so a navigator built later starts where it normally does.
    let ctx = context();
    let (_other, other) = app(&ctx);
    assert_eq!(other.routes(), [Screen::Home]);
}

#[test]
fn a_launch_link_this_navigator_cannot_parse_is_left_for_one_that_can() {
    assert!(!deliver_link("myapp://nowhere"));
    let ctx = context();
    let (_navigator, controller) = app(&ctx);
    assert_eq!(controller.routes(), [Screen::Home]);
    // Still waiting: it was not this navigator's to consume.
    assert!(aimer_events::link::claim_waiting_link(|url| url == "myapp://nowhere"));
}

#[test]
fn back_after_a_deep_link_returns_to_the_first_page() {
    let ctx = context();
    let (navigator, controller) = app(&ctx);
    deliver_link("myapp://topic/42");
    navigator.rebuild_if_dirty(&ctx);
    assert!(aimer_events::back::request_back());
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Screen::Home);
}

#[test]
fn a_navigator_that_is_gone_no_longer_opens_links() {
    let ctx = context();
    let (navigator, _controller) = app(&ctx);
    drop(navigator);
    assert!(!deliver_link("myapp://topic/42"));
}
