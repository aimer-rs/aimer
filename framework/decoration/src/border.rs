use aimer_attribute::dimension::Dimension;
use aimer_color::prelude::Color;
use aimer_widget::Drawable;
use aimer_widget::base::BuildContext;

#[allow(dead_code)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum BorderStyle {
    Solid,
    Dashed,
    Dotted,
    #[default]
    None,
}

#[allow(dead_code)]
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum BorderMode {
    #[default]
    Inside,
    Outside,
}

pub type Stroke = Dimension;

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BorderSlice {
    pub style: BorderStyle,
    pub stroke: Stroke,
    pub color: Color,
}

impl BorderSlice {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn style(mut self, style: BorderStyle) -> Self {
        self.style = style;
        self
    }

    #[inline]
    pub fn stroke(mut self, stroke: impl Into<Stroke>) -> Self {
        self.stroke = stroke.into();
        self
    }

    #[inline]
    pub fn color(mut self, color: impl Into<Color>) -> Self {
        self.color = color.into();
        self
    }
}

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
    pub left: BorderSlice,
    pub right: BorderSlice,
    pub top: BorderSlice,
    pub bottom: BorderSlice,
}

/// An outside outline usable directly as a [`crate::Decoration`].
/// Its paint outsets include the stroke widths; child geometry is unchanged.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BoxOutline {
    pub left: BorderSlice,
    pub right: BorderSlice,
    pub top: BorderSlice,
    pub bottom: BorderSlice,
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct RawBoxBorder {
    pub left: BorderSlice,
    pub right: BorderSlice,
    pub top: BorderSlice,
    pub bottom: BorderSlice,
    pub mode: BorderMode,
    pub radius: [f32; 4],
}

impl RawBoxBorder {
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
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn left(mut self, left: BorderSlice) -> Self {
        self.left = left;
        self
    }

    #[inline]
    pub fn right(mut self, right: BorderSlice) -> Self {
        self.right = right;
        self
    }

    #[inline]
    pub fn top(mut self, top: BorderSlice) -> Self {
        self.top = top;
        self
    }

    #[inline]
    pub fn bottom(mut self, bottom: BorderSlice) -> Self {
        self.bottom = bottom;
        self
    }

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

    #[inline]
    pub fn horizontal(border: BorderSlice) -> Self {
        Self {
            top: border,
            bottom: border,
            ..Default::default()
        }
    }

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
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn left(mut self, left: BorderSlice) -> Self {
        self.left = left;
        self
    }

    #[inline]
    pub fn right(mut self, right: BorderSlice) -> Self {
        self.right = right;
        self
    }

    #[inline]
    pub fn top(mut self, top: BorderSlice) -> Self {
        self.top = top;
        self
    }

    #[inline]
    pub fn bottom(mut self, bottom: BorderSlice) -> Self {
        self.bottom = bottom;
        self
    }

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

    pub fn horizontal(border: BorderSlice) -> Self {
        Self {
            top: border,
            bottom: border,
            ..Default::default()
        }
    }

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
