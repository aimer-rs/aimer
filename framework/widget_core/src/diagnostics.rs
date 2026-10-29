use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{BoxConstraint, ResolvedSize, Vec2d};
use aimer_canvas::{FontFamily, FontStyle};
use aimer_color::prelude::Color;

use crate::base::BuildContext;
use crate::{
    AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, VisitorElement,
    Widget,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverflowEdges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl OverflowEdges {
    pub fn has_overflow(self) -> bool {
        self.left > 0.0 || self.top > 0.0 || self.right > 0.0 || self.bottom > 0.0
    }

    #[cfg(debug_assertions)]
    fn maximum(self) -> f32 {
        self.left.max(self.top).max(self.right).max(self.bottom)
    }
}

pub fn detect_overflow(child: ResolvedSize, bounds: ResolvedSize, offset: Vec2d) -> OverflowEdges {
    OverflowEdges {
        left: (-offset.x).max(0.0),
        top: (-offset.y).max(0.0),
        right: (offset.x + child.width - bounds.width).max(0.0),
        bottom: (offset.y + child.height - bounds.height).max(0.0),
    }
}

#[derive(Clone, Debug, aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer.widget:aimer_widget::ErrorWidget", schema_only)]
/// A diagnostic widget that fills its available bounds with an error message.
///
/// Debug builds display the supplied message. Release builds log that message
/// and render a generic description so internal diagnostic details are not
/// exposed to users.
pub struct ErrorWidget {
    message: String,
}

impl ErrorWidget {
    /// Creates an error diagnostic with the message used for logging and debug
    /// rendering.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Widget for ErrorWidget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        #[cfg(not(debug_assertions))]
        aimer_utils::log::error(&self.message);
        ErrorElement {
            message: self.message,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "ErrorWidget"
    }
}

#[derive(Clone, Debug)]
pub struct ErrorElement {
    message: String,
}

impl ErrorElement {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Records the diagnostic into the current node's retained list: it fills the
    /// available bounds and writes `message` across them.
    ///
    /// The message is written in [`FontFamily::MONOSPACE`] — Aimer's bundled
    /// JetBrains Mono — because a recovered panic reports its own source line
    /// followed by a run of carets under the failing expression. Only a
    /// fixed-advance face keeps the two columns aligned; a proportional one
    /// leaves the carets pointing at whatever happens to sit above them.
    pub fn record_message(ctx: &BuildContext, message: &str) {
        let size = diagnostic_bounds(ctx);
        let scale = ctx.scale;
        let (pos_y, font_size) = if cfg!(target_os = "ios") || cfg!(target_os = "android") {
            (400.0, 40.0)
        } else {
            (200.0, 34.0)
        };

        #[cfg(debug_assertions)]
        let text = message;
        #[cfg(not(debug_assertions))]
        let text = "A layout error occurred";
        #[cfg(not(debug_assertions))]
        let _ = message;

        let canvas = aimer_canvas::Canvas::of(ctx);
        let (background_red, background_green, background_blue, background_alpha) =
            Color::RED.to_rgba();
        canvas.fill_rect(
            aimer_cupid::utilities::Rect::new(
                0.0,
                0.0,
                size.width / scale,
                size.height / scale,
            ),
            [background_red, background_green, background_blue, background_alpha],
        );
        let (red, green, blue, alpha) = Color::YELLOW.to_rgba();
        canvas.draw_text_styled(
            std::sync::Arc::<str>::from(text),
            aimer_cupid::utilities::Vec2d::new(24.0 / scale, pos_y / scale),
            font_size / scale,
            aimer_cupid::utilities::Color::rgba8(red, green, blue, alpha),
            Some((size.width - 24.0).max(0.0) / scale),
            None,
            aimer_cupid::text_pipeline::TextOverflowMode::Wrap,
            aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Left,
            FontFamily::MONOSPACE,
            FontStyle::Normal,
            600,
            None,
            true,
        );
        canvas.finish();
    }

    /// Whether [`Self::record_message`] has finite, usable bounds to fill.
    pub fn can_record_message(ctx: &BuildContext) -> bool {
        let size = diagnostic_bounds(ctx);
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && size.width.is_finite()
            && size.height.is_finite()
            && size.width >= 0.0
            && size.height >= 0.0
    }
}

impl Drawable for ErrorElement {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        Self::can_record_message(ctx)
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        Self::record_message(ctx, &self.message);
    }
}

