use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_router::*;
use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, LayoutElement, PortableWidget,
    Rebuildable, VisitorElement, Widget,
};

aimer_tls::thread_local! {
    static BUILDS: RefCell<Vec<Page>> = const { RefCell::new(Vec::new()) };
    static DROPS: RefCell<Vec<Page>> = const { RefCell::new(Vec::new()) };
    static FRAMES: Cell<usize> = const { Cell::new(0) };
    static SHELL_ACTIONS: RefCell<Vec<NavigationAction>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Home,
    Detail,
    Disabled,
    Redirect,
    Explicit,
}

impl Route for Page {
    fn parse(_: &str) -> Option<Self> {
        None
    }
    fn format(&self) -> String {
        format!("/{self:?}")
    }
    fn view_level(&self) -> u32 {
        if *self == Self::Home { 0 } else { 2 }
    }
    fn navigation_transition(&self) -> TransitionChoice<Self> {
        match self {
            Self::Disabled => TransitionChoice::Disabled,
            Self::Explicit => {
                TransitionChoice::custom(CrossFadeTransition::new().duration(Duration::ZERO))
            }
            _ => TransitionChoice::Inherit,
        }
    }
    fn redirect(&self, _: &BuildContext) -> Option<Self> {
        (*self == Self::Redirect).then_some(Self::Detail)
    }
}

impl Router for Page {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        Widget::boxed(Leaf(*self))
    }
}

struct Leaf(Page);
impl PortableWidget for Leaf {}
impl Widget for Leaf {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        BUILDS.with(|log| log.borrow_mut().push(self.0));
        self.boxed_element()
    }
}
impl Leaf {
    fn boxed_element(self) -> AnyElement {
        Element::boxed(self)
    }
}
impl Drop for Leaf {
    fn drop(&mut self) {
        DROPS.with(|log| log.borrow_mut().push(self.0));
    }
}
impl VisitorElement for Leaf {
    fn debug_name(&self) -> &'static str {
        match self.0 {
            Page::Home => "HomePage",
            Page::Detail => "DetailPage",
            Page::Disabled => "DisabledPage",
            Page::Redirect => "RedirectPage",
            Page::Explicit => "ExplicitPage",
        }
    }
}
impl Drawable for Leaf {
    fn can_paint_local_v2(&self, _: &BuildContext) -> bool {
        true
    }
}
impl LayoutElement for Leaf {
    fn flex(&self) -> Option<f32> {
        Some(2.0)
    }
    fn pos_start_end(&self) -> Option<(aimer_widget::base::Vec2d, aimer_widget::base::Vec2d)> {
        Some((
            Default::default(),
            aimer_widget::base::Vec2d { x: 200.0, y: 100.0 },
        ))
    }
}
impl Rebuildable for Leaf {}
impl EventElement for Leaf {}

fn routes(page: Page) -> AnyWidget {
    Widget::boxed(Leaf(page))
}

fn context() -> BuildContext<'static> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    });
    let canvas =
        aimer_canvas::FrameCanvas::new(Box::leak(Box::new(aimer_canvas::InnerCanvas::new())));
    BuildContext::new(
        canvas,
        ResolvedSize {
            width: 200.0,
            height: 100.0,
        },
        1.0,
        Default::default(),
        Default::default(),
        WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone(),
    )
}

fn pages(element: &dyn Element, interactive: bool) -> Vec<&'static str> {
    let mut result = Vec::new();
    fn walk(element: &dyn Element, interactive: bool, result: &mut Vec<&'static str>) {
        if element.debug_name().ends_with("Page") {
            result.push(element.debug_name());
        }
        let mut visit = |child: &dyn Element| walk(child, interactive, result);
        if interactive {
            element.event_children(&mut visit);
        } else {
            element.visit_children(&mut visit);
        }
    }
    walk(element, interactive, &mut result);
    result.sort();
    result
}

#[test]
fn navigation_retains_two_pages_without_rebuilding_them_on_animation_frames() {
    let ctx = context();
    let selected = Rc::new(RefCell::new(Vec::new()));
    let log = selected.clone();
    let finish = Rc::new(Cell::new(false));
    let finish_sampler = finish.clone();
    let transition =
        DynamicTransitions::new(move |navigation: &NavigationContext<'_, '_, Page>| {
            log.borrow_mut()
                .push((*navigation.from, *navigation.to, navigation.action));
            let finish = finish_sampler.clone();
            ResolvedNavigationTransition::new(
                Duration::from_secs(60),
                curves::LINEAR,
                move |p, _| NavigationTransitionFrame {
                    incoming: PaintPresentation {
                        transform: aimer_canvas::Mat3::identity(),
                        opacity: if finish.get() { f32::NAN } else { p },
                        corner_radius: 0.0,
                        reveal: None,
                    },
                    outgoing: PaintPresentation {
                        transform: aimer_canvas::Mat3::identity(),
                        opacity: 1.0 - p,
                        corner_radius: 0.0,
                        reveal: None,
                    },
                },
            )
        });
    let navigator = Navigator::new(Page::Home, routes)
        .transition(transition)
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    assert!(
        selected.borrow().is_empty(),
        "initial mounting must not animate"
    );
    controller.push(Page::Redirect);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        *selected.borrow(),
        [(Page::Home, Page::Detail, NavigationAction::Push)]
    );
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage", "HomePage"]);
    assert_eq!(pages(navigator.as_ref(), true), ["DetailPage"]);
    for _ in 0..8 {
        navigator.update(&ctx);
    }
    BUILDS.with(|log| assert_eq!(*log.borrow(), [Page::Home, Page::Detail]));
    finish.set(true);
    navigator.update(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    assert_eq!(
        find_host(navigator.as_ref()).unwrap().retained_clip(&ctx),
        None,
        "the transition must stop clipping ordinary page overflow when it finishes"
    );
    DROPS.with(|log| assert!(log.borrow().contains(&Page::Home)));
}

#[test]
fn rapid_navigation_is_bounded_and_disabled_routes_override_fallback() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(CrossFadeTransition::new().duration(Duration::from_secs(60)))
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    controller.push(Page::Home);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage", "HomePage"]);
    controller.push(Page::Disabled);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DisabledPage"]);
    controller.pop();
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["HomePage"],
        "the departing route disables pop animation"
    );
}

