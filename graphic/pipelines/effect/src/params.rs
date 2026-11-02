//! Writing an effect's parameters in the layout its shader declares.

/// Writes an effect's parameters in std140 order, one field at a time.
///
/// The shader declares the parameters as the trailing fields of its `Frame`
/// uniform struct, and `aimer_transpiler` requires that struct to spell out its
/// padding (an `_pad` field wherever std140 would leave a gap). This writer
/// mirrors that rule: call the methods in the same order as the shader's
/// fields, and call [`pad`](Self::pad) wherever the shader has a pad field. A
/// field that is not at its natural alignment, or a total that is not a
/// multiple of 16 bytes, is reported when the command is built.
#[derive(Debug, Default)]
pub struct ParamWriter {
    bytes: Vec<u8>,
    error: Option<String>,
}

impl ParamWriter {
    /// An empty writer.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an `f32` field.
    pub fn f32(&mut self, value: f32) -> &mut Self {
        self.field("f32", 4, &value.to_le_bytes())
    }

    /// Appends an `i32` field.
    pub fn i32(&mut self, value: i32) -> &mut Self {
        self.field("i32", 4, &value.to_le_bytes())
    }

    /// Appends a `u32` field.
    pub fn u32(&mut self, value: u32) -> &mut Self {
        self.field("u32", 4, &value.to_le_bytes())
    }

    /// Appends a `vec2` field.
    pub fn vec2(&mut self, value: [f32; 2]) -> &mut Self {
        self.field("vec2", 8, &floats(&value))
    }

    /// Appends a `vec3` field, which is 16-aligned but only 12 bytes long, so
    /// an `f32` may follow it directly.
    pub fn vec3(&mut self, value: [f32; 3]) -> &mut Self {
        self.field("vec3", 16, &floats(&value))
    }

    /// Appends a `vec4` field.
    pub fn vec4(&mut self, value: [f32; 4]) -> &mut Self {
        self.field("vec4", 16, &floats(&value))
    }

    /// Appends `bytes` zero bytes, for a shader `_pad` field.
    pub fn pad(&mut self, bytes: usize) -> &mut Self {
        self.bytes.resize(self.bytes.len() + bytes, 0);
        self
    }

    /// The number of bytes written so far.
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether nothing has been written.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Finishes the parameters.
    ///
    /// # Errors
    ///
    /// Returns the first misaligned field, or a size that is not a multiple of
    /// 16 bytes (the alignment of the `Frame` header that precedes them).
    pub(crate) fn finish(self) -> Result<Vec<u8>, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let rem = self.bytes.len() % 16;
        if rem != 0 {
            return Err(format!(
                "parameters are {} bytes, which is not a multiple of 16; add `params.pad({})` to match the shader's trailing `_pad`",
                self.bytes.len(),
                16 - rem
            ));
        }
        Ok(self.bytes)
    }

    fn field(&mut self, ty: &str, align: usize, data: &[u8]) -> &mut Self {
        let offset = self.bytes.len();
        // Keep the first problem: later fields are shifted by it and would
        // only report its consequences.
        if self.error.is_none() && offset % align != 0 {
            self.error = Some(format!(
                "{ty} field at byte {offset} needs {align}-byte alignment; add `params.pad({})` before it to match the shader's `_pad` field",
                align - offset % align
            ));
        }
        self.bytes.extend_from_slice(data);
        self
    }
}

fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_written_little_endian_in_order() {
        let mut writer = ParamWriter::new();
        writer.f32(1.0).f32(2.0).vec2([3.0, 4.0]).vec4([5.0, 6.0, 7.0, 8.0]);
        let bytes = writer.finish().unwrap();
        let words: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect();
        assert_eq!(words, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    }

    #[test]
    fn integers_keep_their_bit_patterns() {
        let mut writer = ParamWriter::new();
        writer.i32(-2).u32(7).pad(8);
        let bytes = writer.finish().unwrap();
        assert_eq!(&bytes[0..4], (-2_i32).to_le_bytes());
        assert_eq!(&bytes[4..8], 7_u32.to_le_bytes());
        assert_eq!(&bytes[8..], [0; 8]);
    }

    #[test]
    fn a_vec3_is_followed_by_a_scalar_in_the_same_sixteen_bytes() {
        let mut writer = ParamWriter::new();
        writer.vec3([1.0, 2.0, 3.0]).f32(4.0);
        assert_eq!(writer.len(), 16);
        assert_eq!(writer.finish().unwrap().len(), 16);
    }

    #[test]
    fn a_misaligned_field_names_the_fix() {
        let mut writer = ParamWriter::new();
        writer.f32(1.0).vec4([0.0; 4]);
        let error = writer.finish().unwrap_err();
        assert!(error.contains("vec4") && error.contains("byte 4"), "{error}");
        assert!(error.contains("pad(12)"), "{error}");
    }

    #[test]
    fn only_the_first_misalignment_is_reported() {
        let mut writer = ParamWriter::new();
        writer.f32(1.0).vec2([0.0; 2]).vec2([0.0; 2]).vec4([0.0; 4]);
        let error = writer.finish().unwrap_err();
        assert!(error.contains("vec2") && error.contains("byte 4"), "{error}");
    }

    #[test]
    fn the_total_must_fill_whole_sixteen_byte_rows() {
        let mut writer = ParamWriter::new();
        writer.f32(1.0).f32(2.0);
        let error = writer.finish().unwrap_err();
        assert!(error.contains("multiple of 16") && error.contains("8"), "{error}");
        assert!(ParamWriter::new().finish().unwrap().is_empty());
    }
}
