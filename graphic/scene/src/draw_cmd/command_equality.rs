//! Structural equality for recorded [`DrawCommand`] lists.
//!
//! A rebuilt element records its commands again whether or not its picture
//! changed. Comparing the new recording with the committed one lets an
//! identical repaint keep its revision, so nothing downstream is damaged.
//!
//! The comparison is deliberately conservative: a `false` only costs a repaint,
//! while a wrong `true` would leave stale pixels on screen. Payloads that are
//! expensive or impossible to compare structurally (SVG scenes, retained image
//! pixels) are equal only when they are the same allocation.

use std::sync::Arc;

use super::{DrawCommand, RichTextSegment, Vec2d};

/// Returns whether two command lists record the same picture.
///
/// Floating-point fields are compared with `==`, so a `NaN` never equals
/// anything and the lists are reported as different.
pub(super) fn commands_equal(left: &[DrawCommand], right: &[DrawCommand]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| command_equal(a, b))
}

fn command_equal(left: &DrawCommand, right: &DrawCommand) -> bool {
    use DrawCommand::*;
    match (left, right) {
        (
            FillRect {
                rect,
                color,
                border_radius,
                border_width,
                border_color,
                outline_width,
                outline_color,
            },
            FillRect {
                rect: r_rect,
                color: r_color,
                border_radius: r_border_radius,
                border_width: r_border_width,
                border_color: r_border_color,
                outline_width: r_outline_width,
                outline_color: r_outline_color,
            },
        ) => {
            rect == r_rect
                && color == r_color
                && border_radius == r_border_radius
                && border_width == r_border_width
                && border_color == r_border_color
                && outline_width == r_outline_width
                && outline_color == r_outline_color
        }
        (
            DrawText {
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
                shadow,
                draw_glyphs,
            },
            DrawText {
                position: r_position,
                text: r_text,
                font_size: r_font_size,
                color: r_color,
                bounds_width: r_bounds_width,
                bounds_height: r_bounds_height,
                overflow: r_overflow,
                horizontal_align: r_horizontal_align,
                font_family: r_font_family,
                font_style: r_font_style,
                font_weight: r_font_weight,
                shadow: r_shadow,
                draw_glyphs: r_draw_glyphs,
            },
        ) => {
            // Compare the cheap scalar fields first so a changed run exits
            // before its string is scanned.
            font_size == r_font_size
                && font_weight == r_font_weight
                && draw_glyphs == r_draw_glyphs
                && vec2_equal(*position, *r_position)
                && color == r_color
                && bounds_width == r_bounds_width
                && bounds_height == r_bounds_height
                && overflow == r_overflow
                && horizontal_align == r_horizontal_align
                && font_family == r_font_family
                && font_style == r_font_style
                && shadow == r_shadow
                && (Arc::ptr_eq(text, r_text) || text == r_text)
        }
        (
            DrawRichText {
                position,
                spans,
                font_size,
                color,
                bounds_width,
                bounds_height,
                overflow,
            },
            DrawRichText {
                position: r_position,
                spans: r_spans,
                font_size: r_font_size,
                color: r_color,
                bounds_width: r_bounds_width,
                bounds_height: r_bounds_height,
                overflow: r_overflow,
            },
        ) => {
            font_size == r_font_size
                && vec2_equal(*position, *r_position)
                && color == r_color
                && bounds_width == r_bounds_width
                && bounds_height == r_bounds_height
                && overflow == r_overflow
                && spans_equal(spans, r_spans)
        }
        (
            DrawTextDecoration {
                rect,
                color,
                style,
                thickness,
                period,
            },
            DrawTextDecoration {
                rect: r_rect,
                color: r_color,
                style: r_style,
                thickness: r_thickness,
                period: r_period,
            },
        ) => {
            rect == r_rect
                && color == r_color
                && style == r_style
                && thickness == r_thickness
                && period == r_period
        }
        (
            DrawImage { rect, texture_id },
            DrawImage {
                rect: r_rect,
                texture_id: r_texture_id,
            },
        ) => rect == r_rect && texture_id == r_texture_id,
        (
            DrawImageWithResource { rect, resource },
            DrawImageWithResource {
                rect: r_rect,
                resource: r_resource,
            },
        ) => rect == r_rect && Arc::ptr_eq(resource, r_resource),
        (
            Svg {
                scene,
                destination,
                overrides,
            },
            Svg {
                scene: r_scene,
                destination: r_destination,
                overrides: r_overrides,
            },
        ) => {
            destination == r_destination
                && Arc::ptr_eq(scene, r_scene)
                && (Arc::ptr_eq(overrides, r_overrides) || overrides == r_overrides)
        }
        (
            DrawCustom {
                pipeline_name,
                data,
            },
            DrawCustom {
                pipeline_name: r_pipeline_name,
                data: r_data,
            },
        ) => {
            (Arc::ptr_eq(pipeline_name, r_pipeline_name) || pipeline_name == r_pipeline_name)
                && (Arc::ptr_eq(data, r_data) || data == r_data)
        }
        (
            DrawShadowRect {
                rect,
                shadow_color,
                shadow_params,
                border_radius,
                inset,
                side_params,
            },
            DrawShadowRect {
                rect: r_rect,
                shadow_color: r_shadow_color,
                shadow_params: r_shadow_params,
                border_radius: r_border_radius,
                inset: r_inset,
                side_params: r_side_params,
            },
        ) => {
            rect == r_rect
                && shadow_color == r_shadow_color
                && shadow_params == r_shadow_params
                && border_radius == r_border_radius
                && inset == r_inset
                && side_params == r_side_params
        }
        (
            PushClip {
                rect,
                border_radius,
            },
            PushClip {
                rect: r_rect,
                border_radius: r_border_radius,
            },
        ) => rect == r_rect && border_radius == r_border_radius,
        (PopClip, PopClip) | (PopTransform, PopTransform) | (RestoreAlpha, RestoreAlpha) => true,
        (PushTransform { matrix }, PushTransform { matrix: r_matrix })
        | (SetTransform { matrix }, SetTransform { matrix: r_matrix }) => matrix == r_matrix,
        (SetAlpha { alpha }, SetAlpha { alpha: r_alpha }) => alpha == r_alpha,
        (SetItalic { italic }, SetItalic { italic: r_italic }) => italic == r_italic,
        (SetTextLanguage { language }, SetTextLanguage { language: r_language }) => {
            language == r_language
        }
        _ => false,
    }
}

