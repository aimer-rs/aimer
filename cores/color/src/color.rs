pub mod basic_color;

use basic_color::Colors;
use std::fmt;

/// The form a color is stored and drawn in: one machine word of packed ARGB.
///
/// The channels are laid out as `0xAARRGGBB`, the order every Aimer paint
/// command and GPU pipeline already expects, so a stored color reaches the
/// renderer without a conversion.
///
/// This is the *storage* half of the color vocabulary. [`Color`] is the
/// *authoring* half: it names the ways a human writes a color down — RGB,
/// hexadecimal, grayscale, HSL, a value from [`Colors`] — and resolves each of
/// them to a `PrimitiveColor` at construction time. A retained widget or
/// element therefore never carries an unresolved color description, and a style
/// struct pays four bytes per color instead of the twenty an
/// HSLA-carrying enum would cost.
///
/// # Examples
///
/// ```
/// use aimer_color::prelude::{Color, PrimitiveColor};
///
/// let packed: PrimitiveColor = Color::Rgb(0x11, 0x22, 0x33).as_u32();
/// assert_eq!(packed, 0xFF112233);
/// assert_eq!(Color::from_primitive(packed), Color::Hex(0x112233));
/// ```
pub type PrimitiveColor = u32;

/// A color, stored as a single packed [`PrimitiveColor`].
///
/// `Color` is written the way a color is thought about — explicit RGB/RGBA
/// channels, a packed hexadecimal literal, a grayscale value, HSL/HSLA
/// components, or a named color from [`Colors`] — and each of those forms is
/// resolved to packed ARGB the moment it is constructed. The value that ends up
/// in a `BoxDecoration`, a `TextStyle` or a draw command is therefore always
/// four bytes wide and always ready for the renderer.
///
/// Because the representation is canonical, two colors written differently but
/// meaning the same thing compare equal, and `Color` is [`Eq`] and [`Hash`]:
///
/// ```
/// use aimer_color::prelude::Color;
///
/// assert_eq!(Color::Rgb(255, 0, 0), Color::Hex(0xFF0000));
/// assert_eq!(Color::Rgb(255, 0, 0), Color::Basic(aimer_color::prelude::Colors::Red));
/// ```
///
/// The trade the canonical form makes is that a color does not remember *how*
/// it was written: a named color is indistinguishable from its RGBA value once
/// constructed, and an HSL color is stored as the RGB it resolves to.
///
/// # Constructors
///
/// The constructors are deliberately named after the color models they accept
/// rather than in `snake_case`, so a color reads as one vocabulary regardless of
/// the model it came from, and so a call site written against the older
/// representation keeps its shape:
///
/// ```
/// use aimer_color::prelude::Color;
///
/// let from_channels = Color::Rgba(0x0A, 0x14, 0x1E, 0x80);
/// let from_hex = Color::HexA(0x0A141E80);
/// let from_hsl = Color::Hsl(0.0, 1.0, 0.5);
///
/// assert_eq!(from_channels, from_hex);
/// assert_eq!(from_hsl, Color::RED);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(transparent)]
pub struct Color(PrimitiveColor);

impl Color {
    pub const RED: Self = Color::Hex(0xFFFF0000);
    pub const GREEN: Self = Color::Hex(0xFF00FF00);
    pub const BLUE: Self = Color::Hex(0xFF0000FF);
    pub const WHITE: Self = Color::Hex(0xFFFFFFFF);
    pub const BLACK: Self = Color::Hex(0xFF000000);
    pub const YELLOW: Self = Color::Hex(0xFFFFFF00);
    pub const CYAN: Self = Color::Hex(0xFF00FFFF);
    pub const MAGENTA: Self = Color::Hex(0xFFFF00FF);
    pub const GRAY: Self = Color::Hex(0xFF808080);
    pub const ORANGE: Self = Color::Hex(0xFFFFA500);
    pub const PURPLE: Self = Color::Hex(0xFF800080);
    pub const BROWN: Self = Color::Hex(0xFFA52A2A);

    /// A fully transparent color.
    ///
    /// Every channel is zero, which is also [`Color::default`].
    #[allow(non_upper_case_globals)]
    pub const Transparent: Self = Color(0x00000000);
}

