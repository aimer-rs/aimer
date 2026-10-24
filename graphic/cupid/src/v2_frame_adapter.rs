//! Bridge the retained v2 render tree into the direct render plan a frame
//! packet carries.

use crate::damage_region::{DamageRect, DamageSet};
use crate::draw_cmd::{DrawCommand as LegacyDrawCommand, DrawList as LegacyDrawList};
use crate::draw_cmd_v2::{DrawCommand as V2DrawCommand, Rect, RenderFrame, RenderItem, RenderOp};
use crate::frame::{Frame, FramePacket, FrameRenderMetadata, RetainedRenderPlan, RetainedV2Item};
use crate::utilities::Mat3;

/// Failures while building a retained render plan from a v2 render frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum V2FrameAdapterError {
    /// Device scale must be finite and greater than zero.
    InvalidDeviceScale,
    /// A logical damage rectangle had invalid coordinates or extents.
    InvalidDamage { index: usize },
    /// A render item carried invalid bounds, origin, or clip geometry.
    InvalidRenderItemGeometry { element: u64 },
    /// A full repaint was requested without a full-target v2 damage rectangle.
    FullRepaintRequiresCompleteV2Frame,
    /// A node's opacity is outside the normalized range.
    InvalidOpacity { element: u64 },
    /// Opacity group begin/end operations are unbalanced or mismatched.
    UnbalancedOpacityGroup { element: u64 },
}

impl FramePacket {
    /// Returns the whole-target logical rectangle when `damage`, mapped to
    /// device pixels, would be repainted in full anyway.
    ///
    /// [`DamageSet`] promotes a set to a full repaint when its regions cover
    /// half the target or are too many. A full repaint needs every visible
    /// node, so a caller that culls the render order by `damage` must widen it
    /// to this rectangle first; otherwise the packet is rejected with
    /// [`V2FrameAdapterError::FullRepaintRequiresCompleteV2Frame`]. Returns
    /// `None` for a partial set, an invalid rectangle (the adapter reports it)
    /// or an unusable scale or target.
    #[doc(hidden)]
    pub fn full_target_if_damage_promotes(
        damage: &[Rect],
        scale: f32,
        target_width: u32,
        target_height: u32,
    ) -> Option<Rect> {
        if !scale.is_finite() || scale <= 0.0 || target_width == 0 || target_height == 0 {
            return None;
        }
        let mut set = DamageSet::new(target_width, target_height);
        for (index, rect) in damage.iter().copied().enumerate() {
            if let Ok(Some(rect)) =
                logical_damage_to_device(rect, scale, target_width, target_height, index)
            {
                set.add(rect);
            }
        }
        set.is_full().then(|| {
            Rect::new(
                0.0,
                0.0,
                target_width as f32 / scale,
                target_height as f32 / scale,
            )
        })
    }
}

impl FramePacket {
    /// Builds a complete retained v2 packet whose local lists stay separate:
    /// nothing is lowered into a frame-wide command buffer. Opacity groups and
    /// leaf opacity are preserved for offscreen rendering by the GPU
    /// compositor.
    #[doc(hidden)]
    pub fn from_v2_direct(
        render_frame: RenderFrame,
        metadata: FrameRenderMetadata,
    ) -> Result<Self, V2FrameAdapterError> {
        let (width, height) = metadata.damage().target_size();
        let mut frame = Frame::new(LegacyDrawList::new(), width, height);
        let (plan, damage) =
            Self::prepare_v2_direct_render_plan(&render_frame, &metadata, &mut frame)?;
        if width != 0 && height != 0 && !damage.is_full() {
            return Err(V2FrameAdapterError::FullRepaintRequiresCompleteV2Frame);
        }
        Ok(FramePacket::with_render_plan(
            frame,
            metadata.with_damage(damage),
            None,
            Some(plan),
        ))
    }

    /// [`Self::from_v2_direct`] over a frame the caller already owns.
    ///
    /// The frame's draw list keeps the texture references the plan's commands
    /// use alive; `render_frame` must contain the complete visible tree so the
    /// renderer can recover a newly allocated or invalid persistent target from
    /// any packet.
    #[doc(hidden)]
    pub fn from_v2_direct_with_frame(
        render_frame: RenderFrame,
        metadata: FrameRenderMetadata,
        frame: Frame,
    ) -> Result<Self, V2FrameAdapterError> {
        let mut frame = frame;
        let (plan, damage) =
            Self::prepare_v2_direct_render_plan(&render_frame, &metadata, &mut frame)?;
        Ok(FramePacket::with_render_plan(
            frame,
            metadata.with_damage(damage),
            None,
            Some(plan),
        ))
    }

