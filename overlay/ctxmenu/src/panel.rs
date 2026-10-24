//! The panel a menu's content sits on.
//!
//! It is deliberately *not* a [`aimer_container::Container`]: a container with
//! automatic dimensions only shrinks to its child when the space around it is
//! unbounded, and a floating menu is handed the whole viewport, so a container
//! would paint the panel over the entire window. The panel therefore measures
//! its child itself and is exactly as large as the child plus the style's
//! padding — which is also what tells [`aimer_modal::Floating`] where to place
//! it.

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_events::element::ElementEvent;
use aimer_macro::Rebuildable;
use aimer_style::{BoxDecoration, BoxShadow};
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventResult, LayoutElement, VisitorElement,
};

use crate::style::ContextMenuStyle;

/// The styled panel wrapped around a menu's content.
#[derive(Rebuildable)]
pub(crate) struct RawContextMenuPanel {
    child: AnyElement,
    style: ContextMenuStyle,
}

#[derive(Clone, Copy)]
struct RetainedPanelMetrics {
    size: ResolvedSize,
    radii: [f32; 4],
    background: Option<Color>,
    border_width: [f32; 4],
    border_color: Color,
    outline_width: [f32; 4],
    outline_color: Color,
    has_frame: bool,
    paint_outsets: [f32; 4],
}

impl RawContextMenuPanel {
    /// Creates a panel drawing `style` around `child`.
    #[inline]
    pub(crate) fn element(child: AnyElement, style: ContextMenuStyle) -> AnyElement {
        Self { child, style }.boxed()
    }

    /// The padding around the content, in physical pixels.
    fn insets(&self, ctx: &BuildContext) -> (f32, f32, f32, f32) {
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        let width = ctx.box_constraint.max_width;
        let height = ctx.box_constraint.max_height;
        (
            self.style.padding.left.value(width, scale),
            self.style.padding.top.value(height, scale),
            self.style.padding.right.value(width, scale),
            self.style.padding.bottom.value(height, scale),
        )
    }

    /// The panel's size: its content plus its padding.
    ///
    /// An empty content collapses the panel to nothing, so a menu opened with no
    /// verbs paints no background rather than a bare rounded rectangle.
    fn panel_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let (left, top, right, bottom) = self.insets(ctx);
        let content = self.child.content_size(ctx);
        if content.width <= 0.0 || content.height <= 0.0 {
            return ResolvedSize {
                width: 0.0,
                height: 0.0,
            };
        }
        ResolvedSize {
            width: content.width + left + right,
            height: content.height + top + bottom,
        }
    }

    fn retained_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
    ) -> (Vec2d, ResolvedSize, BuildContext<'a>) {
        let size = self.panel_size(ctx);
        let (left, top, right, bottom) = self.insets(ctx);
        let outsets = self
            .retained_panel_metrics(ctx)
            .map_or([0.0; 4], |metrics| metrics.paint_outsets);
        let offset = Vec2d {
            x: left + outsets[0] * ctx.scale,
            y: top + outsets[1] * ctx.scale,
        };
        let child_size = ResolvedSize {
            width: (size.width - left - right).max(0.0),
            height: (size.height - top - bottom).max(0.0),
        };
        let mut child_context = ctx.clone();
        child_context.parent_size = child_size;
        child_context.parent_pos = Vec2d {
            x: ctx.parent_pos.x + offset.x,
            y: ctx.parent_pos.y + offset.y,
        };
        child_context.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: child_size.width,
            max_height: child_size.height,
        };
        (offset, child_size, child_context)
    }

    fn retained_panel_metrics(&self, ctx: &BuildContext) -> Option<RetainedPanelMetrics> {
        retained_panel_metrics(&self.style.panel, self.panel_size(ctx), ctx.scale)
    }
}

