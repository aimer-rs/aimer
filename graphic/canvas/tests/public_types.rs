//! What an application needs to draw with a [`Canvas`] is reachable from the
//! canvas crate itself, so it does not have to depend on the renderer's crate.

use std::any::TypeId;

use aimer_canvas::{Canvas, Mat3, Rect, RenderColor};
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderTree};

#[test]
fn the_types_the_canvas_draws_with_are_exported_and_are_the_renderers_own() {
    assert_eq!(TypeId::of::<Rect>(), TypeId::of::<aimer_cupid::utilities::Rect>());
    assert_eq!(TypeId::of::<RenderColor>(), TypeId::of::<aimer_cupid::utilities::Color>());
    assert_eq!(TypeId::of::<Mat3>(), TypeId::of::<aimer_cupid::utilities::Mat3>());
}

#[test]
fn a_rounded_rect_can_be_drawn_with_nothing_but_the_canvas_crates_types() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let context = tree.context(root).unwrap();
    let canvas = Canvas::of(&context);

    canvas.fill_rect_styled(
        Rect::new(0.0, 0.0, 20.0, 20.0),
        RenderColor::rgba8(10, 20, 30, 255),
        [4.0; 4],
        [0.0; 4],
        RenderColor::transparent(),
        [0.0; 4],
        RenderColor::transparent(),
    );
    assert_eq!(canvas.finish(), 1);

    let list = tree.draw_list_snapshot(root).unwrap();
    assert!(matches!(
        &list.commands[..],
        [DrawCommand::FillRect { color, .. }] if (color.r, color.g, color.b) == (10, 20, 30)
    ));
}