    /// Validates this frame's damage-culled retained operations and snapshots
    /// them into a render plan.
    ///
    /// `frame` owns the texture references the plan's commands use. Balanced
    /// opacity groups and leaf opacity are represented as ordered retained
    /// operations; the renderer isolates each group in a GPU target.
    #[doc(hidden)]
    pub fn prepare_v2_direct_render_plan(
        render_frame: &RenderFrame,
        metadata: &FrameRenderMetadata,
        frame: &mut Frame,
    ) -> Result<(RetainedRenderPlan, DamageSet), V2FrameAdapterError> {
        let damage = validate_frame_input(render_frame, metadata)?;
        let scale = metadata.device_scale();
        let (width, height) = metadata.damage().target_size();
        let mut operations = Vec::with_capacity(render_frame.operations.len());
        let mut direct_texture_ids = Vec::new();
        let mut opacity_groups = Vec::new();

        for (operation_index, operation) in render_frame.operations.iter().enumerate() {
            match operation {
                RenderOp::Draw(item) => {
                    let Some(bounds) = item_device_bounds(item, scale, width, height) else {
                        continue;
                    };
                    let snapshot = item.snapshot();
                    if snapshot.commands.is_empty() {
                        continue;
                    }
                    if item.opacity < 1.0 {
                        operations.push(RetainedRenderPlan::opacity_group_begin_operation(
                            item.element.get(),
                            item.opacity,
                            false,
                            false,
                            None,
                            bounds,
                        ));
                    }
                    direct_texture_ids.extend_from_slice(&snapshot.image_textures);
                    operations.push(RetainedRenderPlan::local_v2_operation(
                        RetainedV2Item {
                            element: item.element,
                            revision: snapshot.revision,
                            bounds: item.bounds,
                            origin: item.origin,
                            transform: item.transform,
                            clip: item.clip,
                            clip_radius: item.clip_radius,
                            commands: snapshot.commands,
                        },
                        bounds,
                    ));
                    if item.opacity < 1.0 {
                        operations.push(RetainedRenderPlan::opacity_group_end_operation(
                            item.element.get(),
                            bounds,
                        ));
                    }
                }
                RenderOp::BeginOpacityGroup {
                    element,
                    bounds,
                    opacity,
                    masked,
                    fade,
                    morph,
                    ..
                } => {
                    let device_bounds =
                        logical_damage_to_device(*bounds, scale, width, height, operation_index)?
                            .unwrap_or(DamageRect::new(0, 0, 0, 0));
                    opacity_groups.push((element.get(), device_bounds));
                    operations.push(RetainedRenderPlan::opacity_group_begin_operation(
                        element.get(),
                        *opacity,
                        *masked,
                        *fade,
                        *morph,
                        device_bounds,
                    ));
                }
                RenderOp::EndOpacityGroup { element } => {
                    let Some((group, bounds)) = opacity_groups.pop() else {
                        return Err(V2FrameAdapterError::UnbalancedOpacityGroup {
                            element: element.get(),
                        });
                    };
                    if group != element.get() {
                        return Err(V2FrameAdapterError::UnbalancedOpacityGroup {
                            element: element.get(),
                        });
                    }
                    operations.push(RetainedRenderPlan::opacity_group_end_operation(
                        element.get(),
                        bounds,
                    ));
                }
            }
        }

        if let Some((element, _)) = opacity_groups.pop() {
            return Err(V2FrameAdapterError::UnbalancedOpacityGroup { element });
        }

        frame
            .draw_list
            .sync_composed_texture_references_with_ids(direct_texture_ids);
        let complete = damage.is_full();
        Ok((RetainedRenderPlan::new(operations, complete), damage))
    }
}

fn item_device_bounds(
    item: &RenderItem,
    scale: f32,
    target_width: u32,
    target_height: u32,
) -> Option<DamageRect> {
    let mut bounds = item.bounds;
    if let Some(clip) = item.clip {
        let left = bounds.x.max(clip.x);
        let top = bounds.y.max(clip.y);
        let right = (bounds.x + bounds.width).min(clip.x + clip.width);
        let bottom = (bounds.y + bounds.height).min(clip.y + clip.height);
        bounds = Rect::new(left, top, (right - left).max(0.0), (bottom - top).max(0.0));
    }
    let scale = f64::from(scale);
    let left = (f64::from(bounds.x) * scale)
        .floor()
        .clamp(0.0, f64::from(target_width));
    let top = (f64::from(bounds.y) * scale)
        .floor()
        .clamp(0.0, f64::from(target_height));
    let right = ((f64::from(bounds.x) + f64::from(bounds.width)) * scale)
        .ceil()
        .clamp(0.0, f64::from(target_width));
    let bottom = ((f64::from(bounds.y) + f64::from(bounds.height)) * scale)
        .ceil()
        .clamp(0.0, f64::from(target_height));
    if right <= left || bottom <= top {
        return None;
    }
    let x = left as u32;
    let y = top as u32;
    Some(DamageRect::new(x, y, right as u32 - x, bottom as u32 - y))
}

