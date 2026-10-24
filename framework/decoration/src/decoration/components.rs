use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_cupid::utilities::Rect;

use crate::{BorderRadius, BoxBorder, BoxDecoration, BoxOutline, BoxShadow, Decoration};

/// Fills the box with this color without adding child insets or clipping.
/// Even a transparent color supplies an explicit background and overrides a
/// container's fallback color. Later decoration layers inherit this background.
///
/// The fill takes the shape earlier layers declared: after a [`BorderRadius`] in a
/// list the corners are rounded, bounded to half the box. With no declared shape
/// the fill is square.
///
/// ```
/// use aimer_color::prelude::Color;
/// use aimer_container::SizedBox;
/// use aimer_decoration::DecorationBox;
///
/// let widget = DecorationBox::new().decoration(Color::BLUE)
///     .child(SizedBox::new().width(80).height(40));
/// ```
impl Decoration for Color {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        // Radii are declared in logical pixels, the space the rect is in. Half the
        // shorter side is as round as a box can get; a larger declaration is read
        // as that.
        let radii = crate::decoration_box::declared_corner_radii(
            canvas,
            (size.width.min(size.height) / 2.0).max(0.0),
        );
        canvas.fill_rect_styled(
            Rect::new(origin.x, origin.y, size.width, size.height),
            (*self).into(),
            radii,
            [0.0; 4],
            Color::Transparent.into(),
            [0.0; 4],
            Color::Transparent.into(),
        );
    }

    #[inline]
    fn background_color(&self) -> Option<Color> {
        Some(*self)
    }

    #[inline]
    fn revision(&self) -> u64 {
        u64::from(self.as_u32()) + 1
    }
}

/// Paints the border inside the box and reserves its widths as child insets.
/// The border alone does not clip the child or supply a background color.
impl Decoration for BoxBorder {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        BoxDecoration::new().border(*self).paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        BoxDecoration::new().border(*self)
            .paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, _scale: f32) -> [f32; 4] {
        let (left, top, right, bottom) = self.strokes(size.width, size.height, 1.0);
        [left, top, right, bottom].map(|width| width.max(0.0))
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Paints outside the box without changing child insets or clipping.
impl Decoration for BoxOutline {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        BoxDecoration::new().outline(*self).paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        BoxDecoration::new().outline(*self)
            .paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        if !scale.is_finite() || scale <= 0.0 {
            return [0.0; 4];
        }
        let (left, top, right, bottom) = self.strokes(size.width * scale, size.height * scale, scale);
        [left / scale, top / scale, right / scale, bottom / scale]
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Paints a rectangular shadow without changing child geometry.
/// Outer shadows precede a fallback background; inset shadows follow it.
impl Decoration for BoxShadow {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        self.record_local_v2(
            canvas, Rect::new(origin.x, origin.y, size.width, size.height), &[0.0; 4], scale,
        );
    }

    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        if !self.inset {
            self.paint(canvas, origin, size, scale);
        }
        if let Some(background) = background {
            canvas.fill_color_rect(origin, size, background);
        }
        if self.inset {
            self.paint(canvas, origin, size, scale);
        }
    }

    #[inline]
    fn paint_outsets(&self, _size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.local_v2_paint_outsets(scale)
    }
}

/// Supplies rounded child clipping and shapes a container's fallback background.
/// With no fallback background this records no paint. Even zero radii retain
/// the rectangular child clip. Paint-only decoration wrappers do not clip children.
///
/// It also declares the box shape for the layers that follow it in a stack:
/// [`Magnification`](crate::Magnification) and [`LightEdge`](crate::LightEdge)
/// round their corners to the nearest preceding radius.
impl Decoration for BorderRadius {
    #[inline]
    fn paint(&self, canvas: &Canvas, _origin: Vec2d, size: ResolvedSize, scale: f32) {
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        canvas.set_corner_radii(
            self.resolve(size.width * scale, size.height * scale, scale)
                .map(|radius| radius / scale),
        );
    }

    #[inline]
    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32,
        background: Option<Color>,
    ) {
        // A zero-valued default radius is not forwarded as a shape, so declare it here.
        self.paint(canvas, origin, size, scale);
        BoxDecoration::new().border_radius(*self)
            .paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        Some(self.resolve(size.width * scale, size.height * scale, scale)
            .map(|radius| (radius / scale).max(0.0)))
    }
}
