use super::*;
use aimer_attribute::Dimension;
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use crate::{BorderRadius, BorderSlice, BorderStyle, BoxBorder, BoxOutline, BoxShadow, ShadowSide};

fn size() -> ResolvedSize {
    ResolvedSize { width: 80.0, height: 40.0 }
}

fn record(decoration: &dyn Decoration, scale: f32, background: Option<Color>) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    let canvas = Canvas::of(&tree.context(node).unwrap());
    decoration.paint_with_background(&canvas, (3.0, 5.0).into(), size(), scale, background);
    canvas.finish();
    tree.draw_list_snapshot(node).unwrap().commands.to_vec()
}

fn slice(width: impl Into<Dimension>) -> BorderSlice {
    BorderSlice::new().stroke(width).style(BorderStyle::Solid).color(Color::BLUE)
}

#[test]
fn box_border_decoration_paints_at_origin_and_insets_in_logical_units() {
    let border = BoxBorder::new().left(slice(2.0)).top(slice(Dimension::Percent(10.0)))
        .right(slice(6.0)).bottom(slice(8.0));
    for scale in [1.0, 2.0] {
        assert_eq!(border.content_insets(size(), scale), [2.0, 4.0, 6.0, 8.0]);
        assert_eq!(border.paint_outsets(size(), scale), [0.0; 4]);
        assert!(border.child_clip_radius(size(), scale).is_none());
        let commands = record(&border, scale, Some(Color::RED));
        assert!(matches!(commands.as_slice(), [DrawCommand::FillRect {
            rect, color, border_width, border_color, outline_width, ..
        }] if *rect == Rect::new(3.0, 5.0, 80.0, 40.0) && color.r == 255
            && border_color.b == 255 && *border_width == [4.0, 6.0, 8.0, 2.0]
            && *outline_width == [0.0; 4]));
    }
    let invalid = BoxBorder::new().left(slice(-2.0)).right(slice(f32::NAN));
    assert_eq!(invalid.content_insets(size(), 1.0), [0.0; 4]);
    assert!(record(&BoxBorder::default(), 1.0, None).is_empty());
}

#[test]
fn box_outline_decoration_declares_outsets_without_insetting_or_clipping() {
    let outline = BoxOutline::new().left(slice(2.0)).top(slice(Dimension::Percent(10.0)))
        .right(slice(6.0)).bottom(slice(8.0));
    for scale in [1.0, 2.0] {
        assert_eq!(outline.content_insets(size(), scale), [0.0; 4]);
        assert_eq!(outline.paint_outsets(size(), scale), [2.0, 4.0, 6.0, 8.0]);
        assert!(outline.child_clip_radius(size(), scale).is_none());
        let commands = record(&outline, scale, Some(Color::RED));
        assert!(matches!(commands.as_slice(), [DrawCommand::FillRect {
            rect, color, border_width, outline_width, outline_color, ..
        }] if *rect == Rect::new(3.0, 5.0, 80.0, 40.0) && color.r == 255
            && outline_color.b == 255 && *outline_width == [4.0, 6.0, 8.0, 2.0]
            && *border_width == [0.0; 4]));
    }
    assert!(record(&BoxOutline::default(), 1.0, None).is_empty());
}

#[test]
fn box_shadow_decoration_preserves_offsets_blur_spread_inset_and_side() {
    let shadow = BoxShadow::new().offset_x(4.0).offset_y(-6.0).blur(3.0).spread(2.0)
        .color(Color::BLUE).side(ShadowSide::BottomRight);
    for scale in [1.0, 2.0] {
        assert_eq!(shadow.paint_outsets(size(), scale),
            [7.0 / scale, 17.0 / scale, 15.0 / scale, 5.0 / scale]);
        assert_eq!(shadow.content_insets(size(), scale), [0.0; 4]);
        assert!(shadow.child_clip_radius(size(), scale).is_none());
        for inset in [false, true] {
            let shadow = shadow.inset(inset);
            let commands = record(&shadow, scale, None);
            assert!(matches!(commands.as_slice(), [DrawCommand::DrawShadowRect {
                rect, shadow_color, shadow_params, border_radius, inset: actual_inset, side_params,
            }] if *rect == Rect::new(3.0, 5.0, 80.0, 40.0) && shadow_color.b == 255
                && *shadow_params == [4.0 / scale, -6.0 / scale, 3.0 / scale, 2.0 / scale]
                && *border_radius == [0.0; 4] && *actual_inset == inset
                && *side_params == [10.0, 0.0, 0.0]));
            if inset {
                assert_eq!(shadow.paint_outsets(size(), scale), [0.0; 4]);
            }
        }
    }
    assert!(record(&BoxShadow::default(), 1.0, None).is_empty());
    assert!(record(&shadow.color(Color::Transparent), 1.0, None).is_empty());
    let negative = BoxShadow::new().blur(-3.0).spread(-2.0);
    assert_eq!(negative.paint_outsets(size(), 1.0), [0.0; 4]);
    assert!(matches!(record(&shadow, 1.0, Some(Color::RED)).as_slice(),
        [DrawCommand::DrawShadowRect { inset: false, .. }, DrawCommand::FillRect { .. }]));
    assert!(matches!(record(&shadow.inset(true), 1.0, Some(Color::RED)).as_slice(),
        [DrawCommand::FillRect { .. }, DrawCommand::DrawShadowRect { inset: true, .. }]));
}

#[test]
fn border_radius_decoration_clips_and_shapes_fallback_without_painting_a_color() {
    let radius = BorderRadius::new().top_left(Dimension::Percent(10.0)).top_right(8.0)
        .bottom_right(12.0).bottom_left(16.0);
    for scale in [1.0, 2.0] {
        assert_eq!(radius.child_clip_radius(size(), scale), Some([8.0, 8.0, 12.0, 16.0]));
        assert_eq!(radius.content_insets(size(), scale), [0.0; 4]);
        assert_eq!(radius.paint_outsets(size(), scale), [0.0; 4]);
        assert!(radius.background_color().is_none());
        assert!(record(&radius, scale, None).is_empty());
        let commands = record(&radius, scale, Some(Color::RED));
        assert!(matches!(commands.as_slice(), [DrawCommand::FillRect {
            rect, color, border_radius, ..
        }] if *rect == Rect::new(3.0, 5.0, 80.0, 40.0) && color.r == 255
            && *border_radius == [8.0, 8.0, 12.0, 16.0]));
    }
    assert_eq!(BorderRadius::default().child_clip_radius(size(), 1.0), Some([0.0; 4]));
    assert!(!BorderRadius::default().is_empty(), "a zero radius still declares a rectangular clip");
    assert_eq!(BorderRadius::from(-2.0).child_clip_radius(size(), 1.0), Some([0.0; 4]));
}

#[test]
fn component_decorations_ignore_invalid_device_scales() {
    let decorations: [&dyn Decoration; 4] = [
        &BoxBorder::all(slice(2)), &BoxOutline::all(slice(2)),
        &BoxShadow::new().blur(2.0), &BorderRadius::from(4),
    ];
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        for decoration in decorations {
            assert!(record(decoration, scale, None).is_empty());
            assert_eq!(decoration.paint_outsets(size(), scale), [0.0; 4]);
        }
    }
}