/// Checks `render_frame` and maps its logical damage to device pixels.
fn validate_frame_input(
    render_frame: &RenderFrame,
    metadata: &FrameRenderMetadata,
) -> Result<DamageSet, V2FrameAdapterError> {
    let scale = metadata.device_scale();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(V2FrameAdapterError::InvalidDeviceScale);
    }

    let (width, height) = metadata.damage().target_size();
    let mut damage = DamageSet::new(width, height);
    let full_target = DamageRect::new(0, 0, width, height);
    let mut has_full_target_damage = false;
    for (index, rect) in render_frame.damage.iter().copied().enumerate() {
        if let Some(rect) = logical_damage_to_device(rect, scale, width, height, index)? {
            has_full_target_damage |= rect == full_target;
            damage.add(rect);
        }
    }
    if (metadata.damage().is_full() || damage.is_full())
        && width != 0
        && height != 0
        && !has_full_target_damage
    {
        return Err(V2FrameAdapterError::FullRepaintRequiresCompleteV2Frame);
    }

    let mut opacity_stack = Vec::new();
    for operation in &render_frame.operations {
        match operation {
            RenderOp::BeginOpacityGroup {
                element,
                bounds,
                opacity,
                clip,
                ..
            } => {
                if !valid_rect(*bounds) || clip.is_some_and(|clip| !valid_rect(clip)) {
                    return Err(V2FrameAdapterError::InvalidRenderItemGeometry {
                        element: element.get(),
                    });
                }
                if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                    return Err(V2FrameAdapterError::InvalidOpacity {
                        element: element.get(),
                    });
                }
                opacity_stack.push(element.get());
            }
            RenderOp::EndOpacityGroup { element } => {
                if opacity_stack.pop() != Some(element.get()) {
                    return Err(V2FrameAdapterError::UnbalancedOpacityGroup {
                        element: element.get(),
                    });
                }
            }
            RenderOp::Draw(item) => validate_item(item)?,
        }
    }
    if let Some(element) = opacity_stack.pop() {
        return Err(V2FrameAdapterError::UnbalancedOpacityGroup { element });
    }

    Ok(damage)
}

fn validate_item(item: &RenderItem) -> Result<(), V2FrameAdapterError> {
    let element = item.element.get();
    if !valid_rect(item.bounds)
        || !item.origin.0.is_finite()
        || !item.origin.1.is_finite()
        || !valid_transform(item.transform)
        || item.clip.is_some_and(|clip| !valid_rect(clip))
    {
        return Err(V2FrameAdapterError::InvalidRenderItemGeometry { element });
    }
    if !item.opacity.is_finite() || !(0.0..=1.0).contains(&item.opacity) {
        return Err(V2FrameAdapterError::InvalidOpacity { element });
    }
    if let Some(clip) = item.clip {
        let local_clip = Rect::new(
            clip.x - item.origin.0,
            clip.y - item.origin.1,
            clip.width,
            clip.height,
        );
        if !valid_rect(local_clip) {
            return Err(V2FrameAdapterError::InvalidRenderItemGeometry { element });
        }
    }
    Ok(())
}

fn logical_damage_to_device(
    rect: Rect,
    scale: f32,
    target_width: u32,
    target_height: u32,
    index: usize,
) -> Result<Option<DamageRect>, V2FrameAdapterError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width < 0.0
        || rect.height < 0.0
    {
        return Err(V2FrameAdapterError::InvalidDamage { index });
    }
    if rect.width == 0.0 || rect.height == 0.0 || target_width == 0 || target_height == 0 {
        return Ok(None);
    }

    let scale = f64::from(scale);
    let target_right = f64::from(target_width);
    let target_bottom = f64::from(target_height);
    let left = (f64::from(rect.x) * scale).floor().clamp(0.0, target_right);
    let top = (f64::from(rect.y) * scale)
        .floor()
        .clamp(0.0, target_bottom);
    let right = ((f64::from(rect.x) + f64::from(rect.width)) * scale)
        .ceil()
        .clamp(0.0, target_right);
    let bottom = ((f64::from(rect.y) + f64::from(rect.height)) * scale)
        .ceil()
        .clamp(0.0, target_bottom);
    if right <= left || bottom <= top {
        return Ok(None);
    }

    let x = left as u32;
    let y = top as u32;
    Ok(Some(DamageRect::new(
        x,
        y,
        (right as u32) - x,
        (bottom as u32) - y,
    )))
}

fn valid_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

fn valid_transform(transform: Mat3) -> bool {
    transform
        .cols
        .iter()
        .flatten()
        .all(|value| value.is_finite())
}

/// Lowers one retained item's local list into the renderer's command
/// vocabulary, positioned by the item's origin, transform and clip.
pub(crate) fn lower_retained_v2_commands(
    item: &RetainedV2Item,
    scale: f32,
) -> Result<Vec<LegacyDrawCommand>, V2FrameAdapterError> {
    let element = item.element.get();
    if !scale.is_finite() || scale <= 0.0 {
        return Err(V2FrameAdapterError::InvalidDeviceScale);
    }
    if !valid_rect(item.bounds)
        || !item.origin.0.is_finite()
        || !item.origin.1.is_finite()
        || !item
            .transform
            .cols
            .iter()
            .flatten()
            .all(|value| value.is_finite())
        || item.clip.is_some_and(|clip| !valid_rect(clip))
    {
        return Err(V2FrameAdapterError::InvalidRenderItemGeometry { element });
    }
    if item.commands.is_empty() {
        return Ok(Vec::new());
    }
    let origin_transform = Mat3::scale(scale, scale)
        .mul(&item.transform)
        .mul(&Mat3::translate(item.origin.0, item.origin.1));
    let text_scale = scale * node_text_scale(&item.transform);
    let mut output = LegacyDrawList::new();
    output.save();
    if let Some(clip) = item.clip {
        output.push(LegacyDrawCommand::SetTransform {
            matrix: Mat3::scale(scale, scale),
        });
        // Logical clip under the scale transform above, so the renderer
        // scales the logical radius together with the rectangle.
        output.push(LegacyDrawCommand::PushClip {
            rect: clip,
            border_radius: item.clip_radius,
        });
    }
    output.push(LegacyDrawCommand::SetTransform {
        matrix: origin_transform,
    });

    for command in item.commands.iter().cloned() {
        match command {
            V2DrawCommand::DrawImage { rect, texture_id } => {
                output.draw_image(rect, texture_id);
            }
            V2DrawCommand::DrawImageWithResource { rect, resource } => {
                output.draw_image_with_resource(rect, resource);
            }
            command => output.push(lower_command(command, origin_transform, text_scale)),
        }
    }

    output.push(LegacyDrawCommand::SetItalic { italic: false });
    output.push(LegacyDrawCommand::SetTextLanguage { language: None });
    if item.clip.is_some() {
        output.pop_clip();
    }
    output.restore();
    Ok(output.take_commands_for_composition())
}

