//! Border and outline types shared by [`BoxDecoration`](crate::BoxDecoration).
//!
//! A [`BoxBorder`] draws inside the box and is reserved as child inset, while a
//! [`BoxOutline`] draws outside and only enlarges the paint area. Both are made
//! of four [`BorderSlice`]s, and [`RawBoxBorder`] is the flattened form the paint
//! pipeline retains.

use aimer_attribute::dimension::Dimension;
use aimer_color::prelude::Color;
use aimer_widget::Drawable;
use aimer_widget::base::BuildContext;

/// How a single border side is drawn.
///
/// Only [`BorderStyle::Solid`] is rendered today; `Dashed` and `Dotted` are
/// accepted so callers can declare intent, but they paint like a solid stroke
/// until the pipeline supports them. The default is [`BorderStyle::None`],
/// which suppresses the side entirely regardless of its stroke width.
#[allow(dead_code)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum BorderStyle {
    /// A continuous line.
    Solid,
    /// A line broken into dashes.
    Dashed,
    /// A line of dots.
    Dotted,
    /// No line is drawn and the side is ignored when computing visibility.
    #[default]
    None,
}

/// Which side of the box edge a border grows towards.
#[allow(dead_code)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum BorderMode {
    /// The stroke is drawn inside the box edge.
    #[default]
    Inside,
    /// The stroke is drawn outside the box edge and extends the paint area.
    Outside,
}

/// The thickness of a border side, as a [`Dimension`].
///
/// `Px` is scaled by the device scale, `Percent` is relative to the box extent on
/// the matching axis and `Auto` resolves to zero.
pub type Stroke = Dimension;

/// Style, thickness and color of one side of a border or outline.
///
/// A slice is visible only when its style is not [`BorderStyle::None`] and its
/// stroke resolves to a positive width.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BorderSlice {
    /// How the side is drawn.
    pub style: BorderStyle,
    /// The side's thickness.
    pub stroke: Stroke,
    /// The side's color.
    pub color: Color,
}

impl BorderSlice {
    /// Creates an invisible slice (no style, zero stroke, transparent).
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the line style.
    #[inline]
    pub fn style(mut self, style: BorderStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the thickness; accepts anything convertible to a [`Stroke`].
    #[inline]
    pub fn stroke(mut self, stroke: impl Into<Stroke>) -> Self {
        self.stroke = stroke.into();
        self
    }

    /// Sets the color.
    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }
}

/// Resolves a [`Dimension`] to physical pixels.
///
/// `parent_val` is the box extent on the matching axis, already in physical
/// pixels. Only `Px` values are multiplied by `scale`.
#[inline]
pub fn resolve_dim(dim: Dimension, parent_val: f32, scale: f32) -> f32 {
    match dim {
        Dimension::Px(w) => w * scale,
        Dimension::Percent(p) => parent_val * (p / 100.0),
        Dimension::Auto => 0.0,
    }
}

/// An inside border usable directly as a [`crate::Decoration`].
/// Containers reserve its stroke widths as child insets without clipping.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BoxBorder {
    /// The left side.
    pub left: BorderSlice,
    /// The right side.
    pub right: BorderSlice,
    /// The top side.
    pub top: BorderSlice,
    /// The bottom side.
    pub bottom: BorderSlice,
}

/// An outside outline usable directly as a [`crate::Decoration`].
/// Its paint outsets include the stroke widths; child geometry is unchanged.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BoxOutline {
    /// The left side.
    pub left: BorderSlice,
    /// The right side.
    pub right: BorderSlice,
    /// The top side.
    pub top: BorderSlice,
    /// The bottom side.
    pub bottom: BorderSlice,
}

/// Flattened border state retained by the paint pipeline.
///
/// Built from a [`BoxBorder`] or [`BoxOutline`] plus the box corner radii; it is
/// what actually paints, so it carries the [`BorderMode`] and `radius`.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct RawBoxBorder {
    pub left: BorderSlice,
    pub right: BorderSlice,
    pub top: BorderSlice,
    pub bottom: BorderSlice,
    /// Whether the stroke is inside or outside the box.
    pub mode: BorderMode,
    /// Corner radii as `[top_left, top_right, bottom_right, bottom_left]` in physical pixels.
    pub radius: [f32; 4],
}