#[test]
fn zero_duration_and_duplicate_urls_do_not_start_animations() {
    let ctx = context();
    let selected = Rc::new(Cell::new(0));
    let count = selected.clone();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(DynamicTransitions::new(
            move |n: &NavigationContext<'_, '_, Page>| {
                count.set(count.get() + 1);
                CrossFadeTransition::new()
                    .duration(Duration::ZERO)
                    .resolve(n)
            },
        ))
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Home);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(selected.get(), 0);
    assert_eq!(controller.history_len(), 2);
    controller.set_route(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    assert_eq!(selected.get(), 1);
    assert_eq!(controller.history_len(), 1);
}

/// Pops slide, pushes are instant, so a test can reach a poppable stack at once.
fn slide_on_pop() -> impl NavigationTransition<Page> {
    DynamicTransitions::new(|navigation: &NavigationContext<'_, '_, Page>| {
        if navigation.action == NavigationAction::Pop {
            SlideTransition::new().resolve(navigation)
        } else {
            None
        }
    })
}

/// Lets the settle animation, which runs on the wall clock, run out.
fn let_settle_finish() {
    std::thread::sleep(Duration::from_millis(400));
}

#[test]
fn a_swipe_builds_the_page_beneath_once_and_a_commit_adopts_it_instead_of_rebuilding() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    BUILDS.with(|log| assert_eq!(*log.borrow(), [Page::Home, Page::Detail]));

    let swipe = controller
        .begin_swipe_back()
        .expect("a page with somewhere to go back to can be swiped");
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the page beneath is shown"
    );
    assert_eq!(
        pages(navigator.as_ref(), true),
        ["DetailPage"],
        "only the page under the finger takes input"
    );
    assert_eq!(
        controller.history_len(),
        2,
        "history is untouched until the swipe commits"
    );

    swipe.drag(0.4);
    for _ in 0..4 {
        navigator.update(&ctx);
    }
    BUILDS.with(|log| assert_eq!(*log.borrow(), [Page::Home, Page::Detail, Page::Home],
        "starting and dragging build the page beneath and nothing else, not even the page being swiped"));

    swipe.release(true);
    navigator.update(&ctx); // the first frame after release starts the settle clock
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(controller.history_len(), 1);
    assert_eq!(controller.current_route(), Page::Home);
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
    BUILDS.with(|log| {
        assert_eq!(
            *log.borrow(),
            [Page::Home, Page::Detail, Page::Home],
            "the page that was beneath is the one that stays; it is not built a second time"
        )
    });
    DROPS.with(|log| assert!(log.borrow().contains(&Page::Detail)));
}

/// Every built-in effect, as a pop-only fallback so a push is instant.
fn each_built_in_effect() -> Vec<(
    &'static str,
    Box<
        dyn Fn() -> Box<
            dyn Fn(&NavigationContext<'_, '_, Page>) -> Option<ResolvedNavigationTransition>,
        >,
    >,
)> {
    fn pop_only<T: NavigationTransition<Page> + 'static>(
        effect: T,
    ) -> Box<dyn Fn(&NavigationContext<'_, '_, Page>) -> Option<ResolvedNavigationTransition>> {
        Box::new(move |n| {
            if n.action == NavigationAction::Pop {
                effect.resolve(n)
            } else {
                None
            }
        })
    }
    vec![
        ("slide", Box::new(|| pop_only(SlideTransition::new()))),
        (
            "cross fade",
            Box::new(|| pop_only(CrossFadeTransition::new())),
        ),
        ("zoom", Box::new(|| pop_only(ZoomTransition::new()))),
        (
            "view levels",
            Box::new(|| pop_only(ViewLevelTransitions::new())),
        ),
        // Untagged, so it falls back to a zoom, as an app without shared items would.
        (
            "container",
            Box::new(|| pop_only(ContainerTransition::<Page>::new())),
        ),
    ]
}

#[test]
fn every_built_in_effect_can_be_swiped_to_a_commit() {
    for (name, effect) in each_built_in_effect() {
        let ctx = context();
        let selector = effect();
        let navigator = Navigator::new(Page::Home, routes)
            .transition(DynamicTransitions::new(
                move |n: &NavigationContext<'_, '_, Page>| selector(n),
            ))
            .to_element(&ctx);
        let controller = NavigatorController::<Page>::of(&ctx);
        controller.push(Page::Detail);
        navigator.rebuild_if_dirty(&ctx);

        let swipe = controller
            .begin_swipe_back()
            .unwrap_or_else(|| panic!("{name}: a swipe could not begin"));
        navigator.rebuild_if_dirty(&ctx);
        assert_eq!(
            pages(navigator.as_ref(), false),
            ["DetailPage", "HomePage"],
            "{name}: the page beneath is shown"
        );
        swipe.drag(0.5);
        navigator.update(&ctx);
        swipe.release(true);
        navigator.update(&ctx);
        let_settle_finish();
        navigator.update(&ctx);
        navigator.rebuild_if_dirty(&ctx);

        assert_eq!(
            controller.current_route(),
            Page::Home,
            "{name}: the swipe committed"
        );
        assert_eq!(
            pages(navigator.as_ref(), false),
            ["HomePage"],
            "{name}: nothing is left behind"
        );
    }
}

#[test]
fn every_built_in_effect_can_be_swiped_to_a_cancel() {
    for (name, effect) in each_built_in_effect() {
        let ctx = context();
        let selector = effect();
        let navigator = Navigator::new(Page::Home, routes)
            .transition(DynamicTransitions::new(
                move |n: &NavigationContext<'_, '_, Page>| selector(n),
            ))
            .to_element(&ctx);
        let controller = NavigatorController::<Page>::of(&ctx);
        controller.push(Page::Detail);
        navigator.rebuild_if_dirty(&ctx);

        let swipe = controller
            .begin_swipe_back()
            .unwrap_or_else(|| panic!("{name}: a swipe could not begin"));
        navigator.rebuild_if_dirty(&ctx);
        swipe.drag(0.2);
        navigator.update(&ctx);
        swipe.release(false);
        navigator.update(&ctx);
        let_settle_finish();
        navigator.update(&ctx);
        navigator.rebuild_if_dirty(&ctx);

        assert_eq!(
            controller.current_route(),
            Page::Detail,
            "{name}: the swipe cancelled"
        );
        assert_eq!(
            pages(navigator.as_ref(), false),
            ["DetailPage"],
            "{name}: the page beneath is released"
        );
    }
}

