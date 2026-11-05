use std::sync::Arc;

use super::*;
use crate::draw_cmd::{FADE_MASK_PIPELINE_NAME, Vec2d};
use crate::font::{FontFamily, FontStyle};
use crate::pipeline::material::{GradientShape, MaterialGradient};
use crate::pipeline::text_pipeline::{TextOverflowMode, TextShadowRequest};
use crate::text_layout::TextHorizontalAlign;
use crate::utilities::{Color, Rgba8};

fn text(text: &str) -> DrawCommand {
    DrawCommand::DrawText {
        position: Vec2d { x: 0.0, y: 0.0 },
        text: Arc::from(text),
        font_size: 14.0,
        color: Color::rgba8(255, 255, 255, 255),
        bounds_width: None,
        bounds_height: None,
        overflow: TextOverflowMode::default(),
        horizontal_align: TextHorizontalAlign::default(),
        font_family: FontFamily::default(),
        font_style: FontStyle::default(),
        font_weight: 400,
        shadow: None,
        draw_glyphs: true,
    }
}

fn shadowed(text_value: &str) -> DrawCommand {
    let DrawCommand::DrawText {
        position,
        text,
        font_size,
        color,
        bounds_width,
        bounds_height,
        overflow,
        horizontal_align,
        font_family,
        font_style,
        font_weight,
        draw_glyphs,
        ..
    } = text(text_value)
    else {
        unreachable!("text builds text")
    };
    DrawCommand::DrawText {
        position,
        text,
        font_size,
        color,
        bounds_width,
        bounds_height,
        overflow,
        horizontal_align,
        font_family,
        font_style,
        font_weight,
        shadow: Some(TextShadowRequest {
            offset_x: 1.0,
            offset_y: 1.0,
            blur: 0.0,
            color: Rgba8::new(0, 0, 0, 255),
        }),
        draw_glyphs,
    }
}

/// A request for a horizontal ramp over a `width` by `height` box with `stops`,
/// each an offset and an alpha.
fn ramp_request(width: f32, height: f32, stops: &[(f32, u8)]) -> MaterialRequest {
    let mut request = MaterialRequest::new(MaterialKind::Gradient, [0.0, 0.0, width, height]);
    request.opacity = 1.0;
    request.corner_radii = [0.0; 4];
    request.gradient = MaterialGradient {
        shape: GradientShape::Linear,
        geometry: [0.0, 0.0, width, 0.0],
        stop_count: stops.len() as u8,
        ..MaterialGradient::default()
    };
    for (index, (offset, alpha)) in stops.iter().enumerate() {
        request.gradient.colors[index] = MaterialGradient::pack_color([255, 255, 255, *alpha]);
        request.gradient.offsets[index] = *offset;
    }
    request
}

fn custom(name: &str, request: MaterialRequest) -> DrawCommand {
    DrawCommand::Custom {
        pipeline_name: name.to_string(),
        data: Box::new(request.encode_for_test()),
    }
}

fn marker() -> DrawCommand {
    DrawCommand::Custom {
        pipeline_name: FADE_MASK_PIPELINE_NAME.to_string(),
        data: Box::new(Vec::<u8>::new()),
    }
}

fn title_ramp() -> MaterialRequest {
    ramp_request(200.0, 20.0, &[(0.0, 255), (0.74, 255), (1.0, 0)])
}

// ── What text may be faded as a ramp ──────────────────────────────────────

#[test]
fn text_in_scripts_with_outline_glyphs_may_be_ramped() {
    for plain in ["Hello", "héllo wörld", "Привет", "日本語のタイトル", "한국어", "Ａｂｃ"] {
        assert!(!text_may_use_color_glyphs(plain), "{plain}");
    }
}

#[test]
fn the_dashes_and_quotes_of_a_title_do_not_count_as_emoji() {
    assert!(!text_may_use_color_glyphs("It’s a “title” — with… bullets •"));
}