impl EventElement for ErrorElement {}
impl Rebuildable for ErrorElement {
    fn option_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl VisitorElement for ErrorElement {
    fn debug_name(&self) -> &'static str {
        "ErrorWidget"
    }
}

impl LayoutElement for ErrorElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        diagnostic_bounds(ctx)
    }
}

/// Wraps a child and paints a debug overflow warning around constrained edges.
///
/// The child is clipped to the available bounds by default. In debug builds,
/// overflowing edges receive striped markers and a label; release builds keep
/// only the optional clipping behavior.
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_widget::OverflowIndicator", schema_only)]
pub struct OverflowIndicator<W> {
    #[portable_child]
    child: W,
    label: Option<String>,
    #[portable_optional]
    clip: bool,
}

impl<W: Widget + 'static> OverflowIndicator<W> {
    /// Creates an indicator for `child`, with clipping enabled and a label
    /// derived from [`Widget::debug_name`].
    pub fn new(child: W) -> Self {
        Self {
            child,
            label: None,
            clip: true,
        }
    }

    /// Overrides the label displayed in the debug overflow message.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Controls whether child drawing is clipped to the constrained bounds.
    ///
    /// The default is `true`. Overflow detection and debug markers are still
    /// computed when clipping is disabled.
    pub fn clip(mut self, clip: bool) -> Self {
        self.clip = clip;
        self
    }
}

impl<W: Widget + 'static> Widget for OverflowIndicator<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let label = self
            .label
            .unwrap_or_else(|| self.child.debug_name().to_string());
        let child = self.child.to_element(ctx);
        let paint = Rc::new(Cell::new(OverflowPaintData::default()));
        let decoration = RawOverflowDecoration {
            #[cfg(debug_assertions)]
            label: label.clone(),
            paint: Rc::clone(&paint),
            last_v2_paint: Cell::new(None),
        }
        .boxed();
        RawOverflowIndicator {
            child,
            label,
            clip: self.clip,
            paint,
            decoration,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "OverflowIndicator"
    }
}

struct RawOverflowIndicator {
    child: AnyElement,
    label: String,
    clip: bool,
    paint: Rc<Cell<OverflowPaintData>>,
    decoration: AnyElement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct OverflowPaintData {
    bounds: ResolvedSize,
    overflow: OverflowEdges,
}

impl Drawable for RawOverflowIndicator {
    fn update(&self, ctx: &BuildContext) {
        let bounds = self.computed_size(ctx);
        let child_size = self.child.computed_size(ctx);
        let _overflow = detect_overflow(child_size, bounds, Vec2d::default());

        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite() && ctx.scale > 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let bounds = self.computed_size(ctx);
        let child_size = self.child.computed_size(ctx);
        self.paint.set(OverflowPaintData {
            bounds,
            overflow: detect_overflow(child_size, bounds, Vec2d::default()),
        });
    }

    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        let expected = match child_index {
            0 => self.child.id(),
            1 => self.decoration.id(),
            _ => return None,
        };
        (child.id() == expected).then(|| ctx.clone())
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
        let child_ctx = self.retained_v2_child_context_at(ctx, child, child_index)?;
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let paint = self.paint.get();
        if child_index == 0 {
            let size = child
                .retained_v2_bounds(&child_ctx)
                .unwrap_or_else(|| child.content_size(&child_ctx));
            let position = child.pos().unwrap_or_default();
            return Some((
                aimer_cupid::draw_cmd_v2::Rect::new(
                    position.x / scale,
                    position.y / scale,
                    size.width / scale,
                    size.height / scale,
                ),
                self.clip.then(|| {
                    aimer_cupid::draw_cmd_v2::Rect::new(
                        0.0,
                        0.0,
                        paint.bounds.width / scale,
                        paint.bounds.height / scale,
                    )
                }),
            ));
        }
        Some((
            aimer_cupid::draw_cmd_v2::Rect::new(
                0.0,
                0.0,
                paint.bounds.width / scale,
                paint.bounds.height / scale,
            ),
            None,
        ))
    }
}