#[inline]
fn vec2_equal(left: Vec2d, right: Vec2d) -> bool {
    left.x == right.x && left.y == right.y
}

fn spans_equal(left: &[RichTextSegment], right: &[RichTextSegment]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a.font_size == b.font_size
                && a.color == b.color
                && a.font_weight == b.font_weight
                && a.italic == b.italic
                && (Arc::ptr_eq(&a.text, &b.text) || a.text == b.text)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utilities::{Color, Rect};

    fn fill(width: f32) -> DrawCommand {
        DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, width, 10.0),
            color: Color::new(1.0, 0.0, 0.0, 1.0),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::new(0.0, 0.0, 0.0, 0.0),
            outline_width: [0.0; 4],
            outline_color: Color::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn identical_lists_are_equal() {
        let left = [fill(10.0), DrawCommand::PopClip];
        let right = [fill(10.0), DrawCommand::PopClip];
        assert!(commands_equal(&left, &right));
    }

    #[test]
    fn changed_field_differs() {
        assert!(!commands_equal(&[fill(10.0)], &[fill(11.0)]));
    }

    #[test]
    fn length_and_variant_mismatches_differ() {
        assert!(!commands_equal(&[fill(10.0)], &[]));
        assert!(!commands_equal(&[DrawCommand::PopClip], &[DrawCommand::PopTransform]));
    }

    #[test]
    fn nan_never_compares_equal() {
        let nan = DrawCommand::SetAlpha { alpha: f32::NAN };
        assert!(!commands_equal(&[nan.clone()], &[nan]));
    }
}