fn retained_panel_metrics(
    decoration: &BoxDecoration,
    size: ResolvedSize,
    scale: f32,
) -> Option<RetainedPanelMetrics> {
    if !scale.is_finite()
        || scale <= 0.0
        || !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
        || !(size.width / scale).is_finite()
        || !(size.height / scale).is_finite()
    {
        return None;
    }

    let radii = decoration
        .border_radius
        .resolve(size.width, size.height, scale)
        .map(|radius| radius / scale);
    if !radii
        .iter()
        .all(|radius| radius.is_finite() && *radius >= 0.0)
    {
        return None;
    }

    let (border_left, border_top, border_right, border_bottom) =
        decoration.border.strokes(size.width, size.height, scale);
    let (outline_left, outline_top, outline_right, outline_bottom) =
        decoration.outline.strokes(size.width, size.height, scale);
    let has_border = decoration
        .border
        .has_visible_border(size.width, size.height, scale);
    let has_outline = decoration
        .outline
        .has_visible_outline(size.width, size.height, scale);
    let border_width = if has_border {
        [
            border_top / scale,
            border_right / scale,
            border_bottom / scale,
            border_left / scale,
        ]
    } else {
        [0.0; 4]
    };
    let outline_width = if has_outline {
        [
            outline_top / scale,
            outline_right / scale,
            outline_bottom / scale,
            outline_left / scale,
        ]
    } else {
        [0.0; 4]
    };
    if border_width
        .iter()
        .chain(outline_width.iter())
        .any(|width| !width.is_finite() || *width < 0.0)
    {
        return None;
    }

    let mut shadow_x = 0.0f32;
    let mut shadow_y = 0.0f32;
    for shadow in &decoration.box_shadow {
        if shadow.color == Color::Transparent {
            continue;
        }
        let Some((params, _)) = retained_shadow_parameters(shadow, scale) else {
            return None;
        };
        shadow_x = shadow_x.max(params[2] + params[3].abs() + params[0].abs());
        shadow_y = shadow_y.max(params[2] + params[3].abs() + params[1].abs());
    }
    if !shadow_x.is_finite() || !shadow_y.is_finite() {
        return None;
    }

    let border_color = if has_border {
        decoration
            .border
            .effective_color(size.width, size.height, scale)
    } else {
        Color::Transparent
    };
    let outline_color = if has_outline {
        decoration
            .outline
            .effective_color(size.width, size.height, scale)
    } else {
        Color::Transparent
    };
    let paint_outsets = [
        shadow_x.max(outline_width[3]),
        shadow_y.max(outline_width[0]),
        shadow_x.max(outline_width[1]),
        shadow_y.max(outline_width[2]),
    ];
    Some(RetainedPanelMetrics {
        size,
        radii,
        background: decoration.background_color.get(),
        border_width,
        border_color,
        outline_width,
        outline_color,
        has_frame: has_border || has_outline || decoration.background_color.get().is_some(),
        paint_outsets,
    })
}

fn retained_panel_commands(
    decoration: &BoxDecoration,
    metrics: RetainedPanelMetrics,
    scale: f32,
) -> Vec<aimer_cupid::draw_cmd_v2::DrawCommand> {
    use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};

    let mut commands = Vec::with_capacity(decoration.box_shadow.len() + 1);
    for shadow in decoration.box_shadow.iter().filter(|shadow| !shadow.inset) {
        if let Some(command) = retained_shadow_command(shadow, metrics, scale) {
            commands.push(command);
        }
    }
    if metrics.has_frame {
        commands.push(DrawCommand::FillRect {
            rect: Rect::new(
                metrics.paint_outsets[0],
                metrics.paint_outsets[1],
                metrics.size.width / scale,
                metrics.size.height / scale,
            ),
            color: metrics
                .background
                .map(retained_color)
                .unwrap_or_else(aimer_cupid::utilities::Color::transparent),
            border_radius: metrics.radii,
            border_width: metrics.border_width,
            border_color: retained_color(metrics.border_color),
            outline_width: metrics.outline_width,
            outline_color: retained_color(metrics.outline_color),
        });
    }
    for shadow in decoration.box_shadow.iter().filter(|shadow| shadow.inset) {
        if let Some(command) = retained_shadow_command(shadow, metrics, scale) {
            commands.push(command);
        }
    }
    commands
}

