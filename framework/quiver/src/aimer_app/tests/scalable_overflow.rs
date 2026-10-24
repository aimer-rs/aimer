use super::*;
use aimer_container::{Container, Scalable};
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderItem, RenderOp};

fn blue_draw<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> RenderItem {
    app.app.render_tree().render_all().into_iter().find_map(|op| match op {
        RenderOp::Draw(item) if item.snapshot().commands.iter().any(|cmd| matches!(cmd,
            DrawCommand::FillRect { color, .. } if (color.r, color.g, color.b) == (0, 0, 255)
        )) => Some(item),
        _ => None,
    }).expect("scaled child must remain visible")
}

fn scaled_box() -> impl Widget {
    Container::new().width(100).height(40).child(
        Container::new().width(100).height(40).child(
            Scalable::new().scale(2.0)
                .child(SizedBox::new().width(100).height(40).color(Color::BLUE)),
        ),
    )
}

#[test]
fn scalable_overflows_nested_containers() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(scaled_box());
    app.pump_frames(3);
    let draw = blue_draw(&app);
    assert_eq!(draw.bounds, Rect::new(-50.0, -20.0, 200.0, 80.0));
    assert_eq!(draw.clip, None, "ordinary container clips must not cut off scaling");
}

#[test]
fn scalable_escapes_ancestor_scroll_viewports() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(
        SizedBox::new().width(110).height(50).child(Scrollable::new().child(scaled_box())),
    );
    app.pump_frames(3);
    let draw = blue_draw(&app);
    assert_eq!(draw.bounds, Rect::new(-50.0, -20.0, 200.0, 80.0));
    assert_eq!(draw.clip, None);
}

#[test]
fn scalable_changes_rendering_without_moving_a_row_sibling() {
    use aimer_flex::OverflowBehavior;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    for (scale, x, y, width, height) in [
        (0.5, 25.0, 10.0, 50.0, 20.0), (1.0, 0.0, 0.0, 100.0, 40.0),
        (2.0, -50.0, -20.0, 200.0, 80.0),
        (0.0, 0.0, 0.0, 100.0, 40.0), (-1.0, 0.0, 0.0, 100.0, 40.0),
        (f32::NAN, 0.0, 0.0, 100.0, 40.0), (f32::INFINITY, 0.0, 0.0, 100.0, 40.0),
    ] {
        let mut app = AimerApp::start_headless(Row::new().overflow(OverflowBehavior::Visible).children([
            Scalable::new().scale(scale)
                .child(SizedBox::new().width(100).height(40).color(Color::BLUE)).boxed(),
            SizedBox::new().width(10).height(40).color(Color::RED).boxed(),
        ]));
        app.pump_frames(3);
        assert_eq!(blue_draw(&app).bounds, Rect::new(x, y, width, height));
        let sibling = app.app.render_tree().render_all().into_iter().find_map(|op| match op {
            RenderOp::Draw(item) if item.snapshot().commands.iter().any(|cmd| matches!(cmd,
                DrawCommand::FillRect { color, .. } if (color.r, color.g, color.b) == (255, 0, 0)
            )) => Some(item),
            _ => None,
        }).unwrap();
        assert_eq!(sibling.bounds, Rect::new(100.0, 0.0, 10.0, 40.0));
    }
}

#[test]
fn scalable_escapes_ancestor_hidden_overflow() {
    use aimer_flex::OverflowBehavior;

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(
        SizedBox::new().width(110).height(50)
            .child(Row::new().overflow(OverflowBehavior::Hidden).children([scaled_box()])),
    );
    app.pump_frames(3);
    assert_eq!(blue_draw(&app).clip, None);
}

#[test]
fn scalable_escapes_margin_padding_and_rounded_clips_at_both_display_scales() {
    use aimer_style::{BoxDecoration, LayoutSpacing};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    for scale_factor in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Container::new().width(100).height(60).margin(LayoutSpacing::all(20))
                .padding(LayoutSpacing::all(10))
                .box_decoration(BoxDecoration::new().border_radius(12))
                .child(Row::new().children([
                    Scalable::new().scale(2.0).child(
                        SizedBox::new().width(80).height(40).color(Color::BLUE),
                    ),
                ])),
            HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor },
        );
        app.pump_frames(3);
        let draw = blue_draw(&app);
        assert_eq!(draw.bounds, Rect::new(-10.0, 10.0, 160.0, 80.0));
        assert_eq!(draw.clip, None);
        assert_eq!(draw.clip_radius, [0.0; 4]);
    }
}

