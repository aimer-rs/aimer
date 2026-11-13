//! A descendant changes the theme through the controller that `AnimatedTheme`
//! owns, with no stateful widget above the theme and no handle passed down.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::curves;
use aimer_style::{AnimatedTheme, Theme, ThemeController, ThemeMode};
use aimer_widget::StatelessElement;

#[derive(Clone, Copy, Debug, PartialEq, aimer_macro::Theme)]
struct TestTheme {
    tone: u8,
}

impl TestTheme {
    fn light() -> Self {
        Self { tone: 30 }
    }

    fn dark() -> Self {
        Self { tone: 235 }
    }
}

/// What the probe saw on its last build, and the controller it found.
#[derive(Default)]
struct Seen {
    tone: Option<u8>,
    via_of: Option<ThemeController>,
    via_context: Option<ThemeController>,
}

type Shared = Rc<RefCell<Seen>>;

struct Probe {
    seen: Shared,
}

impl Widget for Probe {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let theme = TestTheme::of(ctx);
                {
                    let mut seen = self.seen.borrow_mut();
                    seen.tone = Some(theme.tone);
                    // `Theme::of` hands the controller back with the theme, and
                    // `ThemeController::of` finds the same one on its own.
                    seen.via_of = Some(theme.controller().clone());
                    seen.via_context = Some(ThemeController::of(ctx));
                }
                SizedBox::new()
                    .width(Dimension::Px(100.0))
                    .height(Dimension::Px(20.0))
                    .color(Color::Gray8(theme.tone))
                    .to_element(ctx)
            },
            None,
            "ControllerProbe",
        )
        .boxed()
    }
}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = aimer_time::Instant::now() + Duration::from_millis(millis);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

fn start(mode: ThemeMode, duration_ms: u64) -> (HeadlessAimerApp<impl Widget + 'static>, Shared) {
    let seen: Shared = Rc::default();
    let root = AnimatedTheme::new()
        .adaptive(TestTheme::light(), TestTheme::dark())
        .mode(mode)
        .duration(Duration::from_millis(duration_ms))
        .curve(curves::LINEAR)
        .child(Probe { seen: seen.clone() });
    let mut app = AimerApp::start_headless_with(
        root,
        HeadlessOptions {
            size: PhysicalSize::new(300, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    (app, seen)
}

fn controller(seen: &Shared) -> ThemeController {
    seen.borrow()
        .via_of
        .clone()
        .expect("the probe built under the theme")
}

#[test]
fn a_descendant_can_switch_the_mode_through_the_controller() {
    let (mut app, seen) = start(ThemeMode::Light, 60);
    assert_eq!(seen.borrow().tone, Some(TestTheme::light().tone));

    controller(&seen).set_mode(ThemeMode::Dark);
    run_for(&mut app, 300);

    assert_eq!(seen.borrow().tone, Some(TestTheme::dark().tone));
    assert_eq!(controller(&seen).mode(), ThemeMode::Dark);
}

#[test]
fn the_switch_animates_instead_of_snapping() {
    let (mut app, seen) = start(ThemeMode::Light, 400);

    controller(&seen).set_mode(ThemeMode::Dark);
    app.pump_frames(1);
    run_for(&mut app, 120);

    let tone = seen.borrow().tone.expect("built");
    assert!(
        tone > TestTheme::light().tone && tone < TestTheme::dark().tone,
        "expected an in-between tone mid-transition, saw {tone}"
    );
}

#[test]
fn toggle_crosses_to_the_opposite_appearance_and_back() {
    let (mut app, seen) = start(ThemeMode::Light, 40);
    let controller = controller(&seen);

    controller.toggle();
    run_for(&mut app, 250);
    assert_eq!(seen.borrow().tone, Some(TestTheme::dark().tone));
    assert_eq!(controller.mode(), ThemeMode::Dark);

    controller.toggle();
    run_for(&mut app, 250);
    assert_eq!(seen.borrow().tone, Some(TestTheme::light().tone));
    assert_eq!(controller.mode(), ThemeMode::Light);
}

#[test]
fn setting_the_current_mode_changes_nothing() {
    let (mut app, seen) = start(ThemeMode::Dark, 40);
    let before = seen.borrow().tone;

    controller(&seen).set_mode(ThemeMode::Dark);
    run_for(&mut app, 120);

    assert_eq!(seen.borrow().tone, before);
}

#[test]
fn the_controller_reports_the_appearance_it_resolved() {
    let (_app, seen) = start(ThemeMode::Dark, 40);

    assert_eq!(
        controller(&seen).brightness(),
        aimer_widget::Brightness::Dark
    );
}

#[test]
fn theme_of_and_the_context_lookup_share_one_controller() {
    let (mut app, seen) = start(ThemeMode::Light, 40);

    let via_context = seen
        .borrow()
        .via_context
        .clone()
        .expect("the probe built under the theme");
    via_context.set_mode(ThemeMode::Dark);
    run_for(&mut app, 200);

    // Both handles observe the one mode the theme owns.
    assert_eq!(controller(&seen).mode(), ThemeMode::Dark);
}

struct ParentState {
    mode: ThemeMode,
    seen: Shared,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ParentState>>>>,
}

struct Parent {
    seen: Shared,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<ParentState>>>>,
}

impl State<Parent> for ParentState {
    fn create(widget: Parent) -> Self {
        ParentState {
            mode: ThemeMode::Light,
            seen: widget.seen,
            updater: widget.updater,
        }
    }

    fn on_mount(&mut self, updater: aimer_widget::StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        AnimatedTheme::new()
            .adaptive(TestTheme::light(), TestTheme::dark())
            .mode(this.mode)
            .duration(Duration::from_millis(40))
            .child(Probe { seen: this.seen.clone() })
    }
}

impl Widget for Parent {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, ParentState>(self, ctx, "ThemeControllerParent", None)
            .0
            .boxed()
    }
}

#[test]
fn a_parent_rebuild_keeps_the_controllers_choice_until_it_configures_another_mode() {
    let seen: Shared = Rc::default();
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Parent {
            seen: seen.clone(),
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    // A descendant chooses dark; a parent rebuild that repeats `Light` is not a
    // new decision and must not undo it.
    controller(&seen).set_mode(ThemeMode::Dark);
    run_for(&mut app, 200);
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Light);
    run_for(&mut app, 200);
    assert_eq!(seen.borrow().tone, Some(TestTheme::dark().tone));

    // Configuring a different mode is a new decision from above.
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Dark);
    run_for(&mut app, 200);
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.mode = ThemeMode::Light);
    run_for(&mut app, 200);
    assert_eq!(seen.borrow().tone, Some(TestTheme::light().tone));
}

