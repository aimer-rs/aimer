use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, MaterialDrawRequest, MaterialKind};

use super::{Decoration, declared_corner_radii};

/// Filters content already painted behind a [`super::DecorationBox`].
///
/// The filter covers the box's bounds and leaves child layout and painting
/// unchanged. Its corners take the nearest preceding
/// [`BorderRadius`](crate::BorderRadius) or [`BoxDecoration`](crate::BoxDecoration)
/// radius in the same decoration stack, so glass is `[BorderRadius, BackdropFilter]`;
/// a radius that follows the filter does not reach back, and with none the
/// corners are square. Ancestor canvas clips apply to the filtered region.
/// It adds no tint, border, shadow, highlight, or refraction. Blur uses a
/// separable Gaussian kernel with a standard deviation of one third of the
/// radius, truncated at the radius.
///
/// Saturation, contrast, and brightness are applied in that order. Each defaults
/// to `1.0`, which preserves that aspect of the backdrop. With zero blur and
/// these defaults the decoration records nothing. If backdrop capture is
/// unavailable or exceeds the renderer's budget, the backdrop remains unchanged.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BackdropFilter, DecorationBox};
///
/// let panel = DecorationBox::new()
///     .decoration(BackdropFilter::new().blur_radius(16.0).saturation(0.5))
///     .child(SizedBox::new().width(240).height(80));
/// ```
///
/// A rounded panel declares its shape first:
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BackdropFilter, BorderRadius, Decoration, DecorationBox};
///
/// let glass = DecorationBox::new()
///     .decoration([BorderRadius::from(26).notype(), BackdropFilter::new().notype()])
///     .child(SizedBox::new().width(240).height(52));
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackdropFilter {
    blur_radius: f32,
    saturation: f32,
    brightness: f32,
    contrast: f32,
}

impl Default for BackdropFilter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl BackdropFilter {
    /// Default blur radius in logical pixels.
    pub const DEFAULT_BLUR_RADIUS: f32 = 16.0;
    /// Maximum supported blur radius in logical pixels.
    pub const MAX_BLUR_RADIUS: f32 = MaterialDrawRequest::MAX_BLUR_RADIUS;

    /// Creates a filter with a 16-pixel blur and unchanged backdrop colors.
    #[inline]
    pub fn new() -> Self {
        Self {
            blur_radius: Self::DEFAULT_BLUR_RADIUS,
            saturation: 1.0,
            brightness: 1.0,
            contrast: 1.0,
        }
    }

    /// Sets the blur radius in logical pixels, clamped to `0.0..=96.0`.
    ///
    /// Nonfinite values use the default radius of `16.0`. Zero disables blur;
    /// configured color controls still apply.
    #[inline]
    pub fn blur_radius(mut self, radius: f32) -> Self {
        self.blur_radius = normalize(radius, Self::DEFAULT_BLUR_RADIUS, Self::MAX_BLUR_RADIUS);
        self
    }

    /// Sets saturation, clamped to `0.0..=3.0`.
    ///
    /// Zero produces grayscale; `1.0` preserves saturation. Nonfinite values
    /// use `1.0`.
    #[inline]
    pub fn saturation(mut self, saturation: f32) -> Self {
        self.saturation = normalize(saturation, 1.0, 3.0);
        self
    }

    /// Sets brightness, clamped to `0.0..=3.0`.
    ///
    /// Zero produces black; `1.0` preserves brightness. Nonfinite values
    /// use `1.0`.
    #[inline]
    pub fn brightness(mut self, brightness: f32) -> Self {
        self.brightness = normalize(brightness, 1.0, 3.0);
        self
    }

    /// Sets contrast, clamped to `0.0..=3.0`, around the color midpoint `0.5`.
    ///
    /// Zero produces a uniform midpoint before brightness is applied; `1.0`
    /// preserves contrast. Nonfinite values use `1.0`.
    #[inline]
    pub fn contrast(mut self, contrast: f32) -> Self {
        self.contrast = normalize(contrast, 1.0, 3.0);
        self
    }
}

impl Decoration for BackdropFilter {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        if size.width <= 0.0 || size.height <= 0.0 {
            return;
        }
        if self.blur_radius == 0.0
            && self.saturation == 1.0
            && self.brightness == 1.0
            && self.contrast == 1.0
        {
            return;
        }
        let mut request =
            MaterialDrawRequest::from_layout(MaterialKind::BackdropFilter, origin, size);
        request.blur_radius = self.blur_radius;
        request.saturation = self.saturation;
        request.brightness = self.brightness;
        request.contrast = self.contrast;
        request.tint = [0.0; 4];
        request.opacity = 1.0;
        request.border_color = [0.0; 4];
        request.border_width = 0.0;
        request.corner_radii =
            declared_corner_radii(canvas, MaterialDrawRequest::MAX_CORNER_RADIUS);
        request.shadow_color = [0.0; 4];
        request.shadow_blur = 0.0;
        request.elevation = 0.0;
        request.edge_lighting = 0.0;
        request.specular_highlight = 0.0;
        request.distortion_strength = 0.0;
        request.animation_speed = 0.0;
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
