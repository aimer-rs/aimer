//! The paint-owner census tells a layer that records commands from a wrapper
//! that only passes the walk through.

use super::*;

use aimer_container::Container;
use aimer_input::mouse_region::MouseRegion;

#[test]
fn a_filled_container_records_and_a_hover_region_around_it_does_not() {
    let widget = MouseRegion::new().box_child(
        Container::new()
            .width(80)
            .height(40)
            .color(Color::Rgb(200, 40, 40))
            .child(SizedBox::new().width(10).height(10)),
    );
    let mut app = AimerApp::start_headless_with(
        widget,
        HeadlessOptions {
            size: PhysicalSize::new(200, 120),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);

    let census = app
        .paint_owner_census()
        .expect("a root is mounted after the first frames");
    assert_eq!(census.unmapped, 0, "{census:?}");
    let region = census
        .by_type
        .iter()
        .find(|(name, _)| name.contains("MouseRegion"))
        .expect("the hover region is in the tree")
        .1;
    assert_eq!((region.with_commands, region.pass_through), (0, 1), "{census:?}");
    let container = census
        .by_type
        .iter()
        .find(|(name, _)| name.contains("Container"))
        .expect("the container is in the tree")
        .1;
    assert_eq!(container.with_commands, 1, "{census:?}");
    assert!(container.commands >= 1, "{census:?}");
}