#[test]
fn text_that_could_be_an_emoji_is_not_ramped() {
    for emoji in ["😀", "ship it 🚀", "★", "✅ done", "👨‍👩‍👧", "❤️"] {
        assert!(text_may_use_color_glyphs(emoji), "{emoji}");
    }
}

// ── What content a ramp may stand in for ──────────────────────────────────

#[test]
fn plain_text_is_content_a_ramp_can_draw() {
    let scan = scan_content(&[
        DrawCommand::PushTransform {
            matrix: crate::utilities::Mat3::identity(),
        },
        text("a title"),
        DrawCommand::PopTransform,
    ]);
    assert_eq!(
        scan,
        ContentScan {
            drawable: true,
            texts: 1
        }
    );
}

#[test]
fn text_with_a_shadow_is_not_content_a_ramp_can_draw() {
    assert!(!scan_content(&[shadowed("a title")]).drawable);
}

#[test]
fn text_that_could_be_an_emoji_is_not_content_a_ramp_can_draw() {
    assert!(!scan_content(&[text("a title 🚀")]).drawable);
}

#[test]
fn anything_that_is_not_text_is_not_content_a_ramp_can_draw() {
    let rectangle = DrawCommand::ClearRect {
        rect: crate::draw_cmd::Rect::new(0.0, 0.0, 4.0, 4.0),
    };
    assert!(!scan_content(&[text("title"), rectangle]).drawable);
}

#[test]
fn an_item_that_draws_nothing_is_drawable_but_has_no_text() {
    assert_eq!(
        scan_content(&[]),
        ContentScan {
            drawable: true,
            texts: 0
        }
    );
}

// ── What alpha item is a ramp ─────────────────────────────────────────────

#[test]
fn a_marker_and_a_linear_gradient_are_a_ramp() {
    let request = alpha_item_request(&[marker(), custom(MATERIAL_PIPELINE_NAME, title_ramp())]);
    assert!(request.is_some());
}

#[test]
fn a_gradient_with_more_stops_than_a_ramp_carries_is_not_one() {
    let stops: Vec<(f32, u8)> = (0..5).map(|index| (index as f32 / 4.0, 255)).collect();
    let request = ramp_request(200.0, 20.0, &stops);
    assert!(alpha_item_request(&[marker(), custom(MATERIAL_PIPELINE_NAME, request)]).is_none());
}

#[test]
fn a_radial_gradient_is_not_a_ramp() {
    let mut request = title_ramp();
    request.gradient.shape = GradientShape::Radial;
    assert!(alpha_item_request(&[marker(), custom(MATERIAL_PIPELINE_NAME, request)]).is_none());
}

#[test]
fn a_gradient_with_rounded_corners_is_not_a_ramp() {
    let mut request = title_ramp();
    request.corner_radii = [4.0; 4];
    assert!(alpha_item_request(&[marker(), custom(MATERIAL_PIPELINE_NAME, request)]).is_none());
}

#[test]
fn an_item_with_two_gradients_or_anything_else_is_not_a_ramp() {
    let two = [
        marker(),
        custom(MATERIAL_PIPELINE_NAME, title_ramp()),
        custom(MATERIAL_PIPELINE_NAME, title_ramp()),
    ];
    assert!(alpha_item_request(&two).is_none(), "two gradients");

    let with_text = [marker(), custom(MATERIAL_PIPELINE_NAME, title_ramp()), text("x")];
    assert!(alpha_item_request(&with_text).is_none(), "a gradient and text");

    assert!(alpha_item_request(&[marker()]).is_none(), "a marker alone");
}

// ── The ramp a resolved request makes ─────────────────────────────────────

fn resolved(bounds: [f32; 4], opacity: f32) -> MaterialRequest {
    let mut request = title_ramp();
    request.bounds = bounds;
    request.gradient.geometry = [0.0, 0.0, bounds[2], 0.0];
    request.opacity = opacity;
    request
}