impl RawBoxBorder {
    /// Bundles the four sides, mode and corner radii.
    #[allow(dead_code)]
    pub(crate) fn new(
        left: BorderSlice,
        right: BorderSlice,
        top: BorderSlice,
        bottom: BorderSlice,
        mode: BorderMode,
        radius: [f32; 4],
    ) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
            mode,
            radius,
        }
    }
}

impl BoxBorder {
    /// Creates a border with no visible sides.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the left side.
    #[inline]
    pub fn left(mut self, left: BorderSlice) -> Self {
        self.left = left;
        self
    }

    /// Sets the right side.
    #[inline]
    pub fn right(mut self, right: BorderSlice) -> Self {
        self.right = right;
        self
    }

    /// Sets the top side.
    #[inline]
    pub fn top(mut self, top: BorderSlice) -> Self {
        self.top = top;
        self
    }

    /// Sets the bottom side.
    #[inline]
    pub fn bottom(mut self, bottom: BorderSlice) -> Self {
        self.bottom = bottom;
        self
    }

    /// Applies the same slice to all four sides.
    #[inline]
    pub fn all(border: BorderSlice) -> Self {
        Self {
            left: border,
            right: border,
            top: border,
            bottom: border,
        }
    }

    /// Returns the resolved border stroke for each side: (left, top, right,
    /// bottom).
    #[inline]
    pub fn strokes(&self, box_width: f32, box_height: f32, scale: f32) -> (f32, f32, f32, f32) {
        (
            resolve_dim(self.left.stroke, box_width, scale),
            resolve_dim(self.top.stroke, box_height, scale),
            resolve_dim(self.right.stroke, box_width, scale),
            resolve_dim(self.bottom.stroke, box_height, scale),
        )
    }

    /// Returns true if any side has a non-None style and non-zero stroke.
    #[inline]
    pub fn has_visible_border(&self, box_width: f32, box_height: f32, scale: f32) -> bool {
        let (l, t, r, b) = self.strokes(box_width, box_height, scale);
        (l > 0.0 && self.left.style != BorderStyle::None)
            || (t > 0.0 && self.top.style != BorderStyle::None)
            || (r > 0.0 && self.right.style != BorderStyle::None)
            || (b > 0.0 && self.bottom.style != BorderStyle::None)
    }

    /// Returns the color to paint the border with.
    ///
    /// The per-side border GPU pipeline currently supports only a single
    /// uniform border color, so we cannot honor a different color per side.
    /// Picking `left.color` unconditionally is wrong when only another side is
    /// set (e.g. a `bottom`-only border): `left.color` is then the default
    /// `Color::Transparent` and the border renders invisibly. Instead, return
    /// the color of the first side that is actually visible (non-`None` style
    /// and non-zero stroke), falling back to `left.color`.
    #[inline]
    pub fn effective_color(&self, box_width: f32, box_height: f32, scale: f32) -> Color {
        let (l, t, r, b) = self.strokes(box_width, box_height, scale);
        if l > 0.0 && self.left.style != BorderStyle::None {
            self.left.color
        } else if t > 0.0 && self.top.style != BorderStyle::None {
            self.top.color
        } else if r > 0.0 && self.right.style != BorderStyle::None {
            self.right.color
        } else if b > 0.0 && self.bottom.style != BorderStyle::None {
            self.bottom.color
        } else {
            self.left.color
        }
    }

    /// Applies `border` to the top and bottom sides only.
    #[inline]
    pub fn horizontal(border: BorderSlice) -> Self {
        Self {
            top: border,
            bottom: border,
            ..Default::default()
        }
    }

    /// Applies `border` to the left and right sides only.
    #[inline]
    pub fn vertical(border: BorderSlice) -> Self {
        Self {
            left: border,
            right: border,
            ..Default::default()
        }
    }
}

impl BoxOutline {
    /// Creates an outline with no visible sides.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the left side.
    #[inline]
    pub fn left(mut self, left: BorderSlice) -> Self {
        self.left = left;
        self
    }

    /// Sets the right side.
    #[inline]
    pub fn right(mut self, right: BorderSlice) -> Self {
        self.right = right;
        self
    }

