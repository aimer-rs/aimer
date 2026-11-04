//! Animated wrappers keep their children on the retained render path.
//!
//! An `AnimatedSwitcher` presents its outgoing and incoming child through the
//! render tree, which needs nothing from the children's paint extent; it used
//! to fall back to one legacy island for the whole subtree whenever a child
//! could not promise to paint inside its layout box, which is every child by
//! default. An implicit animation must publish each frame's rebuilt child
//! before the tree syncs, or its whole subtree is painted by the legacy path.

use std::cell::RefCell;
use std::time::Duration;

use aimer::animation::layout::AnimatedLayout;
use aimer::animation::{
    AnimatedPaint, AnimatedSwitcher, AnimationController, curves, FadeTransition,
    ImplicitAnimatedBuilder, MorphTransition, RotationTransition, ScaleTransition, SlideTransition,
};
use aimer::canvas::Canvas;
use aimer_cupid::utilities::{Color as V2Color, Rect};
use aimer::{
    AimerApp, AnyElement, BuildContext, Color, Drawable, Element, EventElement, LayoutElement,
    Rebuildable, ResolvedSize, Size, State, StateUpdater, StatefulElement, StatefulWidget,
    VisitorElement, Widget,
};

/// Renders `frames` direct frames, checking after each one that nothing in the
/// tree fell back to the legacy paint path. A fallback can last a single frame
/// of a transition, so checking only the last would miss it.
fn render_staying_retained<W: Widget + 'static>(
    app: &mut aimer::quiver::aimer_app::HeadlessAimerApp<W>,
    frames: usize,
    what: &str,
) {
    for frame in 0..frames {
        app.render_frame();
        let census = app
            .paint_source_census()
            .expect("a mounted page has a root");
        assert!(
            census.drawn_unmapped.is_empty(),
            "{what}: frame {frame} drew an element the tree does not know: {census:?}"
        );
        assert!(
            census.unresolved_roots.is_empty(),
            "{what}: a node the tree has not painted yet: {:?}",
            census.unresolved_roots
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A leaf that paints one rectangle through the retained path and, like most
/// elements, does not claim that its paint stays inside its layout box.
struct Swatch {
    color: Color,
}

impl Widget for Swatch {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        Element::boxed(self)
    }
}

impl VisitorElement for Swatch {
    fn debug_name(&self) -> &'static str {
        "Swatch"
    }
}

impl EventElement for Swatch {}
impl Rebuildable for Swatch {}

impl LayoutElement for Swatch {
    fn size(&self) -> Option<Size> {
        Some(Size::new(60.0, 40.0))
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        ResolvedSize {
            width: 60.0 * ctx.scale,
            height: 40.0 * ctx.scale,
        }
    }
}

impl Drawable for Swatch {
    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        let (red, green, blue, alpha) = self.color.to_rgba();
        canvas.fill_rect_styled(
            Rect::new(0.0, 0.0, 60.0, 40.0),
            V2Color::rgba8(red, green, blue, alpha),
            [0.0; 4],
            [0.0; 4],
            V2Color::transparent(),
            [0.0; 4],
            V2Color::transparent(),
        );
        canvas.finish();
    }
}

thread_local! {
    /// The page's state, so the test can switch the child the way a button does.
    static PAGE: RefCell<Option<StateUpdater<PageState>>> = const { RefCell::new(None) };
}

struct Page;

struct PageState {
    showing_second: bool,
}

impl StatefulWidget for Page {
    type State = PageState;

    fn create_state(self) -> Self::State {
        PageState {
            showing_second: false,
        }
    }
}

impl State<Page> for PageState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        PAGE.replace(Some(updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let (key, color) = if self.showing_second {
            ("second", Color::Rgb(0, 0, 255))
        } else {
            ("first", Color::Rgb(255, 0, 0))
        };
        AnimatedSwitcher::new(Duration::from_millis(400), curves::LINEAR, Swatch { color })
            .child_key(key)
    }
}

impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "Page", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Page"
    }
}

#[test]
fn a_crossfade_between_unbounded_children_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(Page);
    app.pump_frames(5);

    PAGE.with_borrow(|updater| {
        updater
            .as_ref()
            .expect("the page state publishes its updater")
            .set_state(|state| state.showing_second = true)
    });
    // Mid-transition: both children are alive and the switcher is animating.
    render_staying_retained(&mut app, 8, "the crossfade");
}

thread_local! {
    /// The implicit page's state, so the test can move its target.
    static IMPLICIT: RefCell<Option<StateUpdater<ImplicitPageState>>> = const { RefCell::new(None) };
}

struct ImplicitPage;

struct ImplicitPageState {
    target: f32,
}

impl StatefulWidget for ImplicitPage {
    type State = ImplicitPageState;

    fn create_state(self) -> Self::State {
        ImplicitPageState { target: 0.0 }
    }
}

