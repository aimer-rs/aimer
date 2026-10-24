//! The decoration pages keep one retained child per position, so a resize can
//! synchronize the render tree. A `ChildBuilder` placed in two positions puts
//! one element in the tree twice, the synchronization fails, and the window
//! keeps showing its last consistent frame at the old size.

use super::*;

use aimer::HeadlessOptions;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::quiver::winit::event::WindowEvent;
use aimer_cupid::draw_cmd_v2::RenderOp;

struct Page(&'static str);
impl aimer::PortableWidget for Page {}
impl Widget for Page {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == self.0)
            .expect("the example must be selectable in Jaime");
        build_example(example, theme::app_theme()).to_element(ctx)
    }
}

/// The widest node the retained plan draws, in logical pixels.
fn widest(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> f32 {
    app.render_ops()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.bounds.width),
            _ => None,
        })
        .fold(0.0, f32::max)
}

fn follows_the_window_through_resizes(key: &'static str) {
    for scale in [1.0_f64, 2.0] {
        let mut app = AimerApp::start_headless_with(
            theme::provide(Page(key)),
            HeadlessOptions {
                size: PhysicalSize::new((900.0 * scale) as u32, (700.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(widest(&app), 900.0, "{key} at scale {scale}");
        for width in [1300.0_f32, 1000.0, 1500.0] {
            app.send_window_event(WindowEvent::Resized(PhysicalSize::new(
                (f64::from(width) * scale) as u32,
                (900.0 * scale) as u32,
            )));
            app.pump_frames(3);
            assert_eq!(
                widest(&app),
                width,
                "{key}: the tree follows a {width}px window at scale {scale}"
            );
        }
    }
}

#[test]
fn gradient_page_follows_the_window_through_resizes() {
    follows_the_window_through_resizes("gradient");
}

#[test]
fn foreground_decoration_page_follows_the_window_through_resizes() {
    follows_the_window_through_resizes("foreground-decoration");
}
