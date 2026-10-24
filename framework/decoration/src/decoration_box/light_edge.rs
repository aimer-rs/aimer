use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{Canvas, MaterialDrawRequest, MaterialKind};
use aimer_color::prelude::Color;

use super::{Decoration, declared_corner_radii};

/// Paints a softly blended glass highlight around the edges of a box.
///
/// The inward rim fades smoothly into the surface, follows rounded corners,
/// and can catch a configurable reflection opposite the light. Its width and brightness
/// taper as the edge turns away from the light. It samples no backdrop and adds
/// no fill, blur, refraction, shadow, animation, child inset, or child clip. Ancestor clips
/// apply. Like other decorations, it paints before the child, so opaque child
/// content can cover the highlight.
///
/// Without an explicit color, the rim chooses white or black for contrast with
/// the declared solid background of its container, ancestors, or preceding
/// `MultiDecorationBox` layers. Unknown backgrounds use white. It does not
/// sample images or gradients. [`Self::color`] overrides automatic contrast.
///
/// Corners are square unless a [`BorderRadius`](crate::BorderRadius) or
/// [`BoxDecoration`](crate::BoxDecoration) radius precedes the highlight in the
/// same stack, in which case it follows those per-corner radii, each fitted
/// within half the shorter side. A radius that follows it does not reach back.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_color::prelude::Color;
/// use aimer_decoration::{BorderRadius, Decoration, DecorationBox, LightEdge};
///
/// let highlight = DecorationBox::new()
///     .decoration(vec![
///         BorderRadius::from(20).notype(),
///         LightEdge::new().width(8.0).strength(0.8).reflection_strength(0.35).notype(),
///     ])
///     .child(SizedBox::new().width(240).height(80));
///
/// let colored = LightEdge::new().color(Color::Rgb(180, 220, 255));
/// let automatic = colored.color(None);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightEdge {
    color: Option<Color>,
    strength: f32,
    reflection_strength: f32,
    width: f32,
    light_angle: f32,
}

impl Default for LightEdge {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl LightEdge {
    /// Creates an automatically colored 8-pixel blend at 0.8 strength, lit from the upper left.
    #[inline]
    pub fn new() -> Self {
        Self {
            color: None,
            strength: 0.8,
            reflection_strength: 0.35,
            width: 8.0,
            light_angle: 225.0,
        }
    }

    /// Sets the highlight color. Its alpha multiplies the highlight strength.
    /// Transparent colors disable painting; `None` restores automatic contrast.
    #[inline]
    pub fn color(mut self, color: impl Into<Option<Color>>) -> Self {
        self.color = color.into();
        self
    }

    /// Sets highlight strength, clamped to `0.0..=1.0`.
    /// Zero disables painting; nonfinite values use the default `0.8`.
    #[inline]
    pub fn strength(mut self, strength: f32) -> Self {
        self.strength = normalize(strength, 0.8, 1.0);
        self
    }

    /// Sets the opposing reflection relative to the primary highlight.
    ///
    /// Values are clamped to `0.0..=1.0`: zero disables the reflection and one
    /// makes it as strong as the primary highlight. [`Self::strength`] and the
    /// color's alpha scale both. Nonfinite values use the default `0.35`.
    #[inline]
    pub fn reflection_strength(mut self, strength: f32) -> Self {
        self.reflection_strength = normalize(strength, 0.35, 1.0);
        self
    }

    /// Sets the maximum inward blend width in logical pixels, clamped to `0.0..=16.0`.
    /// Zero disables painting; nonfinite values use the default `8.0`.
    /// The highlight fades to transparent across this width, narrowing at grazing
    /// angles. At paint time width is limited to half the shorter side.
    #[inline]
    pub fn width(mut self, width: f32) -> Self {
        self.width = normalize(width, 8.0, 16.0);
        self
    }

    /// Sets light direction in degrees clockwise from screen-right.
    /// `90.0` lights from below; the default `225.0` lights from the upper
    /// left. Finite angles wrap into `0.0..360.0`; nonfinite values use `225.0`.
    #[inline]
    pub fn light_angle(mut self, angle: f32) -> Self {
        self.light_angle = if angle.is_finite() {
            angle.rem_euclid(360.0)
        } else {
            225.0
        };
        self
    }
}

impl Decoration for LightEdge {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        if size.width <= 0.0 || size.height <= 0.0 || self.is_empty() {
            return;
        }
        let limit = size.width.min(size.height) * 0.5;
        let mut request = MaterialDrawRequest::from_layout(MaterialKind::LightEdge, origin, size);
        let color = self
            .color
            .unwrap_or_else(|| contrast_color(canvas.background_color()));
        request.border_color = aimer_cupid::utilities::Color::from(color).to_array();
        request.border_width = self.width.min(limit);
        request.specular_highlight = self.strength;
        request.light_angle = self.light_angle;
        request.corner_radii = declared_corner_radii(canvas, limit.min(MaterialDrawRequest::MAX_CORNER_RADIUS));
        request.opacity = 1.0;
        request.tint = [0.0; 4];
        request.blur_radius = 0.0;
        request.shadow_color = [0.0; 4];
        request.shadow_blur = 0.0;
        request.elevation = 0.0;
        request.distortion_strength = 0.0;
        request.edge_lighting = self.reflection_strength;
        request.animation_speed = 0.0;
        request.chromatic_aberration = 0.0;
        canvas.draw_material(request);
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.width == 0.0
            || self.strength == 0.0
            || self.color.is_some_and(|color| color.as_u32() >> 24 == 0)
    }
}

fn contrast_color(background: Color) -> Color {
    let [alpha, r, g, b] = background.as_u32().to_be_bytes();
    if alpha == 0 {
        return Color::WHITE;
    }
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    // Equal white/black contrast occurs at sqrt(1.05 * 0.05) - 0.05.
    if luminance > 0.17912878 {
        Color::BLACK
    } else {
        Color::WHITE
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