fn retained_shadow_command(
    shadow: &BoxShadow,
    metrics: RetainedPanelMetrics,
    scale: f32,
) -> Option<aimer_cupid::draw_cmd_v2::DrawCommand> {
    use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect};

    if shadow.color == Color::Transparent {
        return None;
    }
    let (params, side) = retained_shadow_parameters(shadow, scale)?;
    if params[2] == 0.0 && params[3] == 0.0 && params[0] == 0.0 && params[1] == 0.0 && !shadow.inset
    {
        return None;
    }
    Some(DrawCommand::DrawShadowRect {
        rect: Rect::new(
            metrics.paint_outsets[0],
            metrics.paint_outsets[1],
            metrics.size.width / scale,
            metrics.size.height / scale,
        ),
        shadow_color: retained_color(shadow.color),
        shadow_params: params,
        border_radius: metrics.radii,
        inset: shadow.inset,
        side_params: side,
    })
}

fn retained_shadow_parameters(shadow: &BoxShadow, scale: f32) -> Option<([f32; 4], [f32; 3])> {
    if ![shadow.offset_x, shadow.offset_y, shadow.blur, shadow.spread]
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    // The legacy decoration painter supplies shadow sizes in physical pixels;
    // retained local commands are logical and receive the device scale later.
    let params = [
        shadow.offset_x / scale,
        shadow.offset_y / scale,
        shadow.blur.max(0.0) / scale,
        shadow.spread / scale,
    ];
    let side = shadow.side.to_shader_params();
    [
        params[0], params[1], params[2], params[3], side.0, side.1, side.2,
    ]
    .iter()
    .all(|value| value.is_finite())
    .then_some((params, [side.0, side.1, side.2]))
}

fn retained_color(color: Color) -> aimer_cupid::utilities::Color {
    let (red, green, blue, alpha) = color.to_rgba();
    aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha)
}

impl Drawable for RawContextMenuPanel {
    fn update(&self, ctx: &BuildContext) {
        let size = self.panel_size(ctx);
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }

        let (left, top, right, bottom) = self.insets(ctx);
        ctx.canvas.save();
        ctx.canvas.translate(Vec2d { x: left, y: top });
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = ResolvedSize {
            width: (size.width - left - right).max(0.0),
            height: (size.height - top - bottom).max(0.0),
        };
        child_ctx.box_constraint.max_width = child_ctx.parent_size.width;
        child_ctx.box_constraint.max_height = child_ctx.parent_size.height;
        self.child.update(&child_ctx);
        ctx.canvas.restore();
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        self.retained_panel_metrics(ctx).is_some()
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let metrics = self
            .retained_panel_metrics(ctx)
            .expect("retained panel paint was checked before recording");
        let canvas = aimer_canvas::Canvas::of(ctx);
        for command in retained_panel_commands(&self.style.panel, metrics, ctx.scale) {
            canvas.push_command(command);
        }
        canvas.finish();
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        let (offset, _, child_context) = self.retained_child_layout(ctx);
        ctx.canvas.save();
        ctx.canvas.translate(offset);
        self.child.update(&child_context);
        ctx.canvas.restore();
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.retained_panel_metrics(ctx)
            .map(|metrics| metrics.paint_outsets)
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        if child_index != 0 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        Some(self.retained_child_layout(ctx).2)
    }

    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        if child_index != 0 || !std::ptr::eq(child, self.child.as_ref()) {
            return None;
        }
        let (offset, child_size, _) = self.retained_child_layout(ctx);
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                offset.x / scale,
                offset.y / scale,
                child_size.width / scale,
                child_size.height / scale,
            ),
            None,
        ))
    }
}

impl EventElement for RawContextMenuPanel {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawContextMenuPanel {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.panel_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.panel_size(ctx)
    }
}

impl VisitorElement for RawContextMenuPanel {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "ContextMenu"
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::{Dimension, ResolvedSize};
    use aimer_style::{BorderSlice, BorderStyle, BoxBorder, BoxDecoration, BoxOutline, BoxShadow};
    use aimer_widget::base::Color;

