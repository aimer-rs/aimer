pub(crate) mod border_radius;
pub(crate) mod box_shadow;
pub(crate) mod shapes;

use aimer_attribute::position::Vec2d;
use aimer_color::prelude::Color;
use std::cell::Cell;

use crate::border::{BoxBorder, BoxOutline};
use crate::box_decoration::border_radius::BorderRadius;
use crate::box_decoration::box_shadow::BoxShadow;

/// A box background with borders, outlines, rounded corners, and shadows.
///
/// Containers apply its border widths as child insets and clip children to the
/// inner rounded box. [`crate::DecorationBox`] uses only its painting behavior.
/// Background color can change through [`BoxDecoration::update_color`]; other
/// geometry changes require rebuilding the owning widget.
#[derive(Default, Clone, PartialEq, Debug)]
pub struct BoxDecoration {
    pub border: BoxBorder,
    pub outline: BoxOutline,
    pub border_radius: BorderRadius,
    pub box_shadow: Vec<BoxShadow>,
    pub background_color: Cell<Option<Color>>,
}

impl From<BoxShadow> for Vec<BoxShadow> {
    fn from(shadow: BoxShadow) -> Self {
        vec![shadow]
    }
}

impl BoxDecoration {


    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn border(mut self, border: BoxBorder) -> Self {
        self.border = border;
        self
    }

    #[inline]
    pub fn outline(mut self, outline: BoxOutline) -> Self {
        self.outline = outline;
        self
    }

    #[inline]
    pub fn border_radius(mut self, border_radius: impl Into<BorderRadius>) -> Self {
        self.border_radius = border_radius.into();
        self
    }

    #[inline]
    pub fn box_shadow(mut self, box_shadow: impl IntoIterator<Item = BoxShadow>) -> Self {
        self.box_shadow = box_shadow.into_iter().collect();
        self
    }

    #[inline]
    pub fn add_shadow(mut self, shadow: BoxShadow) -> Self {
        self.box_shadow.push(shadow);
        self
    }

    #[inline]
    pub fn background_color(self, background_color: impl Into<Color>) -> Self {
        self.background_color.set(Some(background_color.into()));
        self
    }

    #[inline]
    pub fn update_color(&self, new_color: impl Into<Color>) {
        self.background_color.set(Some(new_color.into()));
    }

    /// Records this decoration into an element-local retained V2 list.
    #[doc(hidden)]
    pub fn record_local_v2(
        &self,
        canvas: &aimer_canvas::Canvas,
        position: Vec2d,
        box_width: f32,
        box_height: f32,
        scale: f32,
    ) {
        self.record_local_v2_with_background(canvas, position, box_width, box_height, scale, None);
    }

    pub(crate) fn record_local_v2_with_background(
        &self,
        canvas: &aimer_canvas::Canvas,
        position: Vec2d,
        box_width: f32,
        box_height: f32,
        scale: f32,
        background: Option<Color>,
    ) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }

        let logical_width = box_width / scale;
        let logical_height = box_height / scale;
        let radii = self
            .border_radius
            .resolve(box_width, box_height, scale)
            .map(|radius| radius / scale);
        let rect = aimer_cupid::utilities::Rect::new(
            position.x,
            position.y,
            logical_width,
            logical_height,
        );

        // Only an explicit radius declares the box shape. Border, outline and
        // shadow decorations delegate here with a default radius and must not
        // square off a shape an earlier layer declared.
        if self.border_radius != BorderRadius::default() {
            canvas.set_corner_radii(radii);
        }

        for shadow in &self.box_shadow {
            if !shadow.inset {
                shadow.record_local_v2(canvas, rect, &radii, scale);
            }
        }

        if self.border.has_visible_border(box_width, box_height, scale)
            || self
                .outline
                .has_visible_outline(box_width, box_height, scale)
        {
            let border = self.border.strokes(box_width, box_height, scale);
            let outline = self.outline.strokes(box_width, box_height, scale);
            canvas.fill_rect_styled(
                rect,
                self.background_color
                    .get()
                    .or(background)
                    .unwrap_or(Color::Transparent)
                    .into(),
                radii,
                [border.1, border.2, border.3, border.0].map(|width| width / scale),
                self.border
                    .effective_color(box_width, box_height, scale)
                    .into(),
                [outline.1, outline.2, outline.3, outline.0].map(|width| width / scale),
                self.outline
                    .effective_color(box_width, box_height, scale)
                    .into(),
            );
        } else if let Some(color) = self.background_color.get().or(background) {
            canvas.fill_rect_styled(
                rect,
                color.into(),
                radii,
                [0.0; 4],
                Color::Transparent.into(),
                [0.0; 4],
                Color::Transparent.into(),
            );
        }

        for shadow in &self.box_shadow {
            if shadow.inset {
                shadow.record_local_v2(canvas, rect, &radii, scale);
            }
        }
    }

    /// Returns logical paint outsets for outlines and outer shadows.
    #[doc(hidden)]
    pub fn local_v2_paint_outsets(&self, box_width: f32, box_height: f32, scale: f32) -> [f32; 4] {
        if !scale.is_finite() || scale <= 0.0 {
            return [0.0; 4];
        }
        let outline = self.outline.strokes(box_width, box_height, scale);
        let mut outsets = [
            outline.0 / scale,
            outline.1 / scale,
            outline.2 / scale,
            outline.3 / scale,
        ];
        for shadow in self.box_shadow.iter().filter(|shadow| !shadow.inset) {
            let shadow_outsets = shadow.local_v2_paint_outsets(scale);
            outsets[0] = outsets[0].max(shadow_outsets[0]);
            outsets[1] = outsets[1].max(shadow_outsets[1]);
            outsets[2] = outsets[2].max(shadow_outsets[2]);
            outsets[3] = outsets[3].max(shadow_outsets[3]);
        }
        outsets
    }
}

impl BoxDecoration {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::border::BorderSlice;

    /// A decoration is copied into every element that carries one, so its width
    /// is a per-node cost on every build. Eight of its bytes-per-color slots
    /// come from the four border and four outline sides, which is why a color
    /// is stored packed: at four bytes per color a slice fits in two words and
    /// the whole decoration in 192 bytes, where a color carrying HSLA
    /// components would have made it 336.
    ///
    /// These numbers are asserted rather than described so that widening a
    /// color, a stroke or a radius shows up here instead of in a frame budget.
    #[test]
    fn a_decoration_stores_its_colors_packed() {
        assert_eq!(size_of::<Color>(), 4);
        assert_eq!(size_of::<BorderSlice>(), 16);
        assert_eq!(size_of::<BoxBorder>(), 4 * size_of::<BorderSlice>());
        assert_eq!(size_of::<BoxOutline>(), 4 * size_of::<BorderSlice>());
        assert_eq!(size_of::<BoxDecoration>(), 192);
    }

    /// Every color model resolves to the same packed value, so a decoration
    /// written two different ways is the same decoration — which `PartialEq`
    /// now reports, and which change detection and theme interpolation rely on.
    #[test]
    fn decorations_written_differently_compare_equal() {
        let named =
            BoxDecoration::new().background_color(Color::Basic(aimer_color::prelude::Colors::Red));
        let hexadecimal = BoxDecoration::new().background_color(Color::Hex(0xFF0000));

        assert_eq!(named, hexadecimal);
    }
}