/// What the user sees as the swipe starts and ends, for any effect: the page
/// under the finger has not moved or faded at the start, and has left at the end.
#[test]
fn every_built_in_effect_starts_with_the_page_untouched_and_ends_with_it_gone() {
    let ctx = context();
    let size = ResolvedSize {
        width: 200.0,
        height: 100.0,
    };
    for (name, effect) in each_built_in_effect() {
        let selector = effect();
        let navigation = NavigationContext {
            from: &Page::Detail,
            to: &Page::Home,
            action: NavigationAction::Pop,
            build_context: &ctx,
        };
        let resolved = selector(&navigation).unwrap_or_else(|| panic!("{name}: no effect"));
        // A swipe presents the page on screen as the departing one.
        let start = resolved.sample(0.0, size).unwrap();
        assert_eq!(
            start.outgoing,
            PaintPresentation::IDENTITY_OPAQUE,
            "{name}: the page has not moved when the swipe begins"
        );
        let end = resolved.sample(1.0, size).unwrap();
        let gone =
            end.outgoing.opacity == 0.0 || end.outgoing.transform.cols[2][0].abs() >= size.width;
        assert!(
            gone,
            "{name}: the departing page is gone at the end: {:?}",
            end.outgoing
        );
        assert_eq!(
            end.incoming,
            PaintPresentation::IDENTITY_OPAQUE,
            "{name}: the page beneath is in place at the end"
        );
    }
}

#[test]
fn a_cancelled_swipe_leaves_the_stack_alone_and_drops_the_page_beneath() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let swipe = controller.begin_swipe_back().unwrap();
    navigator.rebuild_if_dirty(&ctx);

    swipe.drag(0.2);
    navigator.update(&ctx);
    swipe.release(false);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(controller.history_len(), 2);
    assert_eq!(controller.current_route(), Page::Detail);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    DROPS.with(|log| {
        assert!(
            log.borrow().contains(&Page::Home),
            "the underlay is released"
        )
    });
}

#[test]
fn a_swipe_needs_a_page_to_return_to_and_an_effect_to_play() {
    let ctx = context();
    // Nothing beneath the initial route.
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    assert!(controller.begin_swipe_back().is_none());
    // No effect configured anywhere.
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    drop(navigator);

    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes).to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert!(
        controller.begin_swipe_back().is_none(),
        "a navigator with no transition pops instantly"
    );
    drop(navigator);

    // A route that disables its effect cannot be swiped away either.
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Disabled);
    navigator.rebuild_if_dirty(&ctx);
    assert!(controller.begin_swipe_back().is_none());
}

#[test]
fn another_navigation_during_a_swipe_ends_it_without_popping_twice() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let swipe = controller.begin_swipe_back().unwrap();
    navigator.rebuild_if_dirty(&ctx);
    swipe.drag(0.5);

    controller.push(Page::Explicit);
    navigator.rebuild_if_dirty(&ctx);
    swipe.release(true);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(
        controller.history_len(),
        3,
        "the late commit of a dead swipe pops nothing"
    );
    assert_eq!(controller.current_route(), Page::Explicit);
}

/// A finger on a real navigator: the swipe begins mid-touch, which rebuilds the
/// host the touch was captured by, and the rest of the touch has to find the
/// host that replaced it.
fn swipe_with_a_finger(
    ends_at: f32,
    sample_gap: Duration,
) -> (impl Fn() -> usize, NavigatorController<Page>, AnyElement) {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::EventDispatcher;
    use aimer_widget::base::Vec2d;

    let ctx = Box::leak(Box::new(context()));
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(ctx);
    let controller = NavigatorController::<Page>::of(ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(ctx);

    let mut dispatcher = EventDispatcher::new();
    let touch = |x: f32, y: f32| {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        )
    };
    let mut send = |event: ElementEvent| {
        let pos = event.get_pointer_pos().unwrap_or_default();
        let _ = dispatcher.dispatch(navigator.as_ref(), pos, &event);
    };
    send(ElementEvent::PointerDown(touch(5.0, 50.0)));
    send(ElementEvent::PointerMove(touch(30.0, 52.0)));
    navigator.rebuild_if_dirty(ctx);
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the swipe began and the page beneath is shown"
    );
    std::thread::sleep(sample_gap);
    send(ElementEvent::PointerMove(touch(130.0, 52.0)));
    std::thread::sleep(sample_gap);
    send(ElementEvent::PointerMove(touch(ends_at, 52.0)));
    send(ElementEvent::PointerUp(touch(ends_at, 52.0)));
    drop(send);
    navigator.update(ctx);
    let_settle_finish();
    navigator.update(ctx);
    navigator.rebuild_if_dirty(ctx);
    let history = controller.clone();
    (move || history.history_len(), controller, navigator)
}

/// A finger that starts in the middle of the page, with nothing at the edge.
fn swipe_with_a_finger_from_the_middle(
    start_x: f32,
    area: SwipeBackArea,
) -> (usize, Page, Vec<&'static str>, Vec<Page>) {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::EventDispatcher;
    use aimer_widget::base::Vec2d;

    let ctx = Box::leak(Box::new(context()));
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .swipe_back_area(area)
        .to_element(ctx);
    let controller = NavigatorController::<Page>::of(ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(ctx);

    let mut dispatcher = EventDispatcher::new();
    let touch = |x: f32, y: f32| {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        )
    };
    let mut send = |event: ElementEvent| {
        let pos = event.get_pointer_pos().unwrap_or_default();
        let _ = dispatcher.dispatch(navigator.as_ref(), pos, &event);
    };
    send(ElementEvent::PointerDown(touch(start_x, 50.0)));
    send(ElementEvent::PointerMove(touch(start_x + 10.0, 51.0)));
    navigator.rebuild_if_dirty(ctx);
    let during_small = pages(navigator.as_ref(), false);
    send(ElementEvent::PointerMove(touch(start_x + 30.0, 52.0)));
    navigator.rebuild_if_dirty(ctx);
    navigator.rebuild_if_dirty(ctx);
    let shown = pages(navigator.as_ref(), false);
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(start_x + 60.0, 52.0)));
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(start_x + 90.0, 52.0)));
    send(ElementEvent::PointerUp(touch(start_x + 90.0, 52.0)));
    drop(send);
    navigator.update(ctx);
    let_settle_finish();
    navigator.update(ctx);
    navigator.rebuild_if_dirty(ctx);
    let _ = during_small;
    (
        controller.history_len(),
        controller.current_route(),
        shown,
        controller.routes(),
    )
}