    use super::{retained_panel_commands, retained_panel_metrics};

    #[test]
    fn retained_panel_metrics_cover_borders_outlines_and_shadow_damage() {
        let border = BorderSlice::new()
            .style(BorderStyle::Solid)
            .stroke(Dimension::Px(2.0))
            .color(Color::Rgba(10, 20, 30, 255));
        let outline = BorderSlice::new()
            .style(BorderStyle::Solid)
            .stroke(Dimension::Px(4.0))
            .color(Color::Rgba(40, 50, 60, 255));
        let shadow = BoxShadow::new()
            .offset_x(3.0)
            .offset_y(-4.0)
            .blur(5.0)
            .spread(2.0);
        let inset_shadow = BoxShadow::new().inset(true).blur(1.0);
        let decoration = BoxDecoration::new()
            .background_color(Color::Rgba(70, 80, 90, 255))
            .border(BoxBorder::all(border))
            .outline(BoxOutline::all(outline))
            .border_radius(8.0)
            .box_shadow([shadow, inset_shadow]);

        let metrics = retained_panel_metrics(
            &decoration,
            ResolvedSize {
                width: 200.0,
                height: 80.0,
            },
            2.0,
        )
        .expect("finite custom decorations can stay on the retained path");

        assert_eq!(metrics.radii, [8.0; 4]);
        assert_eq!(metrics.border_width, [2.0; 4]);
        assert_eq!(metrics.outline_width, [4.0; 4]);
        assert_eq!(metrics.paint_outsets, [5.0, 5.5, 5.0, 5.5]);

        let commands = retained_panel_commands(&decoration, metrics, 2.0);
        assert_eq!(commands.len(), 3);
        let aimer_cupid::draw_cmd_v2::DrawCommand::DrawShadowRect {
            rect: shadow_rect,
            shadow_color,
            shadow_params,
            inset,
            ..
        } = &commands[0]
        else {
            panic!("outer shadow is recorded before the panel");
        };
        assert_eq!(
            *shadow_color,
            aimer_cupid::utilities::Color::rgba8(0, 0, 0, 128)
        );
        assert_eq!(shadow_rect.x, 5.0);
        assert_eq!(shadow_rect.y, 5.5);
        assert_eq!(*shadow_params, [1.5, -2.0, 2.5, 1.0]);
        assert!(!inset);
        let aimer_cupid::draw_cmd_v2::DrawCommand::FillRect {
            rect,
            color,
            border_radius,
            border_width,
            border_color,
            outline_width,
            outline_color,
            ..
        } = &commands[1]
        else {
            panic!("panel fill and stroke follow its outer shadow");
        };
        assert_eq!(
            *color,
            aimer_cupid::utilities::Color::rgba8(70, 80, 90, 255)
        );
        assert_eq!((rect.x, rect.y), (5.0, 5.5));
        assert_eq!(*border_radius, [8.0; 4]);
        assert_eq!(*border_width, [2.0; 4]);
        assert_eq!(
            *border_color,
            aimer_cupid::utilities::Color::rgba8(10, 20, 30, 255)
        );
        assert_eq!(*outline_width, [4.0; 4]);
        assert_eq!(
            *outline_color,
            aimer_cupid::utilities::Color::rgba8(40, 50, 60, 255)
        );
        let aimer_cupid::draw_cmd_v2::DrawCommand::DrawShadowRect { inset, .. } = &commands[2]
        else {
            panic!("inset shadow is recorded after the panel");
        };
        assert!(inset);
    }

    #[test]
    fn retained_panel_metrics_fall_back_for_non_finite_shadow_geometry() {
        let decoration = BoxDecoration::new().add_shadow(BoxShadow::new().blur(f32::NAN));
        assert!(
            retained_panel_metrics(
                &decoration,
                ResolvedSize {
                    width: 200.0,
                    height: 80.0,
                },
                2.0,
            )
            .is_none()
        );
    }
}