// Replacing one `AnimatedTheme` with another that supplies a different theme
// type must replace the subtree below it, as the Jaime showcase does when the
// reader moves between two theming examples.

struct SwapState {
    custom: bool,
    controller: Rc<RefCell<Option<ThemeController>>>,
    built: Rc<RefCell<Vec<&'static str>>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<SwapState>>>>,
}

struct Swap {
    controller: Rc<RefCell<Option<ThemeController>>>,
    built: Rc<RefCell<Vec<&'static str>>>,
    updater: Rc<RefCell<Option<aimer_widget::StateUpdater<SwapState>>>>,
}

struct Named {
    name: &'static str,
    built: Rc<RefCell<Vec<&'static str>>>,
    custom: bool,
    controller: Rc<RefCell<Option<ThemeController>>>,
}

impl Widget for Named {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                if self.custom {
                    let _ = TestTheme::of(ctx);
                } else {
                    let theme = aimer_style::ThemeData::of(ctx);
                    *self.controller.borrow_mut() = Some(theme.controller().clone());
                }
                self.built.borrow_mut().push(self.name);
                SizedBox::new()
                    .width(Dimension::Px(10.0))
                    .height(Dimension::Px(10.0))
                    .to_element(ctx)
            },
            None,
            "Named",
        )
        .boxed()
    }
}

impl State<Swap> for SwapState {
    fn create(widget: Swap) -> Self {
        SwapState {
            custom: true,
            controller: widget.controller,
            built: widget.built,
            updater: widget.updater,
        }
    }

    fn on_mount(&mut self, updater: aimer_widget::StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(this: &StateView<Self>, _ctx: &BuildContext) -> impl Widget {
        let built = this.built.clone();
        if this.custom {
            AnimatedTheme::new()
                .adaptive(TestTheme::light(), TestTheme::dark())
                .box_child(Named { name: "custom", built, custom: true, controller: this.controller.clone() })
        } else {
            AnimatedTheme::new().box_child(Named { name: "builtin", built, custom: false, controller: this.controller.clone() })
        }
    }
}

impl Widget for Swap {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name::<Self, SwapState>(self, ctx, "ThemeSwap", None)
            .0
            .boxed()
    }
}

#[test]
fn replacing_a_theme_with_one_of_another_type_replaces_its_subtree() {
    let built = Rc::new(RefCell::new(Vec::new()));
    let updater = Rc::new(RefCell::new(None));
    let controller = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Swap { built: built.clone(), updater: updater.clone(), controller: controller.clone() },
        HeadlessOptions {
            size: PhysicalSize::new(300, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    assert_eq!(built.borrow().last(), Some(&"custom"));

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.custom = false);
    run_for(&mut app, 200);

    assert_eq!(
        built.borrow().last(),
        Some(&"builtin"),
        "the old theme's subtree stayed on screen: {:?}",
        built.borrow()
    );
}

#[test]
fn a_controller_found_after_replacing_the_theme_still_switches_it() {
    let built = Rc::new(RefCell::new(Vec::new()));
    let updater = Rc::new(RefCell::new(None));
    let controller = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Swap { built: built.clone(), updater: updater.clone(), controller: controller.clone() },
        HeadlessOptions {
            size: PhysicalSize::new(300, 200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.custom = false);
    run_for(&mut app, 200);
    let controller = controller
        .borrow()
        .clone()
        .expect("the built-in theme's subtree built");

    // Must not panic: the controller belongs to the theme that is on screen.
    controller.set_mode(ThemeMode::Dark);
    run_for(&mut app, 400);

    assert_eq!(controller.mode(), ThemeMode::Dark);
}
