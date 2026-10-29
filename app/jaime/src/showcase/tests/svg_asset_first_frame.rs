//! A bundled SVG must draw as soon as it has loaded, without a resize.
//!
//! Each test builds its page in its own process (nextest's default): the first
//! load of an asset is the case under test, and a second page in the same
//! process would find the SVG already cached and ready when made.

use std::time::Duration;

use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::style::BoxFit;
use aimer::{
    AimerApp, AssetImage, Container, Dimension, HeadlessOptions, Row, SizedBox, SvgAsset, Widget,
};
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderOp};

use crate::Assets;

/// Mounts `page`, then lets the SVG load the way a window would: a task that
/// becomes ready wakes the loop with a *scoped* frame request, and frames are
/// delivered only when something asked for one. Forcing a full frame here would
/// hide a load whose result is never drawn.
fn svg_is_drawn_after_loading(page: impl Widget + 'static) -> bool {
    let mut app = AimerApp::start_headless_with(
        page,
        HeadlessOptions {
            size: PhysicalSize::new(800, 600),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(6);
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(20));
        aimer::events::window::request_scoped_frame();
        app.pump_frames(6);
    }

    app.render_ops().iter().any(|op| match op {
        RenderOp::Draw(item) => item
            .snapshot()
            .commands
            .iter()
            .any(|command| matches!(command, DrawCommand::Svg { .. })),
        _ => false,
    })
}

fn svg() -> impl Widget {
    SvgAsset::new(Assets::CAT_SVGREPO_COM).width(120).height(120)
}

/// An image beside the SVG, in a row of its own: the layout around the SVG is computed and cached
/// while it is still loading, so the finished load has to invalidate it.
#[test]
fn an_svg_beside_an_image_draws_once_it_has_loaded() {
    let page = Row::new().children([Row::new()
        .children([
            Container::new()
                .width(Dimension::Px(240.0))
                .child(AssetImage::new(Assets::MY_IMAGE).fit(BoxFit::FitWidth))
                .boxed(),
            SizedBox::new().width(24).boxed(),
            svg().boxed(),
        ])
        .boxed()]);

    assert!(svg_is_drawn_after_loading(page), "the loaded SVG was never drawn");
}

#[test]
fn the_bundled_assets_page_draws_its_svg_once_it_has_loaded() {
    let page = crate::theme::provide(crate::bundled_assets_example::bundled_assets_example());

    assert!(svg_is_drawn_after_loading(page), "the loaded SVG was never drawn");
}