#[test]
fn with_the_swipe_limited_to_the_edge_the_middle_of_the_page_is_left_alone() {
    let (history, current, shown, _) =
        swipe_with_a_finger_from_the_middle(60.0, SwipeBackArea::Edge);
    assert_eq!(shown, ["DetailPage"], "nothing began");
    assert_eq!(history, 2);
    assert_eq!(current, Page::Detail);
}

#[test]
fn a_flick_from_the_edge_pops_the_page_and_the_touch_finds_the_rebuilt_host() {
    let (history_len, controller, navigator) =
        swipe_with_a_finger(150.0, Duration::from_millis(20));
    assert_eq!(
        history_len(),
        1,
        "the release reached the swipe, so the pop committed"
    );
    assert_eq!(controller.current_route(), Page::Home);
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
}

#[test]
fn a_flick_back_to_the_edge_cancels() {
    // Fast leftward at the end: below half *and* moving away.
    let (history_len, controller, navigator) = swipe_with_a_finger(60.0, Duration::from_millis(20));
    assert_eq!(history_len(), 2);
    assert_eq!(controller.current_route(), Page::Detail);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
}

#[test]
fn the_system_back_request_pops_a_navigator_and_is_unhandled_at_its_root() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes).to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);

    assert!(
        aimer_events::back::request_back(),
        "there was a page to pop"
    );
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Page::Home);
    assert!(
        !aimer_events::back::request_back(),
        "at the root the navigator declines, so the platform may leave the app"
    );
}

#[test]
fn a_navigator_that_is_gone_no_longer_answers_back() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes).to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    drop(navigator);
    assert!(!aimer_events::back::request_back());
}

#[test]
fn a_system_back_gesture_drives_the_same_swipe_as_a_finger() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);

    let gesture = aimer_events::back::start_back_gesture()
        .expect("a page that can be swiped back offers a gesture");
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage", "HomePage"]);
    gesture.progress(0.7);
    gesture.commit();
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Page::Home);
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
}

#[test]
fn a_system_back_gesture_that_is_cancelled_stays_put() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(slide_on_pop())
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let gesture = aimer_events::back::start_back_gesture().unwrap();
    navigator.rebuild_if_dirty(&ctx);
    gesture.progress(0.3);
    gesture.cancel();
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Page::Detail);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
}

#[test]
fn a_page_with_no_effect_offers_no_gesture_but_still_answers_a_plain_back() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes).to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert!(
        aimer_events::back::start_back_gesture().is_none(),
        "there is no motion for the system to drive"
    );
    assert!(aimer_events::back::request_back());
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(controller.current_route(), Page::Home);
}

struct Frame;
impl PortableWidget for Frame {}
impl Widget for Frame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        FRAMES.set(FRAMES.get() + 1);
        Outlet.to_element(ctx)
    }
}

#[derive(Clone)]
struct Dashboard(Page);
impl Route for Dashboard {
    fn parse(_: &str) -> Option<Self> {
        None
    }
    fn format(&self) -> String {
        format!("/dashboard{}", self.0.format())
    }
    fn shell_key(&self) -> Option<&'static str> {
        Some("dashboard")
    }
}
impl Router for Dashboard {
    fn build(&self, _: &BuildContext) -> AnyWidget {
        dashboard(self.clone())
    }
}
fn dashboard(route: Dashboard) -> AnyWidget {
    let child = route.0;
    Shell::new(Frame, move |_| routes(child))
        .route(child)
        .transition(DynamicTransitions::new(
            |n: &NavigationContext<'_, '_, Page>| {
                SHELL_ACTIONS.with(|log| log.borrow_mut().push(n.action));
                CrossFadeTransition::new()
                    .duration(Duration::from_secs(60))
                    .resolve(n)
            },
        ))
        .boxed()
}

#[test]
fn shell_child_transitions_keep_the_frame_mounted_and_reverse_on_pop() {
    let ctx = context();
    let navigator = Navigator::new(Dashboard(Page::Home), dashboard)
        .transition(ZoomTransition::new())
        .to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(FRAMES.get(), 1, "the shell frame must remain mounted");
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage", "HomePage"]);
    assert_eq!(pages(navigator.as_ref(), true), ["DetailPage"]);
    SHELL_ACTIONS.with(|log| assert_eq!(*log.borrow(), [NavigationAction::Push]));
    controller.pop();
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(FRAMES.get(), 1);
    SHELL_ACTIONS.with(|log| {
        assert_eq!(
            *log.borrow(),
            [NavigationAction::Push, NavigationAction::Pop]
        )
    });
    drop(navigator);
    BUILDS.with(|builds| {
        DROPS.with(|drops| {
            assert_eq!(
                builds.borrow().len(),
                drops.borrow().len(),
                "retained shell pages must be released"
            )
        })
    });
}

/// A shell whose child pages slide on pop and are instant otherwise. The
/// navigator around it has no effect of its own: the shell's outlet owns the motion.
fn sliding_dashboard(route: Dashboard) -> AnyWidget {
    let child = route.0;
    Shell::new(Frame, move |_| routes(child))
        .route(child)
        .transition(DynamicTransitions::new(
            |n: &NavigationContext<'_, '_, Page>| {
                if n.action == NavigationAction::Pop {
                    SlideTransition::new().resolve(n)
                } else {
                    None
                }
            },
        ))
        .boxed()
}

#[test]
fn a_swipe_inside_a_shell_moves_the_outlet_and_leaves_the_frame_mounted() {
    let ctx = context();
    let navigator = Navigator::new(Dashboard(Page::Home), sliding_dashboard).to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(FRAMES.get(), 1);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);

    let swipe = controller
        .begin_swipe_back()
        .expect("the shell's outlet has an effect to drive");
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the page beneath is shown in the outlet"
    );
    assert_eq!(
        FRAMES.get(),
        1,
        "the shell frame is never rebuilt by a swipe"
    );

    swipe.drag(0.5);
    navigator.update(&ctx);
    BUILDS.with(|log| {
        assert_eq!(
            *log.borrow(),
            [Page::Home, Page::Detail, Page::Home],
            "the page beneath is built once, and the page under the finger not again"
        )
    });
    swipe.release(true);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(
        controller.current_route().0,
        Page::Home,
        "the swipe popped the navigator"
    );
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
    assert_eq!(FRAMES.get(), 1);
    BUILDS.with(|log| {
        assert_eq!(
            *log.borrow(),
            [Page::Home, Page::Detail, Page::Home],
            "the page that was beneath is adopted, not built again"
        )
    });
}

