//! The frame header every effect's uniform block starts with.

/// Bytes the framework writes at the start of every effect's uniform block.
///
/// Six std140 fields, 64 bytes: `target_size: vec2`, `surface_is_srgb: f32`,
/// `opacity: f32`, `rect: vec4`, `clip_rect: vec4`, `clip_radii: vec4`.
pub const HEADER_SIZE: usize = 64;

/// The names and types of the header fields, in order. A shader's `Frame`
/// struct must begin with exactly these.
pub(crate) const HEADER_FIELDS: [(&str, &str); 6] = [
    ("target_size", "vec2"),
    ("surface_is_srgb", "f32"),
    ("opacity", "f32"),
    ("rect", "vec4"),
    ("clip_rect", "vec4"),
    ("clip_radii", "vec4"),
];

/// What the framework knows about one command at upload time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Header {
    /// Size of the render target in pixels.
    pub target_size: [f32; 2],
    /// The renderer's `surface_is_srgb` flag (0, 1 or 2) as a float.
    pub surface_is_srgb: f32,
    /// Inherited opacity.
    pub opacity: f32,
    /// The effect rectangle `[x, y, width, height]` in target pixels.
    pub rect: [f32; 4],
    /// Clip rectangle in target pixels; a negative width means no clip.
    pub clip_rect: [f32; 4],
    /// Clip corner radii.
    pub clip_radii: [f32; 4],
}

impl Header {
    /// Writes the header into the first [`HEADER_SIZE`] bytes of `out`.
    ///
    /// # Panics
    ///
    /// Panics if `out` is shorter than [`HEADER_SIZE`].
    pub(crate) fn write_to(&self, out: &mut [u8]) {
        let out = &mut out[..HEADER_SIZE];
        let words = [
            self.target_size[0],
            self.target_size[1],
            self.surface_is_srgb,
            self.opacity,
            self.rect[0],
            self.rect[1],
            self.rect[2],
            self.rect[3],
            self.clip_rect[0],
            self.clip_rect[1],
            self.clip_rect[2],
            self.clip_rect[3],
            self.clip_radii[0],
            self.clip_radii[1],
            self.clip_radii[2],
            self.clip_radii[3],
        ];
        for (chunk, word) in out.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(bytes: &[u8], index: usize) -> f32 {
        f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
    }

    #[test]
    fn fields_sit_at_their_std140_offsets() {
        let header = Header {
            target_size: [640.0, 480.0],
            surface_is_srgb: 1.0,
            opacity: 0.5,
            rect: [10.0, 20.0, 30.0, 40.0],
            clip_rect: [1.0, 2.0, 3.0, 4.0],
            clip_radii: [5.0, 6.0, 7.0, 8.0],
        };
        let mut out = [0xAB_u8; HEADER_SIZE + 4];
        header.write_to(&mut out);

        let expected = [
            640.0, 480.0, 1.0, 0.5, 10.0, 20.0, 30.0, 40.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0,
            8.0,
        ];
        for (index, value) in expected.into_iter().enumerate() {
            assert_eq!(word(&out, index), value, "word {index}");
        }
        assert_eq!(out[HEADER_SIZE..], [0xAB; 4], "bytes after the header are untouched");
    }

    #[test]
    #[should_panic]
    fn a_short_buffer_panics() {
        let header = Header {
            target_size: [0.0; 2],
            surface_is_srgb: 0.0,
            opacity: 1.0,
            rect: [0.0; 4],
            clip_rect: [0.0; 4],
            clip_radii: [0.0; 4],
        };
        header.write_to(&mut [0_u8; HEADER_SIZE - 1]);
    }
}