#[test]
fn scalable_keeps_the_center_of_positioned_bounds_at_both_display_scales() {
    use aimer_space::{Positioned, Stack};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    for scale_factor in [1.0, 2.0] {
        let mut app = AimerApp::start_headless_with(
            Stack::new().add_child(Positioned::new().left(60).top(50).child(
                Scalable::new().scale(2.0)
                    .child(Container::new().width(40).height(20)
                        .padding(aimer_style::LayoutSpacing::all(3)).color(Color::BLUE)
                        .child(SizedBox::new())),
            )),
            HeadlessOptions { size: PhysicalSize::new(400, 300), scale_factor },
        );
        app.pump_frames(3);
        assert_eq!(blue_draw(&app).bounds, Rect::new(40.0, 40.0, 80.0, 40.0));
    }
}

#[test]
fn scalable_centers_the_layout_box_independently_of_shadow_outsets() {
    use aimer_style::{BoxDecoration, BoxShadow};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(Scalable::new().scale(2.0).child(
        Container::new().width(100).height(40).box_decoration(
            BoxDecoration::new().background_color(Color::BLUE)
                .add_shadow(BoxShadow::new().spread(4.0).offset_x(2.0).offset_y(-2.0)),
        ).child(SizedBox::new()),
    ));
    app.pump_frames(3);
    let draw = blue_draw(&app);
    let rect = draw.snapshot().commands.iter().find_map(|command| match command {
        DrawCommand::FillRect { rect, color, .. } if (color.r, color.g, color.b) == (0, 0, 255) => Some(*rect),
        _ => None,
    }).unwrap();
    assert_eq!(draw.transform.transform_point(draw.origin.0 + rect.x, draw.origin.1 + rect.y),
        (-50.0, -20.0));
    assert_eq!(draw.transform.transform_point(draw.origin.0 + rect.x + rect.width,
        draw.origin.1 + rect.y + rect.height), (150.0, 60.0));
}

fn text_draw<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> RenderItem {
    app.app.render_tree().render_all().into_iter().find_map(|op| match op {
        RenderOp::Draw(item) if item.snapshot().commands.iter().any(|cmd| matches!(cmd,
            DrawCommand::DrawText { .. }
        )) => Some(item),
        _ => None,
    }).expect("scaled text must remain visible")
}

#[test]
fn scalable_scales_descendant_text_without_growing_its_layout() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(Scalable::new().scale(2.0).child(
        Container::new().width(100).height(40).color(Color::BLUE)
            .child(aimer_text::Text::new("Back")),
    ));
    app.pump_frames(3);
    let draw = text_draw(&app);
    let (x0, _) = draw.transform.transform_point(0.0, 0.0);
    let (x1, _) = draw.transform.transform_point(1.0, 0.0);
    assert_eq!(x1 - x0, 2.0, "text must render at the Scalable's scale");
    assert_eq!(blue_draw(&app).bounds, Rect::new(-50.0, -20.0, 200.0, 80.0));
}

#[test]
fn scalable_scales_text_inside_a_decoration_box() {
    use aimer_decoration::{BorderRadius, LightEdge, Magnification, MagnificationStyle, MultiDecorationBox};

    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut app = AimerApp::start_headless(Scalable::new().scale(2.0).child(
        MultiDecorationBox::new()
            .decoration(BorderRadius::from(50.0))
            .decoration(Magnification::new().zoom(1.3).thickness(15.0).bulge(0.5)
                .style(MagnificationStyle::REFRACTION | MagnificationStyle::UNIFORM
                    | MagnificationStyle::BULGE))
            .decoration(LightEdge::new().strength(2.2).width(3.0))
            .child(Container::new().width(100).height(40).color(Color::BLUE)
                .child(aimer_text::Text::new("Back")
                    .text_align(aimer_style::TextAlign::MidCenter))),
    ));
    app.pump_frames(3);
    let draw = text_draw(&app);
    let (x0, _) = draw.transform.transform_point(0.0, 0.0);
    let (x1, _) = draw.transform.transform_point(1.0, 0.0);
    assert_eq!(x1 - x0, 2.0, "text must render at the Scalable's scale");
}
