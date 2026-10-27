//! [`Magnification`], a decoration that enlarges content painted behind it.

use aimer_attribute::{ResolvedSize, Vec2d};
pub use aimer_canvas::MagnificationStyle;
use aimer_canvas::{Canvas, MaterialDrawRequest, MaterialKind};

use super::{Decoration, declared_corner_radii};

/// Enlarges content already painted behind a [`super::DecorationBox`].
///
/// By default zoom is centered on the box and thickness adds edge refraction.
/// Select optical effects, axes, and anchors with [`Self::style`]; bulge adds
/// a separately controlled swell around the enlargement anchor. The decoration
/// adds no tint, blur, border, shadow, or highlight. It affects neither child
/// layout nor child clipping, and the child paints sharply after the lens.
/// Ancestor clips still apply.
///
/// The lens has square corners unless a [`BorderRadius`](crate::BorderRadius)
/// or [`BoxDecoration`](crate::BoxDecoration) radius precedes it in the same
/// stack, in which case it adopts those per-corner radii, bounded to
/// `0.0..=1024.0`. A radius that follows the lens does not reach back. If capture is unavailable or exceeds the
/// renderer's budget, the backdrop remains unchanged.
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BorderRadius, Decoration, DecorationBox, Magnification};
/// let lens = DecorationBox::new()
///     .decoration(vec![
///         BorderRadius::from(16).notype(),
///         Magnification::new().zoom(2.0).thickness(16.0).notype(),
///     ])
///     .child(SizedBox::new().width(240).height(160));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnification {
    zoom: f32,
    thickness: f32,
    style: MagnificationStyle,
    bulge: f32,
}

impl Default for Magnification {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Magnification {
    /// Creates a centered 1.4× lens with 16-pixel thickness.
    /// The default style enables uniform zoom and refraction on both axes.
    #[inline]
    pub fn new() -> Self {
        Self {
            zoom: 1.4,
            thickness: 16.0,
            style: MagnificationStyle::DEFAULT,
            bulge: 0.25,
        }
    }

    /// Replaces the effects, axes, and anchors with `style`.
    ///
    /// Combine flags with `|`. No axis flags means both axes; opposing anchors
    /// resolve to center. Axes constrain every effect, while anchors move
    /// uniform zoom and bulge. At least one optical effect must be selected.
    ///
    /// ```
    /// use aimer_decoration::{Magnification, MagnificationStyle as Style};
    /// let toolbar = Magnification::new()
    ///     .style(Style::UNIFORM | Style::HORIZONTAL | Style::LEFT)
    ///     .zoom(1.2);
    /// ```
    #[inline]
    pub fn style(mut self, style: MagnificationStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets additional bulge strength, clamped to `0.0..=1.0`.
    ///
    /// The default and nonfinite fallback are `0.25`. Requires the `BULGE`
    /// style flag. Zero disables bulge; one doubles the enlargement at the
    /// anchor, independently of uniform zoom. The swell fades smoothly toward
    /// the furthest box edges along the enabled axes.
    #[inline]
    pub fn bulge(mut self, strength: f32) -> Self {
        self.bulge = normalize(strength, 0.25, 1.0);
        self
    }

    /// Sets the enlargement factor, clamped to `1.0..=8.0`.
    /// Nonfinite values use `1.5`. Requires the `UNIFORM` style flag; a zoom of
    /// one still permits independently enabled bulge and edge refraction.
    #[inline]
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = if zoom.is_finite() {
            zoom.clamp(1.0, 8.0)
        } else {
            1.5
        };
        self
    }

    /// Sets glass depth in logical pixels, clamped to `0.0..=512.0`.
    /// Nonfinite values use `16.0`. Requires the `REFRACTION` style flag.
    /// Zero disables edge refraction. At paint
    /// time depth is limited to half the box's shorter side. The bevel width
    /// fits the rounded corners independently of this optical depth.
    #[inline]
    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = normalize(thickness, 16.0, 512.0);
        self
    }
}

impl Decoration for Magnification {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        let active = (self.style.contains(MagnificationStyle::UNIFORM) && self.zoom != 1.0)
            || (self.style.contains(MagnificationStyle::BULGE) && self.bulge > 0.0)
            || (self.style.contains(MagnificationStyle::REFRACTION) && self.thickness > 0.0);
        if size.width <= 0.0 || size.height <= 0.0 || !active {
            return;
        }
        let mut request =
            MaterialDrawRequest::from_layout(MaterialKind::Magnification, origin, size);
        request.magnification = self.zoom;
        request.bevel_radius = self.thickness;
        request.magnification_style = self.style;
        request.bulge_strength = self.bulge;
        request.corner_radii =
            declared_corner_radii(canvas, MaterialDrawRequest::MAX_CORNER_RADIUS);
        request.blur_radius = 0.0;
        request.tint = [0.0; 4];
        request.opacity = 1.0;
        request.border_color = [0.0; 4];
        request.border_width = 0.0;
        request.shadow_color = [0.0; 4];
        request.shadow_blur = 0.0;
        request.elevation = 0.0;
        request.edge_lighting = 0.0;
        request.specular_highlight = 0.0;
        request.distortion_strength = 0.0;
        request.animation_speed = 0.0;
        request.chromatic_aberration = 0.0;
        canvas.draw_material(request);
    }
}

#[inline]
fn normalize(value: f32, fallback: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, max)
    } else {
        fallback
    }
}
