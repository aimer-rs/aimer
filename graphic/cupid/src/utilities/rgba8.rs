use aimer_color::prelude::Color as AimerColor;
use bytemuck::{Pod, Zeroable};

/// A straight-alpha RGBA color stored in the four bytes consumed by a
/// `Unorm8x4` vertex attribute.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Pod, Zeroable)]
pub struct Rgba8(pub [u8; 4]);

impl Rgba8 {
    /// A fully transparent black color.
    pub const TRANSPARENT: Self = Self([0; 4]);

    /// Creates an RGBA color from byte channels.
    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    /// Converts packed framework ARGB (`0xAARRGGBB`) to RGBA bytes.
    #[inline]
    pub const fn from_argb(argb: u32) -> Self {
        Self([
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
            ((argb >> 24) & 0xff) as u8,
        ])
    }

    /// Converts a framework color without passing through floating point.
    #[inline]
    pub const fn from_aimer_color(color: AimerColor) -> Self {
        Self::from_argb(color.as_u32())
    }

    /// Converts normalized float channels to GPU upload bytes.
    #[inline]
    pub const fn from_unorm(color: [f32; 4]) -> Self {
        Self([
            quantize_channel(color[0]),
            quantize_channel(color[1]),
            quantize_channel(color[2]),
            quantize_channel(color[3]),
        ])
    }

    /// Returns the four bytes in RGBA order.
    #[inline]
    pub const fn as_array(self) -> [u8; 4] {
        self.0
    }

    /// Returns the channels as normalized floats for CPU-side calculations.
    #[inline]
    pub const fn to_unorm_array(self) -> [f32; 4] {
        [
            self.0[0] as f32 / 255.0,
            self.0[1] as f32 / 255.0,
            self.0[2] as f32 / 255.0,
            self.0[3] as f32 / 255.0,
        ]
    }

    /// Applies a normalized opacity multiplier to the alpha byte.
    #[inline]
    pub fn with_opacity(self, opacity: f32) -> Self {
        let alpha = if opacity.is_finite() {
            (self.0[3] as f32 * opacity.clamp(0.0, 1.0)).round() as u8
        } else {
            0
        };
        Self([self.0[0], self.0[1], self.0[2], alpha])
    }
}

#[inline]
const fn quantize_channel(value: f32) -> u8 {
    if value == f32::INFINITY || value == f32::NEG_INFINITY {
        return 0;
    }
    if !(value > 0.0) {
        return 0;
    }
    if value >= 1.0 {
        return 255;
    }
    (value * 255.0 + 0.5) as u8
}