    /// Sets the top side.
    #[inline]
    pub fn top(mut self, top: BorderSlice) -> Self {
        self.top = top;
        self
    }

    /// Sets the bottom side.
    #[inline]
    pub fn bottom(mut self, bottom: BorderSlice) -> Self {
        self.bottom = bottom;
        self
    }

    /// Applies the same slice to all four sides.
    #[inline]
    pub fn all(border: BorderSlice) -> Self {
        Self {
            left: border,
            right: border,
            top: border,
            bottom: border,
        }
    }

    /// Returns true if any side has a non-None style and non-zero stroke.
    #[inline]
    pub fn has_visible_outline(&self, box_width: f32, box_height: f32, scale: f32) -> bool {
        let (l, t, r, b) = self.strokes(box_width, box_height, scale);
        (l > 0.0 && self.left.style != BorderStyle::None)
            || (t > 0.0 && self.top.style != BorderStyle::None)
            || (r > 0.0 && self.right.style != BorderStyle::None)
            || (b > 0.0 && self.bottom.style != BorderStyle::None)
    }

    /// Returns the color to paint the outline with. See
    /// [`BoxBorder::effective_color`] — the per-side outline pipeline supports
    /// a single uniform color, so we pick the color of the first visible
    /// side and fall back to `left.color`.
    pub fn effective_color(&self, box_width: f32, box_height: f32, scale: f32) -> Color {
        let (l, t, r, b) = self.strokes(box_width, box_height, scale);
        if l > 0.0 && self.left.style != BorderStyle::None {
            self.left.color
        } else if t > 0.0 && self.top.style != BorderStyle::None {
            self.top.color
        } else if r > 0.0 && self.right.style != BorderStyle::None {
            self.right.color
        } else if b > 0.0 && self.bottom.style != BorderStyle::None {
            self.bottom.color
        } else {
            self.left.color
        }
    }

    /// Applies `border` to the top and bottom sides only.
    pub fn horizontal(border: BorderSlice) -> Self {
        Self {
            top: border,
            bottom: border,
            ..Default::default()
        }
    }

    /// Applies `border` to the left and right sides only.
    pub fn vertical(border: BorderSlice) -> Self {
        Self {
            left: border,
            right: border,
            ..Default::default()
        }
    }

    /// Returns the resolved outline stroke for each side: (left, top, right,
    /// bottom).
    pub fn strokes(&self, box_width: f32, box_height: f32, scale: f32) -> (f32, f32, f32, f32) {
        (
            resolve_dim(self.left.stroke, box_width, scale),
            resolve_dim(self.top.stroke, box_height, scale),
            resolve_dim(self.right.stroke, box_width, scale),
            resolve_dim(self.bottom.stroke, box_height, scale),
        )
    }
}

impl Drawable for RawBoxBorder {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && ctx.parent_size.width.is_finite()
            && ctx.parent_size.height.is_finite()
            && ctx.parent_size.width >= 0.0
            && ctx.parent_size.height >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let scale = ctx.scale;
        let box_width = ctx.parent_size.width;
        let box_height = ctx.parent_size.height;
        let is_outline = self.mode == BorderMode::Outside;
        let left = resolve_dim(self.left.stroke, box_width, scale).max(0.0);
        let right = resolve_dim(self.right.stroke, box_width, scale).max(0.0);
        let top = resolve_dim(self.top.stroke, box_height, scale).max(0.0);
        let bottom = resolve_dim(self.bottom.stroke, box_height, scale).max(0.0);
        let radii = self.radius.map(|radius| radius / scale);
        let canvas = aimer_canvas::Canvas::of(ctx);
        let uniform_color = self.left.color == self.right.color
            && self.left.color == self.top.color
            && self.left.color == self.bottom.color;

