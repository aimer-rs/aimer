//! The gradient page is taller than a small window, so the wheel must scroll it.

use super::*;

use aimer::HeadlessOptions;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
use aimer_cupid::draw_cmd_v2::RenderOp;

struct Page;
impl aimer::PortableWidget for Page {}
impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == "gradient")
            .expect("Gradients must be selectable in Jaime");
        build_example(example, theme::app_theme()).to_element(ctx)
    }
}

/// The top edge of the first row of gradient cards, `None` once they are gone.
fn card_top(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> Option<f32> {
    app.render_ops()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) if item.bounds.width == 220.0 && item.bounds.height == 120.0 => {
                Some(item.bounds.y)
            }
            _ => None,
        })
        .reduce(f32::min)
}

#[test]
fn the_gradient_page_scrolls_with_the_wheel() {
    let mut app = AimerApp::start_headless_with(
        theme::provide(Page),
        HeadlessOptions {
            size: PhysicalSize::new(900, 360),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let before = card_top(&app).expect("the first row of cards is on screen");

    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(450.0, 180.0),
    });
    app.render_frame();
    for _ in 0..4 {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -60.0)),
            phase: TouchPhase::Moved,
        });
        app.render_frame();
    }
    app.pump_frames(20);
    let after = card_top(&app).map_or(f32::NEG_INFINITY, |top| top);
    assert!(after < before, "the cards move up as the page scrolls: {before} -> {after}");
}