impl EventElement for RawOverflowIndicator {
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Rebuildable for RawOverflowIndicator {}

impl VisitorElement for RawOverflowIndicator {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
        visitor(self.decoration.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "OverflowIndicator"
    }
}

struct RawOverflowDecoration {
    #[cfg(debug_assertions)]
    label: String,
    paint: Rc<Cell<OverflowPaintData>>,
    last_v2_paint: Cell<Option<(OverflowPaintData, u32)>>,
}

impl Drawable for RawOverflowDecoration {
    fn update(&self, _ctx: &BuildContext) {}

    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let paint = self.paint.get();
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && paint.bounds.width.is_finite()
            && paint.bounds.height.is_finite()
            && paint.bounds.width >= 0.0
            && paint.bounds.height >= 0.0
    }

    #[cfg(debug_assertions)]
    fn paint_local_v2(&self, ctx: &BuildContext) {
        let paint = self.paint.get();
        let scale = ctx.scale;
        let canvas = aimer_canvas::Canvas::of(ctx);
        if !paint.overflow.has_overflow()
            || paint.bounds.width <= 0.0
            || paint.bounds.height <= 0.0
        {
            canvas.finish();
            self.last_v2_paint
                .set(Some((paint, scale.to_bits())));
            return;
        }

        const THICKNESS: f32 = 8.0;
        const STRIPE: f32 = 6.0;
        let (yellow_red, yellow_green, yellow_blue, yellow_alpha) = Color::YELLOW.to_rgba();
        let (black_red, black_green, black_blue, black_alpha) = Color::BLACK.to_rgba();
        let paint_horizontal = |y: f32| {
            let mut x = 0.0;
            let mut yellow = true;
            while x < paint.bounds.width {
                let (red, green, blue, alpha) = if yellow {
                    (yellow_red, yellow_green, yellow_blue, yellow_alpha)
                } else {
                    (black_red, black_green, black_blue, black_alpha)
                };
                canvas.fill_rect(
                    aimer_cupid::utilities::Rect::new(
                        x / scale,
                        y / scale,
                        STRIPE.min(paint.bounds.width - x) / scale,
                        THICKNESS.min(paint.bounds.height) / scale,
                    ),
                    [red, green, blue, alpha],
                );
                yellow = !yellow;
                x += STRIPE;
            }
        };
        let paint_vertical = |x: f32| {
            let mut y = 0.0;
            let mut yellow = true;
            while y < paint.bounds.height {
                let (red, green, blue, alpha) = if yellow {
                    (yellow_red, yellow_green, yellow_blue, yellow_alpha)
                } else {
                    (black_red, black_green, black_blue, black_alpha)
                };
                canvas.fill_rect(
                    aimer_cupid::utilities::Rect::new(
                        x / scale,
                        y / scale,
                        THICKNESS.min(paint.bounds.width) / scale,
                        STRIPE.min(paint.bounds.height - y) / scale,
                    ),
                    [red, green, blue, alpha],
                );
                yellow = !yellow;
                y += STRIPE;
            }
        };

        if paint.overflow.top > 0.0 {
            paint_horizontal(0.0);
        }
        if paint.overflow.bottom > 0.0 {
            paint_horizontal((paint.bounds.height - THICKNESS).max(0.0));
        }
        if paint.overflow.left > 0.0 {
            paint_vertical(0.0);
        }
        if paint.overflow.right > 0.0 {
            paint_vertical((paint.bounds.width - THICKNESS).max(0.0));
        }

        let text = format!("{} overflowed by {:.1}px", self.label, paint.overflow.maximum());
        let width = ((text.len() as f32 * 6.0) + 8.0).min(paint.bounds.width);
        let (black_red, black_green, black_blue, black_alpha) = Color::BLACK.to_rgba();
        canvas.fill_rect(
            aimer_cupid::utilities::Rect::new(
                0.0,
                0.0,
                width / scale,
                18.0_f32.min(paint.bounds.height) / scale,
            ),
            [black_red, black_green, black_blue, black_alpha],
        );
        let (yellow_red, yellow_green, yellow_blue, yellow_alpha) = Color::YELLOW.to_rgba();
        canvas.draw_text_styled(
            std::sync::Arc::<str>::from(text),
            aimer_cupid::utilities::Vec2d::new(4.0 / scale, 13.0 / scale),
            10.0 / scale,
            aimer_cupid::utilities::Color::rgba8(
                yellow_red,
                yellow_green,
                yellow_blue,
                yellow_alpha,
            ),
            None,
            None,
            aimer_cupid::text_pipeline::TextOverflowMode::Clip,
            aimer_cupid::text_pipeline::text_layout::TextHorizontalAlign::Left,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            600,
            None,
            true,
        );
        canvas.finish();
        self.last_v2_paint
            .set(Some((paint, scale.to_bits())));
    }

