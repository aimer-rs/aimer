//! A looping rotation repaints only the rotating icon, not the window.

use super::*;

use std::time::Duration;

use aimer_animation::{AnimationController, curves, RotationTransition};
use aimer_svg::{Svg, SvgDocument};
use aimer_text::Text;

pub(super) fn loading_page() -> impl Widget {
    let controller = AnimationController::new(Duration::from_millis(1500), curves::LINEAR);
    controller.set_repeat(true);
    controller.set_curve(curves::EASE_IN_OUT);
    controller.forward_from_first_tick();
    let icon = Svg::new(
        SvgDocument::from_svg(include_bytes!(
            "../../../../../app/jaime/assets/loading-1-svgrepo-com.svg"
        ))
        .expect("the bundled loading icon SVG should be valid"),
    )
    .bounded()
    .width(aimer_attribute::Dimension::Px(96.0))
    .height(aimer_attribute::Dimension::Px(96.0));

    aimer_container::Container::new()
        .width(aimer_attribute::Dimension::Percent(100.0))
        .height(aimer_attribute::Dimension::Percent(100.0))
        .color(Color::WHITE)
        .child(
            Column::new()
                .horizontal_alignment(aimer_flex::BoxAlignment::Center)
                .vertical_alignment(aimer_flex::BoxAlignment::Center)
                .children(vec![
                    RotationTransition::new(controller, icon).boxed(),
                    SizedBox::new().height(16.0).boxed(),
                    Text::new("Loading...").boxed(),
                ]),
        )
}

#[test]
fn a_looping_rotation_repaints_a_small_region_every_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless_with(
        loading_page(),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let window_area = 1280u64 * 800;
    for frame in 0..12 {
        // The controller is clock-driven, so let some time pass between frames.
        std::thread::sleep(Duration::from_millis(16));
        let (_, packet) = direct_headless_frame_packet(&mut app);
        let damage = packet.metadata().damage();
        let area: u64 = damage
            .regions()
            .iter()
            .map(|r| u64::from(r.width) * u64::from(r.height))
            .sum();
        assert!(
            !damage.is_full(),
            "frame {frame}: a spinner must not repaint the window"
        );
        assert!(
            !damage.is_empty(),
            "frame {frame}: the rotation must be presented"
        );
        assert!(
            area * 20 < window_area,
            "frame {frame}: damage {area}px is over 5% of the window"
        );
    }
}