/// The color models a [`Color`] can be written in.
///
/// Each constructor resolves its arguments to packed ARGB immediately, so they
/// are interchangeable: the result carries no trace of the model it came from.
#[allow(non_snake_case)]
impl Color {
    /// Red, green, blue and alpha channels, each `0..=255`.
    #[inline]
    pub const fn Rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Red, green and blue channels, fully opaque.
    #[inline]
    pub const fn Rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgba(r, g, b, 0xFF)
    }

    /// A packed `0xRRGGBB` literal, fully opaque.
    ///
    /// Anything above the low three bytes is ignored, so both `0xFF0000` and
    /// `0xFFFF0000` describe red.
    #[inline]
    pub const fn Hex(rgb: u32) -> Self {
        Color(0xFF000000 | (rgb & 0x00FFFFFF))
    }

    /// A packed `0xRRGGBBAA` literal, alpha last.
    #[inline]
    pub const fn HexA(rgba: u32) -> Self {
        Color(rgba.rotate_right(8))
    }

    /// One gray level applied to all three channels, plus an alpha channel.
    #[inline]
    pub const fn Grayscale(value: u8, a: u8) -> Self {
        Self::Rgba(value, value, value, a)
    }

    /// One gray level applied to all three channels, fully opaque.
    #[inline]
    pub const fn Gray8(value: u8) -> Self {
        Self::Grayscale(value, 0xFF)
    }

    /// Hue in `0..360`, saturation and lightness in `0.0..=1.0`, fully opaque.
    #[inline]
    pub const fn Hsl(h: f32, s: f32, l: f32) -> Self {
        let (r, g, b) = Self::hsl_to_rgb(h, s, l);

        Self::Rgb(r, g, b)
    }

    /// Hue in `0..360`, saturation, lightness and alpha in `0.0..=1.0`.
    #[inline]
    pub const fn Hsla(h: f32, s: f32, l: f32, a: f32) -> Self {
        let (r, g, b) = Self::hsl_to_rgb(h, s, l);

        Self::Rgba(r, g, b, float_to_channel(a * 255.0))
    }

    /// A named color from the [`Colors`] palette.
    #[inline]
    pub const fn Basic(named: Colors) -> Self {
        Color(named.as_u32())
    }
}

impl Color {
    /// Wraps an already packed ARGB value.
    ///
    /// This is the inverse of [`Color::as_u32`] and the entry point for a value
    /// that arrives from a file, a platform API or a GPU buffer already in
    /// `0xAARRGGBB` order.
    #[inline]
    pub const fn from_primitive(primitive: PrimitiveColor) -> Self {
        Color(primitive)
    }