        if uniform_color && self.left.style != BorderStyle::None {
            let (x, y, width, height) = if is_outline {
                (
                    -left / scale,
                    -top / scale,
                    (box_width + left + right) / scale,
                    (box_height + top + bottom) / scale,
                )
            } else {
                (0.0, 0.0, box_width / scale, box_height / scale)
            };
            canvas.fill_rect_styled(
                aimer_cupid::utilities::Rect::new(x, y, width, height),
                Color::Transparent.into(),
                radii,
                [top / scale, right / scale, bottom / scale, left / scale],
                self.left.color.into(),
                [0.0; 4],
                Color::Transparent.into(),
            );
        } else {
            let fill_side = |slice: BorderSlice, x: f32, y: f32, width: f32, height: f32| {
                if slice.style != BorderStyle::None && width > 0.0 && height > 0.0 {
                    canvas.fill_rect_styled(
                        aimer_cupid::utilities::Rect::new(
                            x / scale,
                            y / scale,
                            width / scale,
                            height / scale,
                        ),
                        slice.color.into(),
                        radii,
                        [0.0; 4],
                        Color::Transparent.into(),
                        [0.0; 4],
                        Color::Transparent.into(),
                    );
                }
            };
            if is_outline {
                fill_side(self.top, -left, -top, box_width + left + right, top);
                fill_side(
                    self.bottom,
                    -left,
                    box_height,
                    box_width + left + right,
                    bottom,
                );
                fill_side(self.left, -left, -top, left, box_height + top + bottom);
                fill_side(
                    self.right,
                    box_width,
                    -top,
                    right,
                    box_height + top + bottom,
                );
            } else {
                fill_side(self.top, 0.0, 0.0, box_width, top);
                fill_side(self.bottom, 0.0, box_height - bottom, box_width, bottom);
                fill_side(self.left, 0.0, 0.0, left, box_height);
                fill_side(self.right, box_width - right, 0.0, right, box_height);
            }
        }
        canvas.finish();
    }

    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        if self.mode != BorderMode::Outside {
            return Some([0.0; 4]);
        }
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        let left = resolve_dim(self.left.stroke, ctx.parent_size.width, scale).max(0.0);
        let right = resolve_dim(self.right.stroke, ctx.parent_size.width, scale).max(0.0);
        let top = resolve_dim(self.top.stroke, ctx.parent_size.height, scale).max(0.0);
        let bottom = resolve_dim(self.bottom.stroke, ctx.parent_size.height, scale).max(0.0);
        Some([left / scale, top / scale, right / scale, bottom / scale])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slice(stroke: f32, color: Color) -> BorderSlice {
        BorderSlice {
            style: BorderStyle::Solid,
            stroke: Dimension::Px(stroke),
            color,
        }
    }

    /// Regression for the website header: a `bottom`-only border must paint
    /// with the color set on that side, not the default (transparent)
    /// `left` color.
    #[test]
    fn effective_color_uses_bottom_only_side() {
        let border = BoxBorder {
            bottom: slice(8.0, Color::BLACK),
            ..Default::default()
        };
        // Before the fix this returned `left.color` == Transparent (invisible).
        assert_eq!(border.effective_color(200.0, 60.0, 1.0), Color::BLACK);
    }

    #[test]
    fn effective_color_prefers_left_when_visible() {
        let border = BoxBorder {
            left: slice(4.0, Color::RED),
            bottom: slice(8.0, Color::BLACK),
            ..Default::default()
        };
        assert_eq!(border.effective_color(200.0, 60.0, 1.0), Color::RED);
    }

    #[test]
    fn effective_color_falls_back_to_left_when_nothing_visible() {
        // No side has a stroke: fall back to the (default) left color.
        let border = BoxBorder::default();
        assert_eq!(border.effective_color(200.0, 60.0, 1.0), border.left.color);
    }

    #[test]
    fn effective_color_ignores_none_style_side() {
        // `left` has a stroke but style None → skip it, use the visible `top`.
        let border = BoxBorder {
            left: BorderSlice {
                style: BorderStyle::None,
                stroke: Dimension::Px(4.0),
                color: Color::RED,
            },
            top: slice(2.0, Color::BLACK),
            ..Default::default()
        };
        assert_eq!(border.effective_color(200.0, 60.0, 1.0), Color::BLACK);
    }

    #[test]
    fn outline_effective_color_uses_visible_side() {
        let outline = BoxOutline {
            right: slice(3.0, Color::BLACK),
            ..Default::default()
        };
        assert_eq!(outline.effective_color(200.0, 60.0, 1.0), Color::BLACK);
    }
}
