//! What a frame costs while a translucent bar over a long page fades.
//!
//! The bar is a glass panel inside an implicit animation that a hover starts, as
//! on a post page; the page under it is a scrollable column of deep rows. A
//! frame that only fades the bar must not do work proportional to the page.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_container::Glass;
use aimer_widget::StateUpdater;

const ROW_DEPTH: usize = 5;

struct SceneState {
    rows: usize,
    glass: bool,
    hovered: bool,
    updater: Rc<RefCell<Option<StateUpdater<SceneState>>>>,
}

struct Scene {
    rows: usize,
    glass: bool,
    updater: Rc<RefCell<Option<StateUpdater<SceneState>>>>,
}

impl StatefulWidget for Scene {
    type State = SceneState;

    fn create_state(self) -> SceneState {
        SceneState {
            rows: self.rows,
            glass: self.glass,
            hovered: false,
            updater: self.updater,
        }
    }
}

fn deep_row() -> aimer_widget::AnyWidget {
    let mut row = SizedBox::new()
        .width(Dimension::Px(120.0))
        .height(Dimension::Px(8.0))
        .color(Color::WHITE)
        .boxed();
    for _ in 0..ROW_DEPTH {
        row = aimer_container::Container::new().child(row).boxed();
    }
    row
}

impl State<Scene> for SceneState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn adopt_config_from(&mut self, _new: Self) {}

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let opacity = if self.hovered { 1.0f32 } else { 0.4 };
        let glass = self.glass;
        let rows: Vec<aimer_widget::AnyWidget> = (0..self.rows).map(|_| deep_row()).collect();
        aimer_space::Stack::new().children(vec![
            aimer_space::Positioned::new().layer(3).top(0).box_child(
                aimer_animation::ImplicitAnimatedBuilder::new(
                    opacity,
                    Duration::from_millis(220),
                    aimer_animation::curves::EASE_IN_OUT,
                    move |_opacity| {
                        if glass {
                            Glass::new()
                                .child(
                                    SizedBox::new()
                                        .width(Dimension::Px(200.0))
                                        .height(Dimension::Px(30.0)),
                                )
                                .boxed()
                        } else {
                            aimer_container::Container::new()
                                .color(Color::Rgb(60, 40, 40))
                                .child(
                                    SizedBox::new()
                                        .width(Dimension::Px(200.0))
                                        .height(Dimension::Px(30.0)),
                                )
                                .boxed()
                        }
                    },
                ),
            ),
            Scrollable::new()
                .child(Column::new().children(rows))
                .boxed(),
        ])
    }
}

impl Widget for Scene {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "GlassBarScene", None)
            .0
            .boxed()
    }
}

impl aimer_widget::PortableWidget for Scene {}

/// The element boundaries a frame entered, and the full render-tree syncs it
/// needed, for each frame of the fade a state change starts.
fn fade_frames(rows: usize, glass: bool) -> (Vec<u64>, u64) {
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Scene {
            rows,
            glass,
            updater: updater.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, 300),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    updater
        .borrow()
        .expect("initialised")
        .set_state(|state| state.hovered = true);
    let full_before = app.app.render_tree.sync_counts.full;
    let mut visits = Vec::new();
    for _ in 0..14 {
        std::thread::sleep(Duration::from_millis(16));
        let before = aimer_widget::rebuild_descent_count();
        app.pump_frames(1);
        visits.push(aimer_widget::rebuild_descent_count() - before);
    }
    (visits, app.app.render_tree.sync_counts.full - full_before)
}

#[test]
fn a_fading_bar_does_not_walk_the_page_under_it() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    for glass in [false, true] {
        let (few, _) = fade_frames(20, glass);
        let (many, _) = fade_frames(200, glass);
        // The first frames of an animation legitimately index the tree. After
        // that a frame descends the path to the animation and nothing else, so
        // a page ten times as long costs the same.
        let settled = |frames: &[u64]| {
            frames[6..frames.len() - 3]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
        };
        assert!(
            settled(&many) <= settled(&few) + 4,
            "glass={glass}: a frame of the fade descended {} elements over 200 rows and {} over 20; per frame: {many:?} / {few:?}",
            settled(&many),
            settled(&few)
        );
    }
}