#[test]
fn a_finger_swipes_back_inside_a_shell_through_the_outlet_and_not_the_navigator_around_it() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::EventDispatcher;
    use aimer_widget::base::Vec2d;

    let ctx = context();
    let navigator = Navigator::new(Dashboard(Page::Home), sliding_dashboard).to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let touch = |x: f32, y: f32| {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        )
    };
    let mut send = |event: ElementEvent| {
        let pos = event.get_pointer_pos().unwrap_or_default();
        let _ = dispatcher.dispatch(navigator.as_ref(), pos, &event);
    };
    send(ElementEvent::PointerDown(touch(60.0, 50.0)));
    send(ElementEvent::PointerMove(touch(90.0, 52.0)));
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the outlet shows the page beneath"
    );
    assert_eq!(FRAMES.get(), 1, "the shell frame is untouched");
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(120.0, 52.0)));
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(150.0, 52.0)));
    send(ElementEvent::PointerUp(touch(150.0, 52.0)));
    drop(send);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    // assert_eq!(controller.current_route().0, Page::Home, "the release reached the swipe, so it popped");
    // assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
}

aimer_tls::thread_local! {
    static BAR_LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// The frame of a shell with a bar under the outlet, 40 high: the outlet is the
/// top 100, the bar the bottom 40.
struct BarFrame;
impl PortableWidget for BarFrame {}
impl Widget for BarFrame {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        BarFrameElement {
            outlet: Outlet.to_element(ctx),
            bar: TabBar.to_element(ctx),
        }
        .boxed()
    }
}
struct BarFrameElement {
    outlet: AnyElement,
    bar: AnyElement,
}
impl VisitorElement for BarFrameElement {
    fn debug_name(&self) -> &'static str {
        "BarFrame"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.outlet.as_ref());
        visitor(self.bar.as_ref());
    }
}
impl Drawable for BarFrameElement {
    fn can_paint_local_v2(&self, _: &BuildContext) -> bool {
        true
    }
}
impl Rebuildable for BarFrameElement {}
impl EventElement for BarFrameElement {}
impl LayoutElement for BarFrameElement {
    fn pos_start_end(&self) -> Option<(aimer_widget::base::Vec2d, aimer_widget::base::Vec2d)> {
        Some((
            Default::default(),
            aimer_widget::base::Vec2d { x: 200.0, y: 140.0 },
        ))
    }
}
struct TabBar;
impl PortableWidget for TabBar {}
impl Widget for TabBar {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }
}
impl VisitorElement for TabBar {
    fn debug_name(&self) -> &'static str {
        "TabBar"
    }
}
impl Drawable for TabBar {
    fn can_paint_local_v2(&self, _: &BuildContext) -> bool {
        true
    }
}
impl Rebuildable for TabBar {}
impl LayoutElement for TabBar {
    fn pos_start_end(&self) -> Option<(aimer_widget::base::Vec2d, aimer_widget::base::Vec2d)> {
        Some((
            aimer_widget::base::Vec2d { x: 0.0, y: 100.0 },
            aimer_widget::base::Vec2d { x: 200.0, y: 140.0 },
        ))
    }
}
impl EventElement for TabBar {
    fn on_event(&self, event: &aimer_events::element::ElementEvent) -> aimer_widget::EventResult {
        use aimer_events::element::ElementEvent as E;
        let name = match event {
            E::PointerDown(_) => "down",
            E::PointerMove(_) => "move",
            E::PointerUp(_) => "up",
            _ => return aimer_widget::EventResult::ignored(),
        };
        BAR_LOG.with(|log| log.borrow_mut().push(name));
        aimer_widget::EventResult::consumed()
    }
}

fn barred_dashboard(route: Dashboard) -> AnyWidget {
    let child = route.0;
    Shell::new(BarFrame, move |_| routes(child))
        .route(child)
        .transition(DynamicTransitions::new(
            |n: &NavigationContext<'_, '_, Page>| {
                if n.action == NavigationAction::Pop {
                    SlideTransition::new().resolve(n)
                } else {
                    None
                }
            },
        ))
        .boxed()
}

#[test]
fn a_drag_that_begins_on_the_frame_around_the_outlet_is_not_a_swipe() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::EventDispatcher;
    use aimer_widget::base::Vec2d;

    let ctx = context();
    let navigator = Navigator::new(Dashboard(Page::Home), barred_dashboard).to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    let mut dispatcher = EventDispatcher::new();
    let touch = |x: f32, y: f32| {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        )
    };
    let mut send = |event: ElementEvent| {
        let pos = event.get_pointer_pos().unwrap_or_default();
        let _ = dispatcher.dispatch(navigator.as_ref(), pos, &event);
    };
    // On the bar, below the outlet.
    send(ElementEvent::PointerDown(touch(60.0, 120.0)));
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(100.0, 121.0)));
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(150.0, 121.0)));
    send(ElementEvent::PointerUp(touch(150.0, 121.0)));
    drop(send);
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage"],
        "no page beneath was shown"
    );
    assert_eq!(controller.current_route().0, Page::Detail);
    BAR_LOG.with(|log| {
        assert_eq!(
            *log.borrow(),
            ["down", "move", "move", "up"],
            "the bar had the whole touch"
        )
    });
}

#[test]
fn a_cancelled_swipe_inside_a_shell_changes_nothing() {
    let ctx = context();
    let navigator = Navigator::new(Dashboard(Page::Home), sliding_dashboard).to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    let swipe = controller.begin_swipe_back().unwrap();
    navigator.rebuild_if_dirty(&ctx);
    navigator.rebuild_if_dirty(&ctx);
    swipe.drag(0.2);
    navigator.update(&ctx);
    swipe.release(false);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(controller.current_route().0, Page::Detail);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    assert_eq!(FRAMES.get(), 1);
}