#[test]
fn the_ramp_runs_where_the_request_is_drawn() {
    // A 200 by 20 box at (30, 50) in the target: the axis starts at its corner.
    let ramp = ramp_from_resolved(&resolved([30.0, 50.0, 200.0, 20.0], 1.0)).unwrap();
    assert_eq!(ramp.line, [30.0, 50.0, 230.0, 50.0]);
    assert_eq!(ramp.rect, [30.0, 50.0, 200.0, 20.0]);
    assert!((ramp.alpha_at(100.0, 60.0) - 1.0).abs() < 1e-3, "opaque before the fade");
    assert!((ramp.alpha_at(204.0, 60.0) - 0.5).abs() < 0.01, "halfway through the fade");
    assert_eq!(ramp.alpha_at(20.0, 60.0), 0.0, "nothing outside the box");
}

#[test]
fn the_opacity_above_the_request_scales_the_ramp() {
    let ramp = ramp_from_resolved(&resolved([0.0, 0.0, 200.0, 20.0], 0.5)).unwrap();
    assert!((ramp.alpha_at(50.0, 10.0) - 0.5).abs() < 1e-3);
}

#[test]
fn a_request_that_is_not_a_plain_ramp_makes_none() {
    let mut request = resolved([0.0, 0.0, 200.0, 20.0], 1.0);
    request.gradient.shape = GradientShape::Conic;
    assert!(ramp_from_resolved(&request).is_none());
}

// ── Items as the lowering frames them ─────────────────────────────────────

/// What `lower_retained_v2_commands` puts around an item's own commands.
fn framed(inner: Vec<DrawCommand>) -> Vec<DrawCommand> {
    let mut commands = vec![
        DrawCommand::PushTransform {
            matrix: crate::utilities::Mat3::identity(),
        },
        DrawCommand::SetTransform {
            matrix: crate::utilities::Mat3::identity(),
        },
    ];
    commands.extend(inner);
    commands.push(DrawCommand::SetItalic { italic: false });
    commands.push(DrawCommand::SetTextLanguage { language: None });
    commands.push(DrawCommand::PopTransform);
    commands
}

#[test]
fn a_fades_own_item_is_a_ramp_in_the_frame_the_lowering_gives_it() {
    let commands = framed(vec![marker(), custom(MATERIAL_PIPELINE_NAME, title_ramp())]);
    assert!(alpha_item_request(&commands).is_some());
}

#[test]
fn text_is_content_a_ramp_can_draw_in_the_frame_the_lowering_gives_it() {
    let scan = scan_content(&framed(vec![text("a title")]));
    assert!(scan.drawable);
    assert_eq!(scan.texts, 1);
}

#[test]
fn the_clip_of_the_scene_around_a_fades_own_item_does_not_stop_it_being_a_ramp() {
    // The item of a thread title in a scroll view is framed by the viewport's clip.
    let clip = DrawCommand::PushClip {
        rect: crate::draw_cmd::Rect::new(0.0, 0.0, 10.0, 10.0),
        border_radius: [0.0; 4],
    };
    let commands = framed(vec![
        clip,
        marker(),
        custom(MATERIAL_PIPELINE_NAME, title_ramp()),
        DrawCommand::PopClip,
    ]);
    assert!(alpha_item_request(&commands).is_some());
}

#[test]
fn a_payload_with_a_clip_of_its_own_is_left_to_the_offscreen_path() {
    let mut request = title_ramp();
    request.clip = Some(crate::pipeline::material::MaterialClip {
        rect: [0.0, 0.0, 10.0, 10.0],
        corner_radii: [4.0; 4],
    });
    let commands = framed(vec![marker(), custom(MATERIAL_PIPELINE_NAME, request)]);
    assert!(alpha_item_request(&commands).is_none());
}

#[test]
fn the_clip_the_renderer_adds_when_it_resolves_a_request_does_not_refuse_the_ramp() {
    let mut request = resolved([30.0, 50.0, 200.0, 20.0], 1.0);
    request.clip = Some(crate::pipeline::material::MaterialClip {
        rect: [0.0, 0.0, 100.0, 100.0],
        corner_radii: [0.0; 4],
    });
    assert!(ramp_from_resolved(&request).is_some());
}
