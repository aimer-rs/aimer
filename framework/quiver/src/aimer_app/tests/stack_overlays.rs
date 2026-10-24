//! Bottom overlays occupy their full layout box, including transparent insets.

use super::*;

use aimer_container::Container;
use aimer_cupid::draw_cmd_v2::{DrawCommand as V2DrawCommand, RenderOp};
use aimer_space::{Align, Alignment, Positioned, Stack};
use aimer_style::LayoutSpacing;

fn pill() -> impl Widget {
    Container::new().width(260).height(80)
        .padding(LayoutSpacing::new().bottom(12))
        .child(SizedBox::new().width(260).height(68).color(Color::Rgb(255, 107, 82)))
}

fn pill_bounds<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Rect {
    let tree = app.app.render_tree();
    tree.render_all().into_iter().find_map(|op| {
        let RenderOp::Draw(item) = op else { return None };
        let paint = tree.draw_list_snapshot(item.element).unwrap();
        paint.commands.iter().any(|command| matches!(command,
            V2DrawCommand::FillRect { color, .. }
                if color.r == 255 && color.g == 107 && color.b == 82))
            .then(|| tree.element_bounds(item.element).unwrap())
    }).expect("the floating pill must paint")
}

#[test]
fn top_level_stack_positions_full_padded_box() {
    for scale in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().children([
                SizedBox::new().width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0)).boxed(),
                Positioned::new().left(20).bottom(20).child(pill()).boxed(),
            ]),
            HeadlessOptions {
                size: PhysicalSize::new((400.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(pill_bounds(&app), Rect::new(20.0, 700.0, 260.0, 68.0));
        assert!(app.bounds_audit().unwrap().interaction_disagreements.is_empty());
    }
}

#[test]
fn top_level_stack_aligns_full_padded_box() {
    for scale in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().children([
                SizedBox::new().width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0)).boxed(),
                Align::new().alignment(Alignment::BotCenter).child(pill()).boxed(),
            ]),
            HeadlessOptions {
                size: PhysicalSize::new((400.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(pill_bounds(&app), Rect::new(70.0, 720.0, 260.0, 68.0));
    }
}

/// A pill whose wrapper has an inset but no size of its own, as a floating bar
/// is usually written: the wrapper should take the pill's size plus the inset.
fn unsized_wrapper_pill() -> impl Widget {
    Container::new()
        .padding(LayoutSpacing::new().bottom(12))
        .child(SizedBox::new().width(260).height(68).color(Color::Rgb(255, 107, 82)))
}

#[test]
fn top_level_stack_aligns_unsized_wrapper_to_the_bottom_center() {
    for scale in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().children([
                SizedBox::new().width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0)).boxed(),
                Align::new().alignment(Alignment::BotCenter)
                    .child(unsized_wrapper_pill()).boxed(),
            ]),
            HeadlessOptions {
                size: PhysicalSize::new((400.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(pill_bounds(&app), Rect::new(70.0, 720.0, 260.0, 68.0));
    }
}

#[test]
fn top_level_stack_positions_unsized_wrapper_from_the_bottom_left() {
    for scale in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().children([
                SizedBox::new().width(Dimension::Percent(100.0))
                    .height(Dimension::Percent(100.0)).boxed(),
                Positioned::new().left(20).bottom(20)
                    .child(unsized_wrapper_pill()).boxed(),
            ]),
            HeadlessOptions {
                size: PhysicalSize::new((400.0 * scale) as u32, (800.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(pill_bounds(&app), Rect::new(20.0, 700.0, 260.0, 68.0));
    }
}

/// The bar as an app writes it: the unsized wrapper lives inside a stateless
/// widget, so the element the parent measures is a `StatelessElement` around it.
fn floating_bar() -> impl Widget {
    aimer_widget::NamedWidget::new(unsized_wrapper_pill().boxed(), "FloatingBar")
}

fn start_over_a_full_stack<W: Widget + 'static>(
    placed: W,
    scale: f64,
) -> HeadlessAimerApp<impl Widget> {
    let mut app = AimerApp::start_headless_with(
        Stack::new().children([
            SizedBox::new().width(Dimension::Percent(100.0))
                .height(Dimension::Percent(100.0)).boxed(),
            placed.boxed(),
        ]),
        HeadlessOptions {
            size: PhysicalSize::new((400.0 * scale) as u32, (800.0 * scale) as u32),
            scale_factor: scale,
        },
    );
    app.pump_frames(3);
    app
}

#[test]
fn a_stateless_bar_with_an_unsized_wrapper_floats_at_the_bottom_center() {
    for scale in [1.0, 2.0] {
        let app = start_over_a_full_stack(
            Align::new().alignment(Alignment::BotCenter).child(floating_bar()),
            scale,
        );
        assert_eq!(pill_bounds(&app), Rect::new(70.0, 720.0, 260.0, 68.0));
    }
}

#[test]
fn a_stateless_bar_with_an_unsized_wrapper_is_positioned_from_its_edges() {
    for scale in [1.0, 2.0] {
        let app = start_over_a_full_stack(
            Positioned::new().right(10).top(30).child(floating_bar()),
            scale,
        );
        // 400 - 10 - 260 from the left, 30 from the top.
        assert_eq!(pill_bounds(&app), Rect::new(130.0, 30.0, 260.0, 68.0));
    }
}

#[test]
fn the_margin_of_an_unsized_wrapper_is_part_of_the_box_that_is_aligned() {
    for scale in [1.0, 2.0] {
        let app = start_over_a_full_stack(
            Align::new().alignment(Alignment::BotCenter).child(
                Container::new()
                    .margin(LayoutSpacing::all(10))
                    .child(SizedBox::new().width(260).height(68).color(Color::Rgb(255, 107, 82))),
            ),
            scale,
        );
        // The 280 x 88 outer box sits on the bottom edge (y = 712) and is centred.
        assert_eq!(pill_bounds(&app), Rect::new(70.0, 722.0, 260.0, 68.0));
    }
}

#[test]
fn a_row_under_align_still_fills_instead_of_shrinking_to_its_widest_child() {
    for scale in [1.0, 2.0] {
        let app = start_over_a_full_stack(
            Align::new().alignment(Alignment::BotCenter).child(
                Container::new().child(
                    aimer_flex::Row::new().children([
                        SizedBox::new().width(50).height(30).color(Color::Rgb(10, 20, 30)).boxed(),
                        SizedBox::new().width(50).height(30).color(Color::Rgb(255, 107, 82)).boxed(),
                    ]),
                ),
            ),
            scale,
        );
        // A row is as wide as the stack, so centring it moves nothing sideways.
        assert_eq!(pill_bounds(&app).x, 50.0);
        assert_eq!(pill_bounds(&app).width, 50.0);
    }
}