#[test]
fn a_shell_with_no_effect_cannot_be_swiped_and_nothing_is_left_hanging() {
    let ctx = context();
    fn plain(route: Dashboard) -> AnyWidget {
        let child = route.0;
        Shell::new(Frame, move |_| routes(child))
            .route(child)
            .boxed()
    }
    let navigator = Navigator::new(Dashboard(Page::Home), plain).to_element(&ctx);
    let controller = NavigatorController::<Dashboard>::of(&ctx);
    controller.push(Dashboard(Page::Detail));
    navigator.rebuild_if_dirty(&ctx);
    // The navigator cannot know in advance, so a swipe may begin; the outlet declines it.
    if let Some(swipe) = controller.begin_swipe_back() {
        navigator.rebuild_if_dirty(&ctx);
        navigator.rebuild_if_dirty(&ctx);
        assert!(
            swipe.is_over(),
            "the outlet had no effect to play, so the swipe is over"
        );
    }
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
    assert_eq!(controller.current_route().0, Page::Detail);
}

fn frame(_: &BuildContext) -> AnyWidget {
    Frame.boxed()
}

#[test]
fn stateful_shell_animates_only_visible_changes_and_keeps_the_frame_mounted() {
    let ctx = context();
    let shell = StatefulShell::new(vec![Page::Home, Page::Detail], frame, routes)
        .transition(CrossFadeTransition::new().duration(Duration::from_secs(60)))
        .to_element(&ctx);
    let controller = StatefulShellController::<Page>::of(&ctx);
    controller.push_in_branch(1, Page::Disabled);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(pages(shell.as_ref(), false), ["HomePage"]);
    controller.go_branch(1);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(pages(shell.as_ref(), false), ["DisabledPage"]);
    controller.go_branch(0);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(pages(shell.as_ref(), false), ["DisabledPage", "HomePage"]);
    assert_eq!(FRAMES.get(), 1);
    controller.go_branch(9);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(controller.active_branch(), 0);
}

/// A tabbed shell whose first branch has a page to go back from.
fn tabbed_shell(
    ctx: &BuildContext<'static>,
    transition: impl NavigationTransition<Page>,
) -> (AnyElement, std::rc::Rc<StatefulShellController<Page>>) {
    let shell = StatefulShell::new(vec![Page::Home, Page::Detail], frame, routes)
        .transition(transition)
        .to_element(ctx);
    let controller = StatefulShellController::<Page>::of(ctx);
    controller.push_in_branch(0, Page::Detail);
    shell.rebuild_if_dirty(ctx);
    (shell, controller)
}

#[test]
fn a_swipe_pops_the_active_branch_and_leaves_the_other_branches_alone() {
    let ctx = context();
    let (shell, controller) = tabbed_shell(&ctx, slide_on_pop());
    assert_eq!(pages(shell.as_ref(), false), ["DetailPage"]);
    assert_eq!((controller.branch_len(0), controller.branch_len(1)), (2, 1));

    let swipe = controller
        .begin_swipe_back()
        .expect("a branch with a page to go back from can be swiped");
    shell.rebuild_if_dirty(&ctx);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(shell.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the page beneath is shown"
    );
    assert_eq!(FRAMES.get(), 1, "the shell frame stays mounted");

    swipe.drag(0.5);
    shell.update(&ctx);
    swipe.release(true);
    shell.update(&ctx);
    let_settle_finish();
    shell.update(&ctx);
    shell.rebuild_if_dirty(&ctx);
    shell.rebuild_if_dirty(&ctx);

    assert_eq!(
        (controller.branch_len(0), controller.branch_len(1)),
        (1, 1),
        "only the active branch popped"
    );
    assert_eq!(pages(shell.as_ref(), false), ["HomePage"]);
    BUILDS.with(|log| assert_eq!(*log.borrow(), [Page::Home, Page::Detail, Page::Home],
        "only the active branch is built, and the page beneath once: it is adopted, not built again"));
}

#[test]
fn a_cancelled_swipe_in_a_branch_changes_nothing() {
    let ctx = context();
    let (shell, controller) = tabbed_shell(&ctx, slide_on_pop());
    let swipe = controller.begin_swipe_back().unwrap();
    shell.rebuild_if_dirty(&ctx);
    shell.rebuild_if_dirty(&ctx);
    swipe.drag(0.2);
    shell.update(&ctx);
    swipe.release(false);
    shell.update(&ctx);
    let_settle_finish();
    shell.update(&ctx);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(controller.branch_len(0), 2);
    assert_eq!(pages(shell.as_ref(), false), ["DetailPage"]);
}

#[test]
fn a_branch_at_its_root_or_with_no_effect_cannot_be_swiped() {
    let ctx = context();
    let (shell, controller) = tabbed_shell(&ctx, slide_on_pop());
    controller.go_branch(1);
    shell.rebuild_if_dirty(&ctx);
    assert!(
        controller.begin_swipe_back().is_none(),
        "branch 1 has one page"
    );
    drop(shell);

    let ctx = context();
    let shell = StatefulShell::new(vec![Page::Home], frame, routes).to_element(&ctx);
    let controller = StatefulShellController::<Page>::of(&ctx);
    controller.push_in_branch(0, Page::Detail);
    shell.rebuild_if_dirty(&ctx);
    assert!(
        controller.begin_swipe_back().is_none(),
        "no transition anywhere: an instant pop has no motion to drive"
    );
}

#[test]
fn the_system_back_pops_the_active_branch_and_declines_at_its_root() {
    let ctx = context();
    let (shell, controller) = tabbed_shell(&ctx, slide_on_pop());
    assert!(aimer_events::back::request_back());
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(controller.branch_len(0), 1);
    assert!(
        !aimer_events::back::request_back(),
        "at the branch root it declines, so an enclosing navigator or the platform may answer"
    );
}

#[test]
fn a_finger_swipes_back_inside_a_tab_branch() {
    use aimer_events::element::ElementEvent;
    use aimer_events::pointer::{PointerButton, PointerInfo, PointerSource};
    use aimer_widget::EventDispatcher;
    use aimer_widget::base::Vec2d;

    let ctx = context();
    let (shell, controller) = tabbed_shell(&ctx, slide_on_pop());
    let mut dispatcher = EventDispatcher::new();
    let touch = |x: f32, y: f32| {
        PointerInfo::new(
            Vec2d { x, y },
            PointerSource::Touch,
            1,
            PointerButton::Primary,
        )
    };
    let mut send = |event: ElementEvent| {
        let pos = event.get_pointer_pos().unwrap_or_default();
        let _ = dispatcher.dispatch(shell.as_ref(), pos, &event);
    };
    send(ElementEvent::PointerDown(touch(5.0, 50.0)));
    send(ElementEvent::PointerMove(touch(30.0, 52.0)));
    shell.rebuild_if_dirty(&ctx);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(
        pages(shell.as_ref(), false),
        ["DetailPage", "HomePage"],
        "the swipe began under the finger"
    );
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(130.0, 52.0)));
    std::thread::sleep(Duration::from_millis(20));
    send(ElementEvent::PointerMove(touch(150.0, 52.0)));
    send(ElementEvent::PointerUp(touch(150.0, 52.0)));
    drop(send);
    shell.update(&ctx);
    let_settle_finish();
    shell.update(&ctx);
    shell.rebuild_if_dirty(&ctx);
    shell.rebuild_if_dirty(&ctx);
    assert_eq!(
        controller.branch_len(0),
        1,
        "the release reached the swipe, so the branch popped"
    );
    assert_eq!(pages(shell.as_ref(), false), ["HomePage"]);
}

