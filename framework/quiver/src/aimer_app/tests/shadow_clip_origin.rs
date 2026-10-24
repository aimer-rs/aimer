//! A node whose shadow grows its bounds outward keeps the clip its parent gave
//! it where the parent meant it.
//!
//! The parent states the clip relative to the child's layout box, while the
//! render node's local origin is the grown corner. A clip that is not moved by
//! the growth lands that far up and to the left, and a floating pill is cut off
//! at its right and bottom edges by a clip that no longer covers it.

use super::*;

use aimer_container::Container;
use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use aimer_space::{Align, Alignment, Stack};
use aimer_style::{BoxDecoration, BoxShadow, LayoutSpacing};

const SURFACE: (u8, u8, u8) = (38, 26, 23);

fn pill() -> impl Widget {
    // The same shape as a floating bar: an unsized wrapper with an inset around
    // a rounded, shadowed pill that holds a row of buttons.
    let button = || {
        Container::new().width(Dimension::Px(48.0)).height(Dimension::Px(48.0))
            .box_decoration(BoxDecoration::new()
                .background_color(Color::Rgb(255, 107, 82)).border_radius(24.0))
            .child(aimer_container::ZeroSizedBox).boxed()
    };
    Align::new().alignment(Alignment::BotCenter).box_child(
        Container::new().padding(LayoutSpacing::new().bottom(12)).child(
            Container::new().width(Dimension::Px(236.0)).height(Dimension::Px(68.0))
                .padding(LayoutSpacing::all(10))
                .box_decoration(BoxDecoration::new()
                    .background_color(Color::Rgb(SURFACE.0, SURFACE.1, SURFACE.2))
                    .border_radius(34.0)
                    .box_shadow(vec![BoxShadow::new()
                        .color(Color::BLACK.with_opacity(40)).blur(18.0).offset_y(6.0)]))
                .child(aimer_flex::Row::new()
                    .gaps(LayoutSpacing::horizontal(8u32.into()))
                    .children([button(), button(), button(), button()]))))
}

#[test]
fn a_shadowed_pill_is_not_clipped_short_of_its_own_box() {
    for scale in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().children([
                SizedBox::new().width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0)).boxed(),
                aimer_widget::NamedWidget::new(pill().boxed(), "FloatingBar").boxed(),
            ]),
            HeadlessOptions {
                size: PhysicalSize::new((376.0 * scale) as u32, (300.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);

        let tree = app.app.render_tree();
        let (surface_bounds, clip) = tree.render_all().into_iter().find_map(|op| {
            let RenderOp::Draw(item) = op else { return None };
            let paint = tree.draw_list_snapshot(item.element).unwrap();
            paint.commands.iter().any(|command| matches!(command,
                V2DrawCommand::FillRect { color, .. }
                    if (color.r, color.g, color.b) == SURFACE))
                .then_some((item.bounds, item.clip))
        }).expect("the pill surface must paint");

        // The pill is 236 x 68 at (70, 220). Its wrapper, 236 x 80, is what
        // clips it, so the clip is the wrapper's box in the same space.
        assert_eq!(clip, Some(Rect::new(70.0, 220.0, 236.0, 80.0)),
            "the clip must sit on the wrapper, not on the shadow's grown corner \
             {surface_bounds:?}");
    }
}
