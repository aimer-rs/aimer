//! Gradient fill data carried by `MaterialKind::Gradient` requests.

use super::MaterialDecodeError;

/// The most color stops one gradient request carries.
pub const MAX_GRADIENT_STOPS: usize = 8;

/// Bytes a gradient request appends to the fixed material packet body.
///
/// Layout, little endian: shape `u8`, stop count `u8`, two reserved zero bytes,
/// four geometry `f32`s, [`MAX_GRADIENT_STOPS`] offset `f32`s and
/// [`MAX_GRADIENT_STOPS`] packed RGBA8 `u32`s. Unused stop slots are zero.
pub const GRADIENT_PACKET_EXTENSION_LEN: usize = 4 + 16 + MAX_GRADIENT_STOPS * 4 * 2;

/// The geometry a gradient's color ramp follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum GradientShape {
    /// Colors change along the line from `geometry[0..2]` to `geometry[2..4]`.
    #[default]
    Linear = 0,
    /// Colors change with the distance from the center `geometry[0..2]`,
    /// measured in units of the radii `geometry[2..4]`.
    Radial = 1,
    /// Colors sweep around the center `geometry[0..2]`, starting at the angle
    /// `geometry[2]` in radians and turning clockwise on screen.
    Conic = 2,
}

impl GradientShape {
    #[inline]
    fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Linear),
            1 => Some(Self::Radial),
            2 => Some(Self::Conic),
            _ => None,
        }
    }
}

/// The color ramp of a gradient material.
///
/// Geometry is in pixels relative to the request's bounds origin. Colors are
/// straight (not premultiplied) display-space RGBA, packed as
/// `r | g << 8 | b << 16 | a << 24`, and are blended premultiplied the way
/// CSS gradients are. Offsets are non-decreasing in `0.0..=1.0`; the color
/// before the first stop and after the last one is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialGradient {
    /// Geometry family.
    pub shape: GradientShape,
    /// Shape parameters; see [`GradientShape`].
    pub geometry: [f32; 4],
    /// Number of used stops, at most [`MAX_GRADIENT_STOPS`]. Zero draws nothing.
    pub stop_count: u8,
    /// Stop positions along the ramp.
    pub offsets: [f32; MAX_GRADIENT_STOPS],
    /// Packed RGBA8 stop colors.
    pub colors: [u32; MAX_GRADIENT_STOPS],
}

impl Default for MaterialGradient {
    #[inline]
    fn default() -> Self {
        Self {
            shape: GradientShape::Linear,
            geometry: [0.0; 4],
            stop_count: 0,
            offsets: [0.0; MAX_GRADIENT_STOPS],
            colors: [0; MAX_GRADIENT_STOPS],
        }
    }
}

impl MaterialGradient {
    /// Packs straight RGBA8 channels into the stop color format.
    #[inline]
    pub const fn pack_color([r, g, b, a]: [u8; 4]) -> u32 {
        u32::from_le_bytes([r, g, b, a])
    }

    /// Returns a gradient with finite, ordered, in-range values and zeroed
    /// unused stops, so equal ramps encode to equal bytes.
    pub fn normalized(mut self) -> Self {
        let count = usize::from(self.stop_count).min(MAX_GRADIENT_STOPS);
        self.stop_count = count as u8;
        for value in &mut self.geometry {
            *value = if value.is_finite() {
                value.clamp(-1_000_000.0, 1_000_000.0)
            } else {
                0.0
            };
        }
        if self.shape == GradientShape::Radial {
            self.geometry[2] = self.geometry[2].max(0.0);
            self.geometry[3] = self.geometry[3].max(0.0);
        }
        let mut previous = 0.0_f32;
        for index in 0..MAX_GRADIENT_STOPS {
            if index < count {
                let offset = if self.offsets[index].is_finite() {
                    self.offsets[index].clamp(0.0, 1.0)
                } else {
                    previous
                };
                previous = offset.max(previous);
                self.offsets[index] = previous;
            } else {
                self.offsets[index] = 0.0;
                self.colors[index] = 0;
            }
        }
        self
    }

    /// Appends this gradient's fixed-size packet extension to `bytes`.
    ///
    /// The gradient is normalized first, so the bytes are always decodable.
    pub fn encode_extension(self, bytes: &mut Vec<u8>) {
        let gradient = self.normalized();
        bytes.push(gradient.shape as u8);
        bytes.push(gradient.stop_count);
        bytes.extend_from_slice(&[0, 0]);
        for value in gradient.geometry.iter().chain(&gradient.offsets) {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        for color in gradient.colors {
            bytes.extend_from_slice(&color.to_le_bytes());
        }
    }

    pub(super) fn decode_extension(bytes: &[u8]) -> Result<Self, MaterialDecodeError> {
        let invalid_length = MaterialDecodeError::InvalidLength {
            expected: GRADIENT_PACKET_EXTENSION_LEN,
            actual: bytes.len(),
        };
        if bytes.len() != GRADIENT_PACKET_EXTENSION_LEN {
            return Err(invalid_length);
        }
        let shape = GradientShape::from_wire(bytes[0]).ok_or(MaterialDecodeError::UnsupportedKind)?;
        if usize::from(bytes[1]) > MAX_GRADIENT_STOPS {
            return Err(MaterialDecodeError::InvalidGradient);
        }
        if bytes[2..4] != [0, 0] {
            return Err(MaterialDecodeError::NonZeroReserved);
        }
        let word = |index: usize| -> [u8; 4] {
            let start = 4 + index * 4;
            [bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3]]
        };
        let mut gradient = Self {
            shape,
            stop_count: bytes[1],
            ..Self::default()
        };
        for (index, value) in gradient.geometry.iter_mut().enumerate() {
            *value = f32::from_bits(u32::from_le_bytes(word(index)));
        }
        for (index, value) in gradient.offsets.iter_mut().enumerate() {
            *value = f32::from_bits(u32::from_le_bytes(word(4 + index)));
        }
        for (index, value) in gradient.colors.iter_mut().enumerate() {
            *value = u32::from_le_bytes(word(4 + MAX_GRADIENT_STOPS + index));
        }
        Ok(gradient.normalized())
    }
}
