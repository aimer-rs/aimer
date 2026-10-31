//! An implicitly animated value moves through its in-between values.
//!
//! The animation advances when the rebuild pass reaches its element, and a pass
//! that has indexed the tree skips every element that is not dirty. An animation
//! nobody marked dirty has to keep itself reachable, or it stays where its first
//! frame left it and the value jumps to the end.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{curves, ImplicitAnimatedBuilder};
use aimer_widget::StateUpdater;

struct RootState {
    target: f32,
    values: Rc<RefCell<Vec<f32>>>,
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

struct Root {
    values: Rc<RefCell<Vec<f32>>>,
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

impl StatefulWidget for Root {
    type State = RootState;

    fn create_state(self) -> RootState {
        RootState {
            target: 0.0,
            values: self.values,
            updater: self.updater,
        }
    }
}

impl State<Root> for RootState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let values = self.values.clone();
        let rows: Vec<aimer_widget::AnyWidget> = (0..30u8)
            .map(|i| {
                SizedBox::new()
                    .width(Dimension::Px(100.0))
                    .height(Dimension::Px(10.0))
                    .color(Color::Rgb(i * 8, 80, 120))
                    .boxed()
            })
            .collect();
        Column::new().children(vec![
            ImplicitAnimatedBuilder::new(
                self.target,
                Duration::from_millis(300),
                curves::LINEAR,
                move |value: &f32| {
                    values.borrow_mut().push(*value);
                    SizedBox::new()
                        .width(Dimension::Px(50.0 + 100.0 * *value))
                        .height(Dimension::Px(20.0))
                        .color(Color::Rgb(200, 60, 60))
                },
            )
            .boxed(),
            Column::new().children(rows).boxed(),
        ])
    }
}

impl Widget for Root {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ImplicitRoot", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Root {}

#[test]
fn an_implicit_animation_moves_through_its_in_between_values() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let values = Rc::new(RefCell::new(Vec::new()));
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Root {
            values: values.clone(),
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(300, 400),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    values.borrow_mut().clear();

    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.target = 1.0);
    let end = aimer_time::Instant::now() + Duration::from_millis(500);
    while aimer_time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }

    let seen = values.borrow();
    let between = seen
        .iter()
        .filter(|value| **value > 0.05 && **value < 0.95)
        .count();
    assert!(between >= 5, "the animation jumped: it built with {seen:?}");
    assert_eq!(
        seen.last().copied(),
        Some(1.0),
        "the animation must end on its target"
    );
}
