//! Text inside an animated `Scalable` follows the scale as it changes.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_animation::{curves, ImplicitAnimatedBuilder};
use aimer_container::{Container, Scalable};
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderOp};
use aimer_widget::StateUpdater;

struct RootState {
    target: f32,
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

struct Root {
    updater: Rc<RefCell<Option<StateUpdater<RootState>>>>,
}

impl StatefulWidget for Root {
    type State = RootState;

    fn create_state(self) -> RootState {
        RootState { target: 1.0, updater: self.updater }
    }
}

impl State<Root> for RootState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        ImplicitAnimatedBuilder::new(
            self.target,
            Duration::from_millis(100),
            curves::LINEAR,
            |scale: &f32| {
                Scalable::new().scale(*scale).child(
                    Container::new().width(100).height(40).color(Color::BLUE)
                        .child(aimer_text::Text::new("Back")),
                )
            },
        )
    }
}

impl Widget for Root {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ScalableTextRoot", None).0.boxed()
    }
}

impl aimer_widget::PortableWidget for Root {}

fn text_scale<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> f32 {
    let item = app.app.render_tree().render_all().into_iter().find_map(|op| match op {
        RenderOp::Draw(item) if item.snapshot().commands.iter()
            .any(|cmd| matches!(cmd, DrawCommand::DrawText { .. })) => Some(item),
        _ => None,
    }).expect("text must remain visible");
    let (x0, _) = item.transform.transform_point(0.0, 0.0);
    let (x1, _) = item.transform.transform_point(1.0, 0.0);
    x1 - x0
}

#[test]
fn text_follows_an_animated_scalable() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let updater = Rc::new(RefCell::new(None));
    let mut app = AimerApp::start_headless_with(
        Root { updater: updater.clone() },
        HeadlessOptions { size: PhysicalSize::new(300, 200), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    assert_eq!(text_scale(&app), 1.0);

    updater.borrow().expect("initialised").set_state(|state| state.target = 2.0);
    let end = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(8));
        app.pump_frames(1);
    }
    assert!((text_scale(&app) - 2.0).abs() < 1e-3, "text scale was {}", text_scale(&app));
}
