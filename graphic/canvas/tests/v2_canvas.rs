use aimer_canvas::Canvas;
use aimer_canvas::{DrawShape, MaterialDrawRequest, MaterialKind, ShapeDrawResult};
use aimer_cupid::draw_cmd_v2::RichTextSegment;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::font::TextLanguage;
use aimer_cupid::text_pipeline::TextOverflowMode;
use aimer_cupid::utilities::{Color, Mat3, Vec2d};
use aimer_shape::{FillStyle, ShapeColor, ShapePath, ShapeSize};
use std::sync::Arc;

#[test]
fn canvas_commits_commands_to_its_element_local_list() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 12.0, 40.0, 30.0))
        .unwrap();
    let context = tree.context(child).unwrap();

    let canvas = Canvas::of(&context);
    canvas.fill_rect(Rect::new(0.0, 0.0, 40.0, 30.0), [10, 20, 30, 255]);
    canvas.draw_text("label", [2.0, 20.0], 14.0, [255, 255, 255, 255]);
    assert_eq!(canvas.finish(), 1);

    let child_list = tree.draw_list_snapshot(child).unwrap();
    assert_eq!(child_list.revision, 1);
    assert!(matches!(
        child_list.commands[0],
        DrawCommand::FillRect { .. }
    ));
    assert!(matches!(
        child_list.commands[1],
        DrawCommand::DrawText { .. }
    ));
    assert_eq!(tree.draw_list_revision(root).unwrap(), 0);
}

#[test]
fn try_finish_reports_invalid_scopes_and_drop_commits_valid_commands() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let context = tree.context(root).unwrap();

    let initial = Canvas::of(&context);
    initial.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), [1, 2, 3, 255]);
    initial.finish();
    let before = tree.draw_list_snapshot(root).unwrap();

    let unbalanced = Canvas::of(&context);
    unbalanced.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    assert!(unbalanced.try_finish().is_err());

    let after = tree.draw_list_snapshot(root).unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.commands.len(), before.commands.len());
    assert!(matches!(
        after.commands.first(),
        Some(DrawCommand::FillRect { .. })
    ));
    assert!(tree.dirty_elements().contains(&root));

    let dropped = Canvas::of(&context);
    dropped.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), [4, 5, 6, 255]);
    drop(dropped);

    let after_drop = tree.draw_list_snapshot(root).unwrap();
    assert_eq!(after_drop.revision, before.revision + 1);
    assert!(matches!(
        after_drop.commands.as_ref(),
        [DrawCommand::FillRect { color, .. }]
            if (color.r, color.g, color.b, color.a) == (4, 5, 6, 255)
    ));
    assert!(!tree.dirty_elements().contains(&root));
}

#[test]
fn canvas_records_builtin_paint_commands_and_balanced_state_scopes() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let context = tree.context(root).unwrap();
    let canvas = Canvas::of(&context);

    canvas.fill_rect_styled(
        Rect::new(0.0, 0.0, 20.0, 20.0),
        Color::white(),
        [2.0; 4],
        [1.0; 4],
        Color::black(),
        [0.0; 4],
        Color::transparent(),
    );
    canvas.draw_rich_text(
        Vec2d::new(1.0, 12.0),
        vec![RichTextSegment::new("rich")],
        12.0,
        Color::white(),
        None,
        None,
        TextOverflowMode::Clip,
    );
    canvas.draw_text_decoration(Rect::new(0.0, 14.0, 20.0, 2.0), Color::white(), 1, 1.0, 2.0);
    canvas.draw_image(Rect::new(0.0, 20.0, 8.0, 8.0), 9);
    canvas.draw_shadow_rect(
        Rect::new(0.0, 30.0, 20.0, 20.0),
        Color::black(),
        [0.0, 0.0, 2.0, 1.0],
        [0.0; 4],
        false,
        [0.0; 3],
    );
    canvas.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0), [0.0; 4]);
    canvas.push_transform(Mat3::identity());
    canvas.set_alpha(0.5);
    canvas.set_italic(true);
    canvas.set_text_language(Some(TextLanguage::Japanese));
    canvas.draw_text("scoped", [0.0, 10.0], 12.0, [255; 4]);
    canvas.set_text_language(None);
    canvas.set_italic(false);
    canvas.restore_alpha();
    canvas.pop_transform();
    canvas.pop_clip();
    canvas.finish();

    let commands = tree.draw_list_snapshot(root).unwrap().commands;
    assert_eq!(commands.len(), 16);
    assert!(matches!(commands[0], DrawCommand::FillRect { .. }));
    assert!(matches!(commands[1], DrawCommand::DrawRichText { .. }));
    assert!(matches!(
        commands[2],
        DrawCommand::DrawTextDecoration { .. }
    ));
    assert!(matches!(commands[3], DrawCommand::DrawImage { .. }));
    assert!(matches!(commands[4], DrawCommand::DrawShadowRect { .. }));
    assert!(matches!(commands[10], DrawCommand::DrawText { .. }));
}

#[test]
fn canvas_records_custom_shape_and_material_commands_in_the_local_list() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
    let context = tree.context(root).unwrap();
    let path = ShapePath::builder()
        .move_to(4.0, 4.0)
        .line_to(28.0, 4.0)
        .line_to(28.0, 28.0)
        .line_to(4.0, 28.0)
        .close()
        .build()
        .unwrap();
    let request = DrawShape::new(Arc::new(path)).fill(FillStyle::solid(ShapeColor::BLACK));
    let canvas = Canvas::of(&context);

    assert_eq!(
        canvas.draw_shape(&request, ShapeSize::new(32.0, 32.0)),
        ShapeDrawResult::Submitted
    );
    canvas.draw_material(MaterialDrawRequest::new(
        MaterialKind::Glass,
        [0.0, 0.0, 32.0, 32.0],
    ));
    canvas.finish();

    let commands = tree.draw_list_snapshot(root).unwrap().commands;
    assert!(matches!(commands[0], DrawCommand::Svg { .. }));
    assert!(matches!(
        &commands[1],
        DrawCommand::DrawCustom { pipeline_name, data }
            if pipeline_name.as_ref() == "aimer.material"
                && data.len() == MaterialDrawRequest::PACKET_LEN
    ));
}
