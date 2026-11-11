//! An element that declares it paints nothing is not asked about compositor
//! animations, because it can never have one; every other element still is.

use aimer_widget::CompositorAnimationDecision;

use super::*;

struct Probe {
    transparent: bool,
    samples: Rc<Cell<usize>>,
}

impl Widget for Probe {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        ProbeElement {
            transparent: self.transparent,
            samples: self.samples,
        }
        .boxed()
    }
}

struct ProbeElement {
    transparent: bool,
    samples: Rc<Cell<usize>>,
}

impl VisitorElement for ProbeElement {
    fn debug_name(&self) -> &'static str {
        "TransparentProbe"
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
    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }

    fn paints_nothing(&self) -> bool {
        self.transparent
    }

    fn compositor_animation(&self, _ctx: &BuildContext) -> CompositorAnimationDecision {
        self.samples.set(self.samples.get() + 1);
        CompositorAnimationDecision::None
    }
}

/// Compositor samples the probe was asked for over six full frames.
fn samples_over_frames(transparent: bool) -> usize {
    let samples = Rc::new(Cell::new(0));
    let mut app = AimerApp::start_headless_with(
        Probe {
            transparent,
            samples: samples.clone(),
        },
        HeadlessOptions {
            size: PhysicalSize::new(200, 100),
            scale_factor: 1.0,
        },
    );
    for _ in 0..6 {
        app.app.request_full_redraw();
        app.render_frame();
    }
    samples.get()
}

#[test]
fn a_transparent_element_is_never_sampled_for_a_compositor_animation() {
    assert_eq!(samples_over_frames(true), 0);
}

#[test]
fn an_element_that_may_paint_is_still_sampled_every_frame() {
    assert!(samples_over_frames(false) >= 6);
}
