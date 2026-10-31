//! A field that focuses itself on arrival is told so by the frame that built
//! it, not by whatever input happens next.
//!
//! Focus used to be decided only while an event was dispatched. A search page
//! whose bar autofocuses therefore sat unfocused as far as its `on_focus`
//! callback went — and so did the focus animation that callback starts — until
//! the user moved the pointer or touched the screen.

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

fn counter() -> Arc<AtomicU32> {
    Arc::new(AtomicU32::new(0))
}

fn counting(count: &Arc<AtomicU32>) -> impl Fn(String) + 'static {
    let count = Arc::clone(count);
    move |_| {
        count.fetch_add(1, Ordering::Relaxed);
    }
}

fn headless(widget: impl Widget + 'static) -> HeadlessAimerApp<impl Widget + 'static> {
    AimerApp::start_headless_with(
        Column::new().children([widget.boxed()]),
        HeadlessOptions {
            size: PhysicalSize::new(400, 200),
            scale_factor: 1.0,
        },
    )
}

#[test]
fn an_autofocus_field_hears_it_gained_focus_without_any_input() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let gained = counter();
    let mut app = headless(
        TextField::new()
            .auto_focus(true)
            .on_focus(counting(&gained)),
    );

    app.pump_frames(3);

    assert_eq!(
        gained.load(Ordering::Relaxed),
        1,
        "no event was sent, yet the field that focuses itself must be told"
    );
}

#[test]
fn a_field_that_does_not_autofocus_is_left_alone() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let gained = counter();
    let mut app = headless(TextField::new().on_focus(counting(&gained)));

    app.pump_frames(3);

    assert_eq!(gained.load(Ordering::Relaxed), 0);
}

#[test]
fn settling_reports_the_gain_once_however_many_frames_follow() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let gained = counter();
    let mut app = headless(
        TextField::new()
            .auto_focus(true)
            .on_focus(counting(&gained)),
    );

    app.pump_frames(10);

    assert_eq!(gained.load(Ordering::Relaxed), 1);
}

/// What the search bar of a forum does: the field focuses itself, and its
/// `on_focus` flips the target of an implicit animation that draws the focus
/// ring. Nothing but frames passes, and the ring has to animate in anyway.
mod focus_ring {
    use super::*;

    use std::cell::RefCell;
    use std::time::Duration;

    use aimer_animation::{ImplicitAnimatedBuilder, curves};
    use aimer_widget::StateUpdater;

    struct FocusProbe {
        values: Rc<RefCell<Vec<f32>>>,
    }

    struct FocusProbeState {
        focused: bool,
        values: Rc<RefCell<Vec<f32>>>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for FocusProbe {
        type State = FocusProbeState;

        fn create_state(self) -> FocusProbeState {
            FocusProbeState {
                focused: false,
                values: self.values,
                updater: StateUpdater::new(),
            }
        }
    }

    impl State<FocusProbe> for FocusProbeState {
        fn init_state(&mut self, updater: StateUpdater<Self>) {
            self.updater = updater;
        }

        fn build(&self, _ctx: &BuildContext) -> impl Widget {
            let values = self.values.clone();
            let updater = self.updater.clone();
            ImplicitAnimatedBuilder::new(
                if self.focused { 1.0 } else { 0.0 },
                Duration::from_millis(280),
                curves::LINEAR,
                move |focus: &f32| {
                    values.borrow_mut().push(*focus);
                    let updater = updater.clone();
                    TextField::new()
                        .auto_focus(true)
                        .on_focus(move |_: String| updater.set_state(|state| state.focused = true))
                },
            )
        }
    }

    impl Widget for FocusProbe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            StatefulElement::new_with_name(self, ctx, "FocusProbe", None)
                .0
                .boxed()
        }
    }

    impl aimer_widget::PortableWidget for FocusProbe {}

    #[test]
    fn the_focus_animation_of_an_autofocus_field_plays_without_any_input() {
        let _serial = VIRTUALIZED_RENDER_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let values = Rc::new(RefCell::new(Vec::new()));
        let mut app = AimerApp::start_headless_with(
            FocusProbe {
                values: values.clone(),
            },
            HeadlessOptions {
                size: PhysicalSize::new(400, 200),
                scale_factor: 1.0,
            },
        );

        let end = aimer_time::Instant::now() + Duration::from_millis(900);
        while aimer_time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(8));
            app.pump_frames(1);
        }

        let seen = values.borrow();
        let between = seen
            .iter()
            .filter(|value| **value > 0.05 && **value < 0.95)
            .count();
        assert!(
            between >= 5,
            "the ring never animated in: the field built with {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some(1.0), "and it settles focused");
    }
}