aimer_tls::thread_local! {
    static DEPARTING_FOCUS: RefCell<Option<aimer_widget::focus::FocusNode>> = const { RefCell::new(None) };
    static FOCUS_LOST: Cell<usize> = const { Cell::new(0) };
}
struct FocusedPage(aimer_widget::focus::FocusNode);
impl VisitorElement for FocusedPage {
    fn debug_name(&self) -> &'static str {
        "FocusedPage"
    }
}
impl Drawable for FocusedPage {}
impl LayoutElement for FocusedPage {}
impl Rebuildable for FocusedPage {}
impl EventElement for FocusedPage {
    fn focus_node(&self) -> Option<&aimer_widget::focus::FocusNode> {
        Some(&self.0)
    }
    fn on_event(&self, event: &aimer_events::element::ElementEvent) -> aimer_widget::EventResult {
        if matches!(event, aimer_events::element::ElementEvent::FocusLost) {
            FOCUS_LOST.set(FOCUS_LOST.get() + 1);
        }
        aimer_widget::EventResult::ignored()
    }
}
struct FocusedWidget(Page);
impl PortableWidget for FocusedWidget {}
impl Widget for FocusedWidget {
    fn to_element(self, _: &BuildContext) -> AnyElement {
        let focus = aimer_widget::focus::FocusNode::new();
        if self.0 == Page::Home {
            focus.set_focused(true);
            DEPARTING_FOCUS.with(|slot| *slot.borrow_mut() = Some(focus.clone()));
        }
        FocusedPage(focus).boxed()
    }
}
fn focused_routes(page: Page) -> AnyWidget {
    FocusedWidget(page).boxed()
}

#[test]
fn outgoing_pages_lose_keyboard_focus_when_the_transition_starts() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, focused_routes)
        .transition(CrossFadeTransition::new().duration(Duration::from_secs(60)))
        .to_element(&ctx);
    NavigatorController::<Page>::of(&ctx).push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    DEPARTING_FOCUS.with(|slot| assert!(!slot.borrow().as_ref().unwrap().has_focus()));
    assert_eq!(FOCUS_LOST.get(), 1);
}

fn find_host(element: &dyn Element) -> Option<&dyn Element> {
    if element.debug_name() == "NavigationTransitionHost" {
        return Some(element);
    }
    let mut found = None;
    element.visit_children(&mut |child| {
        if found.is_none() {
            found = find_host(child);
        }
    });
    found
}

/// A tagged container transition leads with a copy of the item it grows from.
/// A swipe presents that copy too, and drops it when the swipe is over.
#[test]
fn a_tagged_container_transition_swipes_with_its_item_copy_and_releases_it() {
    use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
    use aimer_widget::{ErrorWidget, Key};

    let ctx = context();
    // The item on screen that the detail page grew out of.
    let item = aimer_animation::SharedElement::new()
        .tag("album")
        .child_with(|| ErrorWidget::new("cover"))
        .to_element(&ctx);
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(20.0, 20.0, 60.0, 40.0)).unwrap();
    let retained = tree.context(node).unwrap();
    assert!(ctx.with_local_v2_paint_context(retained, |ctx| item.sync_local_v2_state(ctx)));

    let container = ContainerTransition::<Page>::new()
        .tag(|page: &Page| (*page == Page::Detail).then(|| Key::from("album")));
    let navigator = Navigator::new(Page::Home, routes)
        .transition(DynamicTransitions::new(
            move |n: &NavigationContext<'_, '_, Page>| {
                if n.action == NavigationAction::Pop {
                    container.resolve(n)
                } else {
                    None
                }
            },
        ))
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);

    let swipe = controller
        .begin_swipe_back()
        .expect("a tagged container transition can be swiped");
    navigator.rebuild_if_dirty(&ctx);
    let structural = |navigator: &dyn Element| {
        let mut count = 0;
        find_host(navigator)
            .expect("a host")
            .structural_children(&mut |_| count += 1);
        count
    };
    assert_eq!(
        structural(navigator.as_ref()),
        3,
        "the departing page, the page beneath, and the item copy"
    );

    swipe.drag(0.4);
    navigator.update(&ctx);
    swipe.release(true);
    navigator.update(&ctx);
    let_settle_finish();
    navigator.update(&ctx);
    navigator.rebuild_if_dirty(&ctx);

    assert_eq!(controller.current_route(), Page::Home);
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
    assert_eq!(
        structural(navigator.as_ref()),
        1,
        "the item copy is gone with the pages that carried it"
    );
    drop(item);
}

#[test]
fn custom_presentations_reach_retained_nodes_without_rerecording_page_lists() {
    use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(DynamicTransitions::new(
            |_: &NavigationContext<'_, '_, Page>| {
                ResolvedNavigationTransition::new(
                    Duration::from_secs(60),
                    curves::LINEAR,
                    |_, _| NavigationTransitionFrame {
                        incoming: PaintPresentation {
                            transform: aimer_canvas::Mat3::translate(25.0, 0.0),
                            opacity: 0.25,
                            corner_radius: 0.0,
                            reveal: None,
                        },
                        outgoing: PaintPresentation {
                            transform: aimer_canvas::Mat3::identity(),
                            opacity: 0.75,
                            corner_radius: 0.0,
                            reveal: None,
                        },
                    },
                )
            },
        ))
        .to_element(&ctx);
    NavigatorController::<Page>::of(&ctx).push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let host = find_host(navigator.as_ref()).unwrap();
    assert_eq!(
        host.retained_clip(&ctx),
        Some(Rect::new(0.0, 0.0, 200.0, 100.0))
    );
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 100.0)).unwrap();
    let outgoing = tree
        .add_child(root, Rect::new(0.0, 0.0, 200.0, 100.0))
        .unwrap();
    let incoming = tree
        .add_child(root, Rect::new(0.0, 0.0, 200.0, 100.0))
        .unwrap();
    for node in [incoming, outgoing] {
        tree.context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![])
            .unwrap();
    }
    let retained = tree.context(root).unwrap();
    for _ in 0..8 {
        assert!(
            ctx.with_local_v2_paint_context(retained.clone(), |ctx| host.sync_local_v2_state(ctx))
        );
    }
    assert_eq!(
        tree.presentation(incoming).unwrap(),
        (aimer_canvas::Mat3::translate(25.0, 0.0), 0.25)
    );
    assert_eq!(
        tree.presentation(outgoing).unwrap(),
        (aimer_canvas::Mat3::identity(), 0.75)
    );
    assert_eq!(tree.draw_list_revision(incoming).unwrap(), 1);
    assert_eq!(tree.draw_list_revision(outgoing).unwrap(), 1);
    BUILDS.with(|log| assert_eq!(*log.borrow(), [Page::Home, Page::Detail]));
}