    /// Returns the red channel, `0..=255`.
    #[inline]
    pub const fn red(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    /// Returns the green channel, `0..=255`.
    #[inline]
    pub const fn green(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    /// Returns the blue channel, `0..=255`.
    #[inline]
    pub const fn blue(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Returns the alpha channel, `0..=255`, where `0` is fully transparent.
    #[inline]
    pub const fn alpha(self) -> u8 {
        ((self.0 >> 24) & 0xFF) as u8
    }

    /// Returns the four channels in `(red, green, blue, alpha)` order.
    ///
    /// This replaces destructuring the color, which the packed representation
    /// no longer allows.
    #[inline]
    pub const fn to_rgba(self) -> (u8, u8, u8, u8) {
        self.to_rgba_components()
    }

    pub const fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let m = l - c / 2.0;

        let (r_prime, g_prime, b_prime) = if h < 60.0 {
            (c, x, 0.0)
        } else if h < 120.0 {
            (x, c, 0.0)
        } else if h < 180.0 {
            (0.0, c, x)
        } else if h < 240.0 {
            (0.0, x, c)
        } else if h < 300.0 {
            (x, 0.0, c)
        } else {
            (c, 0.0, x)
        };

        (
            ((r_prime + m) * 255.0).round() as u8,
            ((g_prime + m) * 255.0).round() as u8,
            ((b_prime + m) * 255.0).round() as u8,
        )
    }

    /// Returns the packed ARGB value this color is stored as.
    ///
    /// The result is a [`PrimitiveColor`] in `0xAARRGGBB` order and the call is
    /// a plain field read: the packing happened when the color was constructed.
    #[inline]
    pub const fn as_u32(&self) -> PrimitiveColor {
        self.0
    }

    /// Returns this color with its alpha channel replaced by `opacity`.
    ///
    /// `opacity` is interpreted as an 8-bit alpha value, where `0` is fully
    /// transparent and `255` is fully opaque. The color channels are preserved
    /// exactly.
    #[inline]
    pub const fn with_opacity(self, opacity: u8) -> Self {
        Color((self.0 & 0x00FFFFFF) | ((opacity as u32) << 24))
    }

    /// Scales the brightness of this color by `strength`.
    ///
    /// This is equivalent to [`Color::multiply`]: RGB channels are multiplied
    /// by `strength`, clamped to valid channel values, and the alpha
    /// channel is preserved.
    pub const fn with_brightness(self, strength: f32) -> Self {
        self.multiply(strength)
    }

    /// Darkens this color by moving its RGB channels toward black.
    ///
    /// `amount` is clamped to `0.0..=1.0`, where `0.0` returns the original
    /// color and `1.0` returns black with the original alpha.
    pub const fn darken(self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        self.multiply(1.0 - amount)
    }

    /// Lightens this color by moving its RGB channels toward white.
    ///
    /// `amount` is clamped to `0.0..=1.0`, where `0.0` returns the original
    /// color and `1.0` returns white with the original alpha.
    pub const fn lighten(self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let (r, g, b, a) = self.to_rgba_components();

        Self::from_rgba_components(
            lerp_channel(r, 255, amount),
            lerp_channel(g, 255, amount),
            lerp_channel(b, 255, amount),
            a,
        )
    }

    /// Returns this color with its alpha channel replaced by `alpha`.
    ///
    /// `alpha` is a normalized value clamped to `0.0..=1.0`, where `0.0` is
    /// fully transparent and `1.0` is fully opaque.
    pub const fn with_alpha(self, alpha: f32) -> Self {
        let (r, g, b, _) = self.to_rgba_components();

        Self::from_rgba_components(r, g, b, float_to_channel(alpha.clamp(0.0, 1.0) * 255.0))
    }

    /// Multiplies this color's RGB channels by `factor`.
    ///
    /// Negative factors are treated as `0.0`, resulting channels are clamped to
    /// `0..=255`, and the alpha channel is preserved.
    pub const fn multiply(self, factor: f32) -> Self {
        let factor = factor.max(0.0);
        let (r, g, b, a) = self.to_rgba_components();

        Self::from_rgba_components(
            float_to_channel(r as f32 * factor),
            float_to_channel(g as f32 * factor),
            float_to_channel(b as f32 * factor),
            a,
        )
    }

    /// Blends this color toward `other` by the interpolation value `t`.
    ///
    /// This is an alias for [`Color::lerp`].
    pub const fn blend(self, other: Color, t: f32) -> Self {
        self.lerp(other, t)
    }

    /// Linearly interpolates between this color and `other`.
    ///
    /// `t` is clamped to `0.0..=1.0`, where `0.0` returns this color and `1.0`
    /// returns `other`. RGB and alpha channels are interpolated independently.
    pub const fn lerp(self, other: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let (r1, g1, b1, a1) = self.to_rgba_components();
        let (r2, g2, b2, a2) = other.to_rgba_components();

        Self::from_rgba_components(
            lerp_channel(r1, r2, t),
            lerp_channel(g1, g2, t),
            lerp_channel(b1, b2, t),
            lerp_channel(a1, a2, t),
        )
    }

    /// Inverts this color's RGB channels.
    ///
    /// Each RGB channel is replaced with `255 - channel`, while the alpha
    /// channel is preserved.
    pub const fn invert(self) -> Self {
        let (r, g, b, a) = self.to_rgba_components();

        Self::from_rgba_components(255 - r, 255 - g, 255 - b, a)
    }

    /// Increases this color's saturation by `amount`.
    ///
    /// `amount` is clamped to a minimum of `0.0`. RGB channels are moved away
    /// from their luminance value and clamped to valid channel values; alpha is
    /// preserved.
    pub const fn saturate(self, amount: f32) -> Self {
        let amount = amount.max(0.0);
        let (r, g, b, a) = self.to_rgba_components();
        let gray = luminance(r, g, b);

        Self::from_rgba_components(
            float_to_channel(gray + (r as f32 - gray) * (1.0 + amount)),
            float_to_channel(gray + (g as f32 - gray) * (1.0 + amount)),
            float_to_channel(gray + (b as f32 - gray) * (1.0 + amount)),
            a,
        )
    }

    /// Decreases this color's saturation by `amount`.
    ///
    /// `amount` is clamped to `0.0..=1.0`, where `0.0` returns the original
    /// color and `1.0` returns a grayscale color with the original alpha.
    pub const fn desaturate(self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let (r, g, b, a) = self.to_rgba_components();
        let gray = float_to_channel(luminance(r, g, b));

        Self::from_rgba_components(
            lerp_channel(r, gray, amount),
            lerp_channel(g, gray, amount),
            lerp_channel(b, gray, amount),
            a,
        )
    }

    /// Converts this color to grayscale while preserving alpha.
    ///
    /// The gray value is calculated from the RGB channels using luminance
    /// weights.
    pub const fn grayscale(self) -> Self {
        self.desaturate(1.0)
    }

    const fn to_rgba_components(self) -> (u8, u8, u8, u8) {
        let argb = self.as_u32();

        (
            ((argb >> 16) & 0xFF) as u8,
            ((argb >> 8) & 0xFF) as u8,
            (argb & 0xFF) as u8,
            ((argb >> 24) & 0xFF) as u8,
        )
    }

    const fn from_rgba_components(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color::Rgba(r, g, b, a)
    }
}

const fn float_to_channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

const fn lerp_channel(start: u8, end: u8, t: f32) -> u8 {
    float_to_channel(start as f32 + (end as f32 - start as f32) * t)
}

const fn luminance(r: u8, g: u8, b: u8) -> f32 {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

/// Prints the packed value, the only thing a color still knows about itself.
///
/// The format is `Color(#AARRGGBB)`, which round-trips through
/// [`Color::HexA`] read as `0xRRGGBBAA` and is directly comparable to
/// [`Color::as_u32`].
impl fmt::Debug for Color {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Color(#{:08X})", self.0)
    }
}

impl From<PrimitiveColor> for Color {
    #[inline]
    fn from(value: PrimitiveColor) -> Self {
        Self::from_primitive(value)
    }
}

impl From<Color> for PrimitiveColor {
    #[inline]
    fn from(value: Color) -> Self {
        value.as_u32()
    }
}

impl From<Colors> for Color {
    fn from(value: Colors) -> Self {
        Self::Basic(value)
    }
}

impl From<Colors> for Option<Color> {
    fn from(value: Colors) -> Self {
        Some(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn with_opacity_rgb() {
        assert_eq!(
            Color::Rgb(10, 20, 30).with_opacity(128).as_u32(),
            0x800A141E
        );
    }

    #[test]
    fn with_opacity_rgba_overrides_alpha() {
        assert_eq!(
            Color::Rgba(10, 20, 30, 255).with_opacity(0).as_u32(),
            0x000A141E
        );
    }

    #[test]
    fn with_opacity_hex() {
        assert_eq!(Color::Hex(0x112233).with_opacity(0x80).as_u32(), 0x80112233);
    }

    #[test]
    fn with_opacity_hexa_overrides_alpha() {
        assert_eq!(
            Color::HexA(0x112233FF).with_opacity(0x80).as_u32(),
            0x80112233
        );
    }

    #[test]
    fn with_opacity_gray() {
        assert_eq!(Color::Gray8(0x40).with_opacity(0x80).as_u32(), 0x80404040);
    }

    #[test]
    fn with_opacity_basic() {
        assert_eq!(
            Color::Basic(Colors::Red).with_opacity(0x80).as_u32(),
            0x80FF0000
        );
    }

    #[test]
    fn with_opacity_transparent() {
        assert_eq!(Color::Transparent.with_opacity(0x80).as_u32(), 0x80000000);
    }

    #[test]
    fn with_opacity_hsl() {
        // Red HSL -> alpha 0.5 * 255 rounds to 128 (0x80)
        assert_eq!(
            Color::Hsl(0.0, 1.0, 0.5).with_opacity(128).as_u32(),
            0x80FF0000
        );
    }

    #[test]
    fn darken_scales_rgb_only() {
        assert_eq!(
            Color::Rgba(100, 150, 200, 128).darken(0.5).as_u32(),
            0x80324B64
        );
    }

    #[test]
    fn lighten_moves_rgb_toward_white() {
        assert_eq!(Color::Rgb(100, 150, 200).lighten(0.5).as_u32(), 0xFFB2CBE4);
    }

    #[test]
    fn with_alpha_accepts_normalized_alpha() {
        assert_eq!(Color::Rgb(10, 20, 30).with_alpha(0.5).as_u32(), 0x800A141E);
    }

    #[test]
    fn multiply_clamps_rgb_and_keeps_alpha() {
        assert_eq!(
            Color::Rgba(100, 150, 200, 128).multiply(2.0).as_u32(),
            0x80C8FFFF
        );
    }

    #[test]
    fn blend_and_lerp_interpolate_channels() {
        let red = Color::Rgb(255, 0, 0);
        let blue = Color::Rgba(0, 0, 255, 0);

        assert_eq!(red.lerp(blue, 0.5).as_u32(), 0x80800080);
        assert_eq!(red.blend(blue, 0.5), red.lerp(blue, 0.5));
    }

    #[test]
    fn invert_keeps_alpha() {
        assert_eq!(Color::Rgba(10, 20, 30, 40).invert().as_u32(), 0x28F5EBE1);
    }

    #[test]
    fn saturate_pushes_channels_away_from_gray() {
        assert_eq!(Color::Rgb(100, 150, 200).saturate(0.5).as_u32(), 0xFF509BE6);
    }

    #[test]
    fn desaturate_moves_channels_toward_gray() {
        assert_eq!(
            Color::Rgb(100, 150, 200).desaturate(0.5).as_u32(),
            0xFF7992AB
        );
    }

    #[test]
    fn grayscale_sets_rgb_to_luminance() {
        assert_eq!(Color::Rgb(100, 150, 200).grayscale().as_u32(), 0xFF8D8D8D);
    }

    #[test]
    fn a_color_is_one_packed_word() {
        assert_eq!(size_of::<Color>(), size_of::<PrimitiveColor>());
        assert_eq!(size_of::<Color>(), 4);
        assert_eq!(align_of::<Color>(), align_of::<PrimitiveColor>());
    }

    #[test]
    fn every_model_resolves_to_the_same_packed_value() {
        let red = Color::Rgb(255, 0, 0);

        assert_eq!(red, Color::Rgba(255, 0, 0, 255));
        assert_eq!(red, Color::Hex(0xFF0000));
        assert_eq!(red, Color::HexA(0xFF0000FF));
        assert_eq!(red, Color::Hsl(0.0, 1.0, 0.5));
        assert_eq!(red, Color::Hsla(0.0, 1.0, 0.5, 1.0));
        assert_eq!(red, Color::Basic(Colors::Red));
        assert_eq!(red, Color::RED);

        let gray = Color::Gray8(0x40);

        assert_eq!(gray, Color::Grayscale(0x40, 0xFF));
        assert_eq!(gray, Color::Rgb(0x40, 0x40, 0x40));
    }

    #[test]
    fn a_packed_value_round_trips() {
        let packed: PrimitiveColor = 0x80123456;

        assert_eq!(Color::from_primitive(packed).as_u32(), packed);
        assert_eq!(Color::from(packed), Color::HexA(0x12345680));
        assert_eq!(PrimitiveColor::from(Color::Hex(0x123456)), 0xFF123456);
    }

    #[test]
    fn channels_are_readable_without_destructuring() {
        let color = Color::Rgba(0x0A, 0x14, 0x1E, 0x80);

        assert_eq!(color.red(), 0x0A);
        assert_eq!(color.green(), 0x14);
        assert_eq!(color.blue(), 0x1E);
        assert_eq!(color.alpha(), 0x80);
        assert_eq!(color.to_rgba(), (0x0A, 0x14, 0x1E, 0x80));
    }

    #[test]
    fn the_default_color_is_transparent() {
        assert_eq!(Color::default(), Color::Transparent);
        assert_eq!(Color::default().as_u32(), 0x00000000);
    }

    #[test]
    fn debug_prints_the_packed_value() {
        assert_eq!(
            format!("{:?}", Color::Rgba(0x0A, 0x14, 0x1E, 0x80)),
            "Color(#800A141E)"
        );
    }
}
