//! The copy of a shared element that a transition draws follows a theme that
//! changes after the element was first built.
//!
//! A card that reads the theme and is built with `child_with` hands the registry
//! a closure that has the theme it was built with in it. The card rebuilds when
//! the theme changes, and the copy a transition leads with has to be the new
//! one: a copy in the old theme would open in the old colours over a screen in
//! the new ones.

use super::*;

use std::cell::{Cell, RefCell};
use std::time::Duration;

use aimer_animation::{SharedElement, curves, last_shared_element};
use aimer_style::{AnimatedTheme, Theme, ThemeMode};
use aimer_widget::{Key, StateUpdater, StatelessElement};

#[derive(Clone, Copy, Debug, PartialEq, aimer_macro::Theme)]
struct TestTheme {
    tone: u8,
}

/// The tone the latest copy of the card was built with.
type Built = Rc<Cell<Option<u8>>>;

struct Card {
    built: Built,
}

impl Widget for Card {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            move |ctx| {
                let tone = TestTheme::of(ctx).tone;
                let built = self.built.clone();
                SharedElement::new()
                    .tag("themed-card")
                    .child_with(move || {
                        built.set(Some(tone));
                        SizedBox::new().width(100).height(50).color(Color::Gray8(tone))
                    })
                    .to_element(ctx)
            },
            None,
            "ThemedCard",
        )
        .boxed()
    }
}

/// Where the card stands in the tree.
#[derive(Clone, Copy)]
enum Placement {
    /// Directly under the theme.
    Plain,
    /// A row of a list built lazily by a builder, inside a scroll view, as a feed's cards are.
    ListRow,
}

struct ShellState {
    placement: Placement,
    mode: ThemeMode,
    built: Built,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

struct Shell {
    placement: Placement,
    built: Built,
    updater: Rc<RefCell<Option<StateUpdater<ShellState>>>>,
}

impl StatefulWidget for Shell {
    type State = ShellState;

    fn create_state(self) -> ShellState {
        ShellState { placement: self.placement, mode: ThemeMode::Dark, built: self.built, updater: self.updater }
    }
}

impl State<Shell> for ShellState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        AnimatedTheme::new()
            .adaptive(TestTheme { tone: 30 }, TestTheme { tone: 235 })
            .mode(self.mode)
            .duration(Duration::from_millis(150))
            .curve(curves::EASE_IN_OUT)
            .child(match self.placement {
                Placement::Plain => Card { built: self.built.clone() }.boxed(),
                Placement::ListRow => {
                    let built = self.built.clone();
                    Scrollable::new()
                        .child(
                            Column::new()
                                .list(vec![0usize, 1, 2])
                                .builder(move |_: &usize| {
                                    aimer_container::Container::new().child(Card { built: built.clone() })
                                }),
                        )
                        .boxed()
                }
            })
    }
}

impl Widget for Shell {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "SharedThemeShell", None).0.boxed()
    }
}

fn run_for<W: Widget + 'static>(app: &mut HeadlessAimerApp<W>, millis: u64) {
    let end = aimer_time::Instant::now() + Duration::from_millis(millis);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
}

/// The tone a transition's copy of the card would be built with right now.
fn copy_tone(built: &Built) -> Option<u8> {
    built.set(None);
    let snapshot = last_shared_element(&Key::from("themed-card"))?;
    drop(snapshot.content?.build());
    built.get()
}

fn follows_the_theme(placement: Placement) {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let built: Built = Rc::new(Cell::new(None));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Shell { placement, built: built.clone(), updater: updater.clone() },
        HeadlessOptions { size: PhysicalSize::new(300, 300), scale_factor: 2.0 },
    );
    app.pump_frames(3);
    assert_eq!(copy_tone(&built), Some(235), "the dark theme's copy to begin with");

    for (mode, tone) in [(ThemeMode::Light, 30), (ThemeMode::Dark, 235), (ThemeMode::Light, 30)] {
        updater.borrow().expect("initialised").set_state(move |state| state.mode = mode);
        run_for(&mut app, 400);
        assert_eq!(copy_tone(&built), Some(tone), "after switching to {mode:?}");
    }
}

#[test]
fn the_copy_of_a_shared_card_follows_the_theme_when_it_changes() {
    follows_the_theme(Placement::Plain);
}

#[test]
fn the_copy_of_a_card_in_a_scrolling_list_follows_the_theme_when_it_changes() {
    follows_the_theme(Placement::ListRow);
}