impl State<ImplicitPage> for ImplicitPageState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        IMPLICIT.replace(Some(updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        ImplicitAnimatedBuilder::new(
            self.target,
            Duration::from_millis(400),
            curves::LINEAR,
            |value: &f32| Swatch {
                color: Color::Rgb((value.clamp(0.0, 1.0) * 255.0) as u8, 0, 0),
            },
        )
    }
}

impl Widget for ImplicitPage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ImplicitPage", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ImplicitPage"
    }
}

#[test]
fn an_implicit_animation_paints_its_rebuilt_child_on_the_retained_path() {
    let mut app = AimerApp::start_headless(ImplicitPage);
    app.pump_frames(5);

    IMPLICIT.with_borrow(|updater| {
        updater
            .as_ref()
            .expect("the page state publishes its updater")
            .set_state(|state| state.target = 1.0)
    });
    // Mid-animation the builder produces a fresh child every frame.
    render_staying_retained(&mut app, 8, "the implicit animation");
}

#[test]
fn an_animated_layout_around_an_unbounded_child_stays_on_the_retained_path() {
    struct LayoutPage;

    impl Widget for LayoutPage {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            AnimatedLayout::new()
                .duration(Duration::from_millis(400))
                .child(Swatch {
                    color: Color::Rgb(0, 128, 0),
                })
                .to_element(ctx)
        }
    }

    let mut app = AimerApp::start_headless(LayoutPage);
    render_staying_retained(&mut app, 8, "the layout transition");
}

thread_local! {
    /// The morph page's state, so the test can switch the child.
    static MORPH: RefCell<Option<StateUpdater<MorphPageState>>> = const { RefCell::new(None) };
}

struct MorphPage;

struct MorphPageState {
    showing_second: bool,
}

impl StatefulWidget for MorphPage {
    type State = MorphPageState;

    fn create_state(self) -> Self::State {
        MorphPageState {
            showing_second: false,
        }
    }
}

impl State<MorphPage> for MorphPageState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        MORPH.replace(Some(updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let (key, color) = if self.showing_second {
            ("second", Color::Rgb(0, 0, 255))
        } else {
            ("first", Color::Rgb(255, 0, 0))
        };
        MorphTransition::new(Duration::from_millis(400), curves::LINEAR, Swatch { color })
            .child_key(key)
    }
}

impl Widget for MorphPage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "MorphPage", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "MorphPage"
    }
}

#[test]
fn a_morph_between_unbounded_children_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(MorphPage);
    app.pump_frames(5);

    MORPH.with_borrow(|updater| {
        updater
            .as_ref()
            .expect("the page state publishes its updater")
            .set_state(|state| state.showing_second = true)
    });
    render_staying_retained(&mut app, 8, "the morph");
}

#[test]
fn an_animated_opacity_over_an_unbounded_child_stays_on_the_retained_path() {
    struct FadePage;

    impl Widget for FadePage {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let controller = AnimationController::with_millis(400, curves::LINEAR);
            controller.forward();
            AnimatedPaint::new(
                controller,
                Swatch {
                    color: Color::Rgb(0, 128, 0),
                },
            )
            .opacity()
            .to_element(ctx)
        }
    }

    let mut app = AimerApp::start_headless(FadePage);
    render_staying_retained(&mut app, 8, "the animated opacity");
}

/// A page built by a closure, for fixtures that need no state of their own.
struct Built(Box<dyn FnOnce(&BuildContext) -> AnyElement>);

impl Widget for Built {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        (self.0)(ctx)
    }
}

/// A controller that is mid-run for the whole test.
fn running_controller() -> AnimationController {
    let controller = AnimationController::with_millis(2_000, curves::LINEAR);
    controller.forward_from_first_tick();
    controller
}

fn swatch() -> Swatch {
    Swatch {
        color: Color::Rgb(0, 128, 0),
    }
}

/// A running transition presents its child as a render-node transform and
/// opacity. It used to ask the child for a paint-extent promise and, without
/// one, drew the whole subtree through a legacy island every frame.
#[test]
fn a_running_fade_over_an_unbounded_child_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(Built(Box::new(|ctx| {
        FadeTransition::new(running_controller(), swatch()).to_element(ctx)
    })));
    render_staying_retained(&mut app, 8, "the fade");
}

#[test]
fn a_running_slide_over_an_unbounded_child_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(Built(Box::new(|ctx| {
        SlideTransition::new(running_controller(), (10.0, 4.0), swatch()).to_element(ctx)
    })));
    render_staying_retained(&mut app, 8, "the slide");
}

#[test]
fn a_running_scale_over_an_unbounded_child_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(Built(Box::new(|ctx| {
        ScaleTransition::new(running_controller(), swatch()).to_element(ctx)
    })));
    render_staying_retained(&mut app, 8, "the scale");
}

#[test]
fn a_running_rotation_over_an_unbounded_child_stays_on_the_retained_path() {
    let mut app = AimerApp::start_headless(Built(Box::new(|ctx| {
        RotationTransition::new(running_controller(), swatch())
            .turn_range(0.0, -0.25)
            .to_element(ctx)
    })));
    render_staying_retained(&mut app, 8, "the rotation");
}
