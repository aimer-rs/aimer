use std::ops::Mul;

pub use super::rgba8::Rgba8;
use aimer_color::prelude::Color as AimerColor;

/// Cupid's renderer-side paint color.
///
/// The framework supplies colors as packed ARGB. Cupid keeps ordinary paint
/// colors as RGBA bytes all the way through command recording, and the GPU
/// vertex format normalizes them when it fetches an instance. `new` and
/// `to_array` remain normalized-float convenience APIs for code that needs
/// them; the stored representation is four bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// Creates a color from normalized channels, quantized to RGBA8.
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self::from_unorm([r, g, b, a])
    }

    /// Creates a color from normalized channels, quantized to RGBA8.
    pub const fn from_unorm([r, g, b, a]: [f32; 4]) -> Self {
        let rgba = Rgba8::from_unorm([r, g, b, a]);
        Self::rgba8(rgba.0[0], rgba.0[1], rgba.0[2], rgba.0[3])
    }

    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn white() -> Self {
        Self::rgba8(255, 255, 255, 255)
    }

    pub const fn black() -> Self {
        Self::rgba8(0, 0, 0, 255)
    }

    pub const fn red() -> Self {
        Self::rgba8(255, 0, 0, 255)
    }

    pub const fn green() -> Self {
        Self::rgba8(0, 255, 0, 255)
    }

    pub const fn blue() -> Self {
        Self::rgba8(0, 0, 255, 255)
    }

    pub const fn transparent() -> Self {
        Self::rgba8(0, 0, 0, 0)
    }
}

impl Mul<u8> for Color {
    type Output = Self;
    fn mul(self, rhs: u8) -> Self::Output {
        Self {
            a: Rgba8::from_unorm([0.0, 0.0, 0.0, self.a as f32 / 255.0 * rhs as f32]).0[3],
            ..self
        }
    }
}

impl Color {
    pub const fn to_array(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// Returns the stored straight-alpha RGBA bytes without a float round-trip.
    #[inline]
    pub const fn to_rgba8(self) -> Rgba8 {
        Rgba8::new(self.r, self.g, self.b, self.a)
    }

    pub const fn set_alpha(&mut self, alpha: u8) -> Self {
        self.a = alpha;
        *self
    }
}

impl From<AimerColor> for Color {
    #[inline]
    fn from(c: AimerColor) -> Self {
        let rgba = Rgba8::from_aimer_color(c);
        Self::rgba8(rgba.0[0], rgba.0[1], rgba.0[2], rgba.0[3])
    }
}

impl From<Rgba8> for Color {
    #[inline]
    fn from(color: Rgba8) -> Self {
        Self::rgba8(color.0[0], color.0[1], color.0[2], color.0[3])
    }
}

impl From<AimerColor> for Rgba8 {
    #[inline]
    fn from(color: AimerColor) -> Self {
        Self::from_aimer_color(color)
    }
}

impl From<Color> for Rgba8 {
    #[inline]
    fn from(color: Color) -> Self {
        color.to_rgba8()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_framework_argb_is_explicitly_swizzled_to_rgba() {
        let color = AimerColor::Rgba(0x11, 0x22, 0x33, 0x44);

        assert_eq!(
            Rgba8::from_aimer_color(color).as_array(),
            [0x11, 0x22, 0x33, 0x44]
        );
    }

    #[test]
    fn opacity_only_rewrites_the_alpha_byte() {
        assert_eq!(
            Rgba8::new(10, 20, 30, 128).with_opacity(0.5).as_array(),
            [10, 20, 30, 64]
        );
    }

    #[test]
    fn float_colors_are_clamped_and_quantized_for_gpu_upload() {
        assert_eq!(
            Rgba8::from_unorm([-1.0, 0.5, 2.0, f32::NAN]).as_array(),
            [0, 128, 255, 0]
        );
        assert_eq!(
            Rgba8::from_unorm([f32::INFINITY, f32::NEG_INFINITY, 0.0, 1.0]).as_array(),
            [0, 0, 0, 255]
        );
    }

    #[test]
    fn cupid_color_keeps_framework_channels_packed() {
        let color: Color = AimerColor::Rgba(0x11, 0x22, 0x33, 0x44).into();

        assert_eq!(std::mem::size_of::<Color>(), 4);
        assert_eq!(color.to_rgba8(), Rgba8::new(0x11, 0x22, 0x33, 0x44));
    }
}
