//! An element that stops animating returns to its resting place.
//!
//! The traversal clears a retained compositor animation for every element that
//! reports none. These tests pin what that clearing is for, so that doing it
//! only where an animation can exist stays invisible.

use aimer_widget::{CompositorAnimationDecision, CompositorAnimationFrame, CompositorTransform};

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Resting,
    Compositor(f32),
    Live(f32),
}

struct Probe {
    mode: Rc<Cell<Mode>>,
}

impl Widget for Probe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        ProbeElement { mode: self.mode }.boxed()
    }
}

impl aimer_widget::PortableWidget for Probe {}

struct ProbeElement {
    mode: Rc<Cell<Mode>>,
}

impl VisitorElement for ProbeElement {
    fn debug_name(&self) -> &'static str {
        "CompositorProbe"
    }
}

impl EventElement for ProbeElement {}
impl Rebuildable for ProbeElement {}

impl LayoutElement for ProbeElement {
    fn size(&self) -> Option<Size> {
        Some(Size::new(Dimension::Px(100.0), Dimension::Px(50.0)))
    }
}

impl Drawable for ProbeElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        canvas.fill_rect(Rect::new(0.0, 0.0, 100.0, 50.0), [200, 40, 40, 255]);
        canvas.finish();
    }

    fn compositor_animation(&self, _ctx: &BuildContext) -> CompositorAnimationDecision {
        let frame = |x: f32| {
            CompositorAnimationFrame::new(
                0.5,
                CompositorTransform::Translate { x, y: 0.0 },
                None,
                None,
                false,
                true,
            )
        };
        match self.mode.get() {
            Mode::Resting => CompositorAnimationDecision::None,
            Mode::Compositor(x) => CompositorAnimationDecision::Compositor(frame(x)),
            Mode::Live(x) => CompositorAnimationDecision::Live(frame(x)),
        }
    }
}

struct Scene {
    app: HeadlessAimerApp<aimer_modal::ModalHost<Probe>>,
    mode: Rc<Cell<Mode>>,
}

impl Scene {
    fn new() -> Self {
        let mode = Rc::new(Cell::new(Mode::Resting));
        let mut app = AimerApp::start_headless_with(
            Probe { mode: mode.clone() },
            HeadlessOptions {
                size: PhysicalSize::new(200, 100),
                scale_factor: 1.0,
            },
        );
        for _ in 0..4 {
            app.render_frame();
        }
        Self { app, mode }
    }

    /// Draws one full frame in `mode` and returns where the probe ended up.
    fn frame_in(&mut self, mode: Mode) -> Option<f32> {
        self.mode.set(mode);
        self.app.app.request_full_redraw();
        self.app.render_frame();
        self.left_edge()
    }

    /// The left edge of the probe's drawn rectangle, in logical pixels.
    fn left_edge(&self) -> Option<f32> {
        self.app
            .app
            .render_tree()
            .render_all()
            .into_iter()
            .find_map(|operation| match operation {
                aimer_cupid::draw_cmd_v2::RenderOp::Draw(item)
                    if (item.bounds.width - 100.0).abs() < 0.01 =>
                {
                    Some(item.bounds.x)
                }
                _ => None,
            })
    }
}

#[test]
fn a_compositor_animation_moves_the_element_and_its_end_puts_it_back() {
    let mut scene = Scene::new();
    let rest = scene.left_edge().expect("the probe is drawn");

    assert_eq!(scene.frame_in(Mode::Compositor(30.0)), Some(rest + 30.0));
    assert_eq!(scene.frame_in(Mode::Compositor(45.0)), Some(rest + 45.0));
    assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
    assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
}

#[test]
fn a_live_frame_does_not_leave_a_retained_animation_behind() {
    let mut scene = Scene::new();
    let rest = scene.left_edge().expect("the probe is drawn");

    assert_eq!(scene.frame_in(Mode::Compositor(30.0)), Some(rest + 30.0));
    // The live frame paints nothing and clears the retained animation.
    assert_ne!(scene.frame_in(Mode::Live(30.0)), Some(rest + 30.0));
    assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
}

#[test]
fn an_element_that_never_animates_stays_where_it_is() {
    let mut scene = Scene::new();
    let rest = scene.left_edge().expect("the probe is drawn");
    for _ in 0..5 {
        assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
    }
}

#[test]
fn an_animation_can_start_again_after_the_element_rested() {
    let mut scene = Scene::new();
    let rest = scene.left_edge().expect("the probe is drawn");

    for round in 0..3 {
        let shift = 10.0 + round as f32 * 7.0;
        assert_eq!(scene.frame_in(Mode::Compositor(shift)), Some(rest + shift));
        assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
        assert_eq!(scene.frame_in(Mode::Resting), Some(rest));
    }
}