/// The factor by which a node's transform enlarges glyphs.
///
/// The renderer maps a text request's position through the current transform
/// but takes its font size and layout bounds as given, so a scaled node has to
/// scale them before lowering. A uniform scale is exact; a non-uniform one
/// (a stretching transition) cannot be expressed by a glyph, so the geometric
/// mean keeps the text proportional to the area it occupies. A degenerate
/// transform (zero, negative or non-finite scale) leaves the text unscaled
/// rather than collapsing it. Returns exactly `1.0` for a pure translation, so
/// unscaled nodes lower to the same sizes as before.
#[inline]
fn node_text_scale(transform: &Mat3) -> f32 {
    let x = transform.cols[0][0].hypot(transform.cols[0][1]);
    let y = transform.cols[1][0].hypot(transform.cols[1][1]);
    let scale = (x * y).sqrt();
    if scale.is_finite() && scale > 0.0 && transform.cols[0][0] * transform.cols[1][1]
        - transform.cols[0][1] * transform.cols[1][0] > 0.0
    {
        scale
    } else {
        1.0
    }
}

/// Lowers one retained command. `scale` converts logical to device pixels for
/// text; the renderer applies `origin` to positions and geometry itself.
fn lower_command(command: V2DrawCommand, origin: Mat3, scale: f32) -> LegacyDrawCommand {
    match command {
        V2DrawCommand::FillRect {
            rect,
            color,
            border_radius,
            border_width,
            border_color,
            outline_width,
            outline_color,
        } => LegacyDrawCommand::FillRect {
            rect,
            color,
            border_radius,
            border_width,
            border_color,
            outline_width,
            outline_color,
        },
        V2DrawCommand::DrawText {
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
        } => LegacyDrawCommand::DrawText {
            position,
            text,
            font_size: font_size * scale,
            color,
            bounds_width: bounds_width.map(|width| width * scale),
            bounds_height: bounds_height.map(|height| height * scale),
            overflow,
            horizontal_align,
            font_family,
            font_style,
            font_weight,
            shadow,
            draw_glyphs,
        },
        V2DrawCommand::DrawRichText {
            position,
            mut spans,
            font_size,
            color,
            bounds_width,
            bounds_height,
            overflow,
        } => LegacyDrawCommand::DrawRichText {
            position,
            spans: {
                for span in &mut spans {
                    span.font_size = span.font_size.map(|size| size * scale);
                }
                spans
            },
            font_size: font_size * scale,
            color,
            bounds_width: bounds_width.map(|width| width * scale),
            bounds_height: bounds_height.map(|height| height * scale),
            overflow,
        },
        V2DrawCommand::DrawTextDecoration {
            rect,
            color,
            style,
            thickness,
            period,
        } => LegacyDrawCommand::DrawTextDecoration {
            rect,
            color,
            style,
            thickness,
            period,
        },
        V2DrawCommand::DrawImage { rect, texture_id } => {
            LegacyDrawCommand::DrawImage { rect, texture_id }
        }
        V2DrawCommand::DrawImageWithResource { rect, resource } => {
            LegacyDrawCommand::DrawImageWithResource { rect, resource }
        }
        V2DrawCommand::Svg {
            scene,
            destination,
            overrides,
        } => LegacyDrawCommand::Svg {
            scene,
            destination,
            overrides,
        },
        V2DrawCommand::DrawCustom {
            pipeline_name,
            data,
        } => LegacyDrawCommand::Custom {
            pipeline_name: pipeline_name.to_string(),
            data: Box::new(data.to_vec()),
        },
        V2DrawCommand::DrawShadowRect {
            rect,
            shadow_color,
            shadow_params,
            border_radius,
            inset,
            side_params,
        } => LegacyDrawCommand::DrawShadowRect {
            rect,
            shadow_color,
            shadow_params,
            border_radius,
            inset,
            side_params,
        },
        V2DrawCommand::PushClip {
            rect,
            border_radius,
        } => LegacyDrawCommand::PushClip {
            rect,
            border_radius,
        },
        V2DrawCommand::PopClip => LegacyDrawCommand::PopClip,
        V2DrawCommand::PushTransform { matrix } => LegacyDrawCommand::PushTransform {
            matrix: origin.mul(&matrix),
        },
        V2DrawCommand::PopTransform => LegacyDrawCommand::PopTransform,
        V2DrawCommand::SetAlpha { alpha } => LegacyDrawCommand::SetAlpha { alpha },
        V2DrawCommand::RestoreAlpha => LegacyDrawCommand::RestoreAlpha,
        V2DrawCommand::SetItalic { italic } => LegacyDrawCommand::SetItalic { italic },
        V2DrawCommand::SetTextLanguage { language } => {
            LegacyDrawCommand::SetTextLanguage { language }
        }
        V2DrawCommand::SetTransform { matrix } => LegacyDrawCommand::SetTransform {
            matrix: origin.mul(&matrix),
        },
    }
}