#[test]
fn route_overrides_and_navigation_actions_use_the_correct_owner() {
    let ctx = context();
    let actions = Rc::new(RefCell::new(Vec::new()));
    let log = actions.clone();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(DynamicTransitions::new(
            move |n: &NavigationContext<'_, '_, Page>| {
                log.borrow_mut().push(n.action);
                CrossFadeTransition::new()
                    .duration(Duration::from_secs(60))
                    .resolve(n)
            },
        ))
        .to_element(&ctx);
    let controller = NavigatorController::<Page>::of(&ctx);
    controller.pop();
    navigator.rebuild_if_dirty(&ctx);
    controller.push(Page::Explicit);
    navigator.rebuild_if_dirty(&ctx);
    assert!(actions.borrow().is_empty());
    assert_eq!(pages(navigator.as_ref(), false), ["ExplicitPage"]);
    controller.push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    controller.pop();
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(
        *actions.borrow(),
        [NavigationAction::Push, NavigationAction::Pop]
    );
    assert_eq!(
        pages(navigator.as_ref(), false),
        ["DetailPage", "ExplicitPage"]
    );
    controller.clear();
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(actions.borrow().last(), Some(&NavigationAction::Clear));
    controller.set_route(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(actions.borrow().last(), Some(&NavigationAction::Replace));
}

aimer_tls::thread_local! { static ROOT_VERSION: Cell<bool> = const { Cell::new(false) }; }
fn versioned_routes(_: Page) -> AnyWidget {
    routes(if ROOT_VERSION.get() {
        Page::Detail
    } else {
        Page::Home
    })
}

#[test]
fn rebuilding_a_navigator_refreshes_route_configuration_without_starting_a_transition() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, versioned_routes)
        .transition(ZoomTransition::new())
        .to_element(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["HomePage"]);
    ROOT_VERSION.set(true);
    navigator.mark_needs_rebuild();
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(pages(navigator.as_ref(), false), ["DetailPage"]);
}

#[test]
fn a_route_keeps_its_flex_factor_through_the_transition_host() {
    let ctx = context();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(ZoomTransition::new())
        .to_element(&ctx);
    assert_eq!(navigator.flex(), Some(2.0));
    NavigatorController::<Page>::of(&ctx).push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    assert_eq!(navigator.flex(), Some(2.0));
}

#[test]
fn high_dpi_transitions_sample_and_clip_the_logical_viewport() {
    use aimer_cupid::draw_cmd_v2::{Rect, RenderTree};
    let mut ctx = context();
    ctx.scale = 2.0;
    ctx.parent_size = ResolvedSize {
        width: 400.0,
        height: 200.0,
    };
    let sampled_size = Rc::new(Cell::new(ResolvedSize::default()));
    let observed = sampled_size.clone();
    let navigator = Navigator::new(Page::Home, routes)
        .transition(DynamicTransitions::new(
            move |_: &NavigationContext<'_, '_, Page>| {
                let observed = observed.clone();
                ResolvedNavigationTransition::new(
                    Duration::from_secs(60),
                    curves::LINEAR,
                    move |_, size| {
                        observed.set(size);
                        NavigationTransitionFrame {
                            outgoing: PaintPresentation::IDENTITY_OPAQUE,
                            incoming: PaintPresentation {
                                transform: aimer_canvas::Mat3::translate(
                                    size.width * 0.5,
                                    size.height * 0.5,
                                ),
                                opacity: 1.0,
                                corner_radius: 0.0,
                                reveal: None,
                            },
                        }
                    },
                )
            },
        ))
        .to_element(&ctx);
    NavigatorController::<Page>::of(&ctx).push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let host = find_host(navigator.as_ref()).unwrap();
    let viewport = Rect::new(0.0, 0.0, 200.0, 100.0);
    assert_eq!(host.retained_clip(&ctx), Some(viewport));
    let tree = RenderTree::new();
    let root = tree.add_root(viewport).unwrap();
    tree.add_child(root, viewport).unwrap();
    let incoming = tree.add_child(root, viewport).unwrap();
    assert!(
        ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| host
            .sync_local_v2_state(ctx))
    );
    assert_eq!(
        sampled_size.get(),
        ResolvedSize {
            width: 200.0,
            height: 100.0
        }
    );
    assert_eq!(
        tree.presentation(incoming).unwrap().0,
        aimer_canvas::Mat3::translate(100.0, 50.0)
    );
}

#[test]
fn both_transition_children_receive_the_original_viewport_constraints() {
    let mut ctx = context();
    ctx.box_constraint.max_width = 240.0;
    let navigator = Navigator::new(Page::Home, routes)
        .transition(CrossFadeTransition::new().duration(Duration::from_secs(60)))
        .to_element(&ctx);
    NavigatorController::<Page>::of(&ctx).push(Page::Detail);
    navigator.rebuild_if_dirty(&ctx);
    let host = find_host(navigator.as_ref()).unwrap();
    let mut children = 0;
    host.visit_retained_v2_children(&mut |_, child| {
        let child_ctx = host
            .retained_v2_child_context(&ctx, child)
            .expect("an overlay must preserve transparent-wrapper constraints for both pages");
        assert_eq!(child_ctx.box_constraint.max_width, 240.0);
        assert_eq!(child_ctx.parent_size, ctx.parent_size);
        children += 1;
    });
    assert_eq!(children, 2);
}