    #[cfg(not(debug_assertions))]
    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
        self.last_v2_paint
            .set(Some((self.paint.get(), ctx.scale.to_bits())));
    }

    fn local_v2_paint_needs_recording(&self, ctx: &BuildContext) -> bool {
        self.last_v2_paint.get() != Some((self.paint.get(), ctx.scale.to_bits()))
    }
}

impl EventElement for RawOverflowDecoration {}
impl Rebuildable for RawOverflowDecoration {}

impl VisitorElement for RawOverflowDecoration {
    fn debug_name(&self) -> &'static str {
        "OverflowDecoration"
    }
}

impl LayoutElement for RawOverflowDecoration {
    fn computed_size(&self, _ctx: &BuildContext) -> ResolvedSize {
        self.paint.get().bounds
    }
}

impl LayoutElement for RawOverflowIndicator {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let child = self.child.computed_size(ctx);
        ResolvedSize {
            width: constrain(
                child.width,
                ctx.box_constraint.min_width,
                ctx.box_constraint.max_width,
            ),
            height: constrain(
                child.height,
                ctx.box_constraint.min_height,
                ctx.box_constraint.max_height,
            ),
        }
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }
}

fn constrain(value: f32, min: f32, max: f32) -> f32 {
    if max == f32::MAX {
        value.max(min)
    } else {
        value.clamp(min, max.max(min))
    }
}

fn diagnostic_bounds(ctx: &BuildContext) -> ResolvedSize {
    let BoxConstraint {
        min_width,
        min_height,
        max_width,
        max_height,
    } = ctx.box_constraint;
    ResolvedSize {
        width: if max_width == f32::MAX {
            ctx.parent_size.width.max(min_width)
        } else {
            max_width.max(min_width)
        },
        height: if max_height == f32::MAX {
            ctx.parent_size.height.max(min_height)
        } else {
            max_height.max(min_height)
        },
    }
}



#[cfg(test)]
mod tests {
    use aimer_attribute::ResolvedSize;
    use aimer_canvas::{FrameCanvas, FontFamily, InnerCanvas};

    use super::{ErrorElement, OverflowEdges, detect_overflow};
    use crate::base::{BuildContext, WindowHandle};

    #[tokio::test]
    async fn the_diagnostic_message_is_written_in_the_bundled_monospace_face() {
        let canvas = Box::leak(Box::new(InnerCanvas::new()));
        let ctx = BuildContext::new(
            FrameCanvas::new(canvas),
            ResolvedSize {
                width: 800.0,
                height: 600.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            tokio::runtime::Handle::current(),
        );

        let tree = aimer_cupid::draw_cmd_v2::RenderTree::new();
        let root = tree
            .add_root(aimer_cupid::draw_cmd_v2::Rect::new(0.0, 0.0, 800.0, 600.0))
            .unwrap();
        let node_context = tree.context(root).unwrap();
        ctx.with_local_v2_paint_context(node_context, |ctx| {
            assert!(ErrorElement::can_record_message(ctx));
            ErrorElement::record_message(ctx, "Widget `Broken` panicked during build: boom");
        });

        let family = tree
            .draw_list_snapshot(root)
            .unwrap()
            .commands
            .iter()
            .find_map(|command| match command {
                aimer_cupid::draw_cmd_v2::DrawCommand::DrawText { font_family, .. } => {
                    Some(*font_family)
                }
                _ => None,
            });

        assert_eq!(family, Some(FontFamily::MONOSPACE));
    }

    #[test]
    fn detects_each_overflowing_edge() {
        let overflow = detect_overflow(
            ResolvedSize {
                width: 120.0,
                height: 80.0,
            },
            ResolvedSize {
                width: 100.0,
                height: 60.0,
            },
            (-4.0, -3.0).into(),
        );

        assert_eq!(
            overflow,
            OverflowEdges {
                left: 4.0,
                top: 3.0,
                right: 16.0,
                bottom: 17.0
            }
        );
    }

    #[test]
    fn fitting_child_has_no_overflow() {
        let overflow = detect_overflow(
            ResolvedSize {
                width: 40.0,
                height: 30.0,
            },
            ResolvedSize {
                width: 100.0,
                height: 60.0,
            },
            (10.0, 10.0).into(),
        );

        assert!(!overflow.has_overflow());
    }
}