#[cfg(test)]
mod rounded_clip_tests;

#[cfg(test)]
mod text_scale_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::damage_region::{DamageRect, DamageSet};
    use crate::draw_cmd::{DrawCommand as LegacyDrawCommand, DrawList as LegacyDrawList};
    use crate::draw_cmd_v2::{
        DrawCommand, Rect, RenderFrame, RenderOp, RenderPaintSource, RenderTree,
    };
    use crate::frame::{
        Frame, FramePacket, FrameRenderMetadata, RetainedRenderOperationKind, RetainedV2Item,
    };
    use crate::utilities::{Color, Mat3};

    #[test]
    fn direct_packet_keeps_local_lists_in_tree_order_without_a_flat_command_buffer() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(16.0, 16.0, 32.0, 32.0))
            .unwrap();
        record_fill(&tree, root, Color::rgba8(220, 20, 20, 255));
        record_fill(&tree, child, Color::rgba8(20, 220, 20, 255));

        let packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(64, 64)),
            Frame::new(LegacyDrawList::new(), 64, 64),
        )
        .unwrap();

        assert!(packet.frame().draw_list.commands().is_empty());
        let plan = packet.render_plan().expect("direct retained render plan");
        assert!(plan.is_complete());
        assert_eq!(plan.local_v2_revision(root), Some(1));
        let operations = plan
            .operations_for_region(DamageRect::new(0, 0, 64, 64))
            .collect::<Vec<_>>();
        assert_eq!(operations.len(), 2);
        assert!(matches!(
            &operations[0].kind,
            RetainedRenderOperationKind::LocalV2(item) if item.element == root
        ));
        assert!(matches!(
            &operations[1].kind,
            RetainedRenderOperationKind::LocalV2(item) if item.element == child
        ));
    }

    #[test]
    fn lowers_local_paint_and_rounds_damage_outward() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(5.0, 6.0, 80.0, 70.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(10.0, 20.0, 30.0, 24.0))
            .unwrap();
        tree.set_clip(root, Some(Rect::new(0.0, 0.0, 70.0, 50.0)))
            .unwrap();
        let commands = vec![
            DrawCommand::SetTransform {
                matrix: crate::utilities::Mat3::translate(2.0, 3.0),
            },
            DrawCommand::FillRect {
                rect: Rect::new(1.0, 2.0, 3.0, 4.0),
                color: crate::utilities::Color::rgba8(220, 80, 40, 255),
                border_radius: [0.0; 4],
                border_width: [0.0; 4],
                border_color: crate::utilities::Color::transparent(),
                outline_width: [0.0; 4],
                outline_color: crate::utilities::Color::transparent(),
            },
            DrawCommand::DrawText {
                position: crate::utilities::Vec2d::new(0.0, 8.0),
                text: std::sync::Arc::<str>::from("scaled"),
                font_size: 8.0,
                color: crate::utilities::Color::white(),
                bounds_width: Some(10.0),
                bounds_height: Some(20.0),
                overflow: crate::text_pipeline::TextOverflowMode::Clip,
                horizontal_align: crate::text_pipeline::text_layout::TextHorizontalAlign::Left,
                font_family: crate::font::FontFamily::SANS_SERIF,
                font_style: crate::font::FontStyle::Normal,
                font_weight: 400,
                shadow: None,
                draw_glyphs: true,
            },
        ];
        tree.context(child)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(commands)
            .unwrap();

        let packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![Rect::new(2.25, 3.2, 4.25, 5.1)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.5, 7, 2, 3, 4, DamageSet::new(128, 128)),
            Frame::new(LegacyDrawList::new(), 128, 128),
        )
        .unwrap();

        assert_eq!((packet.frame().width, packet.frame().height), (128, 128));
        assert_eq!(
            packet.metadata().damage().regions(),
            &[DamageRect::new(3, 4, 7, 9)]
        );
        assert_eq!(packet.metadata().surface_identity(), 7);

        // The plan keeps each local list as recorded; lowering is what the
        // renderer does with an item, so check it here.
        let commands = packet
            .render_plan()
            .expect("direct retained render plan")
            .operations_for_region(DamageRect::new(0, 0, 128, 128))
            .filter_map(|operation| match &operation.kind {
                RetainedRenderOperationKind::LocalV2(item) => {
                    Some(super::lower_retained_v2_commands(item, 1.5).unwrap())
                }
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        let transform = commands
            .iter()
            .find_map(|command| match command {
                LegacyDrawCommand::SetTransform { matrix }
                    if matrix.transform_point(0.0, 0.0) == (22.5, 39.0) =>
                {
                    Some(matrix)
                }
                _ => None,
            })
            .expect("element origin transform");
        assert_eq!(transform.transform_point(0.0, 0.0), (22.5, 39.0));
        let local_transform = commands
            .iter()
            .filter_map(|command| match command {
                LegacyDrawCommand::SetTransform { matrix } => Some(matrix),
                _ => None,
            })
            .nth(2)
            .expect("composed local transform");
        assert_eq!(local_transform.transform_point(0.0, 0.0), (25.5, 43.5));
        assert!(commands.iter().any(|command| matches!(
            command,
            LegacyDrawCommand::PushClip { rect, .. }
                if rect.x == 5.0 && rect.y == 6.0 && rect.width == 70.0 && rect.height == 50.0
        )));
        assert!(commands.iter().any(|command| matches!(
            command,
            LegacyDrawCommand::FillRect { rect, .. }
                if rect.x == 1.0 && rect.y == 2.0 && rect.width == 3.0 && rect.height == 4.0
        )));
        let text = commands.iter().find_map(|command| match command {
            LegacyDrawCommand::DrawText {
                font_size,
                bounds_width,
                bounds_height,
                ..
            } => Some((*font_size, *bounds_width, *bounds_height)),
            _ => None,
        });
        assert_eq!(text, Some((12.0, Some(15.0), Some(30.0))));
    }

    #[test]
    fn retained_commands_apply_node_transform_and_final_clip() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        tree.set_paint_source(root, RenderPaintSource::LocalV2)
            .unwrap();
        tree.set_clip(root, Some(Rect::new(0.0, 0.0, 40.0, 40.0)))
            .unwrap();
        record_fill(&tree, root, Color::rgba8(20, 40, 60, 255));
        let node_transform = Mat3::translate(50.0, 50.0)
            .mul(&Mat3::scale(0.5, 0.5))
            .mul(&Mat3::translate(-50.0, -50.0));
        tree.set_transform(root, node_transform).unwrap();

        let item = tree
            .render_all()
            .into_iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) => Some(item),
                _ => None,
            })
            .expect("retained root item");
        let snapshot = item.snapshot();
        let retained = RetainedV2Item {
            element: item.element,
            revision: snapshot.revision,
            bounds: item.bounds,
            origin: item.origin,
            transform: item.transform,
            clip: item.clip,
            clip_radius: item.clip_radius,
            commands: snapshot.commands,
        };
        let commands = super::lower_retained_v2_commands(&retained, 2.0).unwrap();

        let expected_transform = Mat3::scale(2.0, 2.0).mul(&node_transform);
        assert!(commands.iter().any(|command| matches!(
            command,
            LegacyDrawCommand::SetTransform { matrix } if *matrix == expected_transform
        )));
        assert!(commands.iter().any(|command| matches!(
            command,
            LegacyDrawCommand::PushClip { rect, .. }
                if *rect == Rect::new(25.0, 25.0, 20.0, 20.0)
        )));
    }

    #[test]
    fn complete_packet_keeps_clean_nodes_for_target_recreation() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(16.0, 16.0, 32.0, 32.0))
            .unwrap();
        record_fill(&tree, root, Color::rgba8(220, 20, 20, 255));
        record_fill(&tree, child, Color::rgba8(20, 20, 220, 255));
        // The whole target is dirty, so the plan holds the complete tree.
        let packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 4, 1, 1, 1, DamageSet::new(64, 64)),
            Frame::new(LegacyDrawList::new(), 64, 64),
        )
        .unwrap();

        let plan = packet.render_plan().expect("v2 packet render plan");
        assert!(plan.is_complete());
        let left = plan
            .operations_for_region(DamageRect::new(2, 2, 4, 4))
            .collect::<Vec<_>>();
        let child_damage = plan
            .operations_for_region(DamageRect::new(24, 24, 4, 4))
            .collect::<Vec<_>>();
        assert_eq!(left.len(), 1);
        assert_eq!(child_damage.len(), 2);
        assert!(left.iter().all(|operation| matches!(
            operation.kind,
            crate::frame::RetainedRenderOperationKind::LocalV2(_)
        )));
    }

    #[test]
    fn direct_packet_preserves_opacity_group_boundaries() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        let first = tree
            .add_child(root, Rect::new(8.0, 8.0, 40.0, 40.0))
            .unwrap();
        let second = tree
            .add_child(root, Rect::new(24.0, 24.0, 32.0, 32.0))
            .unwrap();
        tree.set_opacity(root, 0.5).unwrap();
        record_fill(&tree, root, Color::rgba8(220, 20, 20, 255));
        record_fill(&tree, first, Color::rgba8(20, 220, 20, 255));
        record_fill(&tree, second, Color::rgba8(20, 20, 220, 255));

        let packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::full(64, 64),
        )
        .expect("the direct renderer accepts opacity groups");

        let plan = packet.render_plan().unwrap();
        assert!(plan.is_complete());
        assert_eq!(
            plan.operations_for_region(DamageRect::new(0, 0, 64, 64))
                .count(),
            5,
            "group begin/end enclose the three overlapping local lists"
        );
        assert_eq!(
            plan.operations_for_region(DamageRect::new(40, 40, 1, 1))
                .count(),
            5,
            "partial overlap damage replays the group and every intersecting item"
        );
        assert_eq!(
            plan.operations_for_region(DamageRect::new(60, 60, 1, 1))
                .count(),
            3,
            "group damage outside the children only replays the parent list"
        );
        let revisions = [root, first, second].map(|node| {
            packet
                .render_plan()
                .unwrap()
                .local_v2_revision(node)
                .unwrap()
        });
        let _ = tree.take_damage();
        tree.set_opacity(root, 0.25).unwrap();
        let update_damage = tree.take_damage();
        let updated_packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: update_damage,
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 8, 1, 1, 1, DamageSet::new(64, 64)),
            Frame::new(LegacyDrawList::new(), 64, 64),
        )
        .unwrap();
        for (node, revision) in [root, first, second].into_iter().zip(revisions) {
            assert_eq!(
                updated_packet
                    .render_plan()
                    .unwrap()
                    .local_v2_revision(node),
                Some(revision),
                "opacity updates do not rerecord local commands"
            );
        }
    }

    #[test]
    fn direct_packet_isolates_leaf_opacity_without_rerecording_its_list() {
        let tree = RenderTree::new();
        let leaf = tree.add_root(Rect::new(8.0, 8.0, 32.0, 32.0)).unwrap();
        tree.set_opacity(leaf, 0.25).unwrap();
        record_fill(&tree, leaf, Color::rgba8(20, 80, 220, 255));

        let packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::full(64, 64),
        )
        .expect("direct rendering isolates leaf opacity too");

        let plan = packet.render_plan().unwrap();
        let operations = plan
            .operations_for_region(DamageRect::new(0, 0, 64, 64))
            .collect::<Vec<_>>();
        assert_eq!(operations.len(), 3);
        assert!(matches!(
            &operations[0].kind,
            RetainedRenderOperationKind::OpacityGroupBegin { opacity, .. }
                if (*opacity - 0.25).abs() < f32::EPSILON
        ));
        assert_eq!(plan.local_v2_revision(leaf), Some(1));
        assert!(matches!(
            &operations[2].kind,
            RetainedRenderOperationKind::OpacityGroupEnd { .. }
        ));
    }

    #[test]
    fn direct_packet_rejects_malformed_opacity_groups() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(8.0, 8.0, 32.0, 32.0))
            .unwrap();
        tree.set_opacity(root, 0.5).unwrap();
        record_fill(&tree, root, Color::rgba8(220, 20, 20, 255));
        record_fill(&tree, child, Color::rgba8(20, 20, 220, 255));

        let mut unbalanced_operations = tree.render_all();
        unbalanced_operations.pop();
        let unbalanced = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: unbalanced_operations,
            },
            FrameRenderMetadata::full(64, 64),
        );
        assert!(matches!(
            unbalanced,
            Err(super::V2FrameAdapterError::UnbalancedOpacityGroup { .. })
        ));

        let mut invalid_operations = tree.render_all();
        let RenderOp::BeginOpacityGroup { opacity, .. } = &mut invalid_operations[0] else {
            panic!("tree starts with its opacity group");
        };
        *opacity = 1.5;
        let invalid = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: invalid_operations,
            },
            FrameRenderMetadata::full(64, 64),
        );
        assert!(matches!(
            invalid,
            Err(super::V2FrameAdapterError::InvalidOpacity { .. })
        ));
    }

    #[test]
    fn rejects_invalid_scale_and_damage() {
        let invalid_scale = FramePacket::from_v2_direct(
            RenderFrame {
                damage: Vec::new(),
                operations: Vec::new(),
            },
            FrameRenderMetadata::new(0.0, 0, 0, 0, 0, DamageSet::new(10, 10)),
        );
        assert!(matches!(
            invalid_scale,
            Err(super::V2FrameAdapterError::InvalidDeviceScale)
        ));

        let invalid_damage = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, -1.0, 2.0)],
                operations: Vec::new(),
            },
            FrameRenderMetadata::new(1.0, 0, 0, 0, 0, DamageSet::new(10, 10)),
        );
        assert!(matches!(
            invalid_damage,
            Err(super::V2FrameAdapterError::InvalidDamage { index: 0 })
        ));

        let partial_with_full_metadata = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 8.0, 8.0)],
                operations: Vec::new(),
            },
            FrameRenderMetadata::full(10, 10),
        );
        assert!(matches!(
            partial_with_full_metadata,
            Err(super::V2FrameAdapterError::FullRepaintRequiresCompleteV2Frame)
        ));

        let partial_promoted_to_full = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 8.0, 8.0)],
                operations: Vec::new(),
            },
            FrameRenderMetadata::new(1.0, 0, 0, 0, 0, DamageSet::new(10, 10)),
        );
        assert!(matches!(
            partial_promoted_to_full,
            Err(super::V2FrameAdapterError::FullRepaintRequiresCompleteV2Frame)
        ));
    }

    #[test]
    fn damage_that_the_device_set_promotes_to_full_is_widened_to_the_target() {
        // Two thirds of the target is partial for the caller but a full repaint
        // for the damage set, which the adapter then insists on drawing complete.
        let large = [Rect::new(0.0, 0.0, 10.0, 7.0)];
        let widened = FramePacket::full_target_if_damage_promotes(&large, 1.0, 10, 10)
            .expect("two thirds of the target promotes to a full repaint");
        assert_eq!(widened, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(
            FramePacket::from_v2_direct(
                RenderFrame {
                    damage: vec![widened],
                    operations: Vec::new()
                },
                FrameRenderMetadata::new(1.0, 0, 0, 0, 0, DamageSet::new(10, 10)),
            )
            .is_ok()
        );

        let small = [Rect::new(0.0, 0.0, 4.0, 4.0)];
        assert_eq!(
            FramePacket::full_target_if_damage_promotes(&small, 1.0, 10, 10),
            None
        );
        let scaled = FramePacket::full_target_if_damage_promotes(&large, 2.0, 20, 20)
            .expect("the same fraction promotes on a scaled target");
        assert_eq!(scaled, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(
            FramePacket::full_target_if_damage_promotes(&large, 0.0, 10, 10),
            None
        );
    }

    #[test]
    fn empty_or_off_target_damage_does_not_expand_the_packet_damage() {
        let packet = FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![
                    Rect::new(-5.0, -5.0, 0.0, 2.0),
                    Rect::new(20.0, 20.0, 2.0, 2.0),
                ],
                operations: Vec::new(),
            },
            FrameRenderMetadata::new(1.0, 0, 0, 0, 0, DamageSet::new(10, 10)),
            Frame::new(LegacyDrawList::new(), 10, 10),
        )
        .unwrap();

        assert!(packet.metadata().damage().is_empty());
    }

    #[test]
    fn retained_custom_commands_reach_the_existing_pipeline_payload_path() {
        let payload: Arc<[u8]> = Arc::from(vec![1_u8, 2, 3, 4]);
        let lowered = super::lower_command(
            DrawCommand::DrawCustom {
                pipeline_name: Arc::from("test.pipeline"),
                data: Arc::clone(&payload),
            },
            Mat3::identity(),
            1.0,
        );
        let LegacyDrawCommand::Custom {
            pipeline_name,
            data,
        } = lowered
        else {
            panic!("a retained custom request lowers to a legacy custom command");
        };

        assert_eq!(pipeline_name, "test.pipeline");
        assert_eq!(
            data.downcast_ref::<Vec<u8>>().unwrap().as_slice(),
            payload.as_ref()
        );
    }

    fn record_commands(
        tree: &RenderTree,
        node: crate::draw_cmd_v2::RenderNodeId,
        commands: Vec<DrawCommand>,
    ) {
        tree.context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(commands)
            .unwrap();
    }

    fn image(texture_id: u32) -> DrawCommand {
        DrawCommand::DrawImage {
            rect: Rect::new(0.0, 0.0, 8.0, 8.0),
            texture_id,
        }
    }

    fn fill(color: Color) -> DrawCommand {
        DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            color,
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }
    }

    fn packet_for(tree: &RenderTree) -> FramePacket {
        FramePacket::from_v2_direct_with_frame(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(64, 64)),
            Frame::new(LegacyDrawList::new(), 64, 64),
        )
        .unwrap()
    }

    #[test]
    fn textures_drawn_directly_by_planned_items_are_referenced_by_the_frame() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        let child = tree
            .add_child(root, Rect::new(0.0, 0.0, 32.0, 32.0))
            .unwrap();
        record_commands(&tree, root, vec![image(5), image(5)]);
        record_commands(
            &tree,
            child,
            vec![fill(Color::rgba8(1, 2, 3, 255)), image(9)],
        );

        let packet = packet_for(&tree);

        // Repeats collapse: the frame references a texture once.
        assert_eq!(
            packet.frame().draw_list.referenced_texture_ids(),
            vec![5, 9]
        );
    }

    #[test]
    fn a_planned_item_without_images_references_no_textures() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        record_commands(&tree, root, vec![fill(Color::rgba8(1, 2, 3, 255))]);

        assert!(
            packet_for(&tree)
                .frame()
                .draw_list
                .referenced_texture_ids()
                .is_empty()
        );
    }

    #[test]
    fn re_recording_an_item_changes_which_textures_the_frame_references() {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        record_commands(&tree, root, vec![image(5), image(6)]);
        assert_eq!(
            packet_for(&tree).frame().draw_list.referenced_texture_ids(),
            vec![5, 6]
        );

        record_commands(&tree, root, vec![image(6), image(7)]);
        assert_eq!(
            packet_for(&tree).frame().draw_list.referenced_texture_ids(),
            vec![6, 7]
        );

        record_commands(&tree, root, vec![fill(Color::rgba8(9, 9, 9, 255))]);
        assert!(
            packet_for(&tree)
                .frame()
                .draw_list
                .referenced_texture_ids()
                .is_empty()
        );
    }

    fn record_fill(tree: &RenderTree, node: crate::draw_cmd_v2::RenderNodeId, color: Color) {
        tree.context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![DrawCommand::FillRect {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                color,
                border_radius: [0.0; 4],
                border_width: [0.0; 4],
                border_color: Color::transparent(),
                outline_width: [0.0; 4],
                outline_color: Color::transparent(),
            }])
            .unwrap();
    }
}
