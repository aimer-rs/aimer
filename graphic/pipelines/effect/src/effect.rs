//! The application-facing description of an effect, and its draw payload.

use std::sync::Arc;

use crate::ParamWriter;

/// An application-defined shader effect.
///
/// An effect is a `.aimer` shader plus the values that drive it. Implement this
/// for a type that holds those values, register the type once, and draw it with
/// [`EffectCommand::new`]. The renderer translates [`SOURCE`](Self::SOURCE) for
/// whichever backend it is running on.
///
/// # The shader contract
///
/// The shader draws one rectangle. It has no vertex buffers: the framework
/// passes everything through one uniform block at `#[bind(0, 0)]` whose struct
/// begins with this 64-byte header, in this order, and continues with the
/// effect's own fields:
///
/// ```text
/// uniform struct Frame {
///     target_size: vec2,      // render target size in pixels
///     surface_is_srgb: f32,   // 0, 1 or 2, as in the built-in shaders
///     opacity: f32,           // inherited opacity
///     rect: vec4,             // x, y, width, height in target pixels
///     clip_rect: vec4,        // active clip; width < 0 when none
///     clip_radii: vec4,       // clip corner radii
///     // ...your fields, written by `write_params`...
/// }
/// ```
///
/// The vertex stage `vs_main(vi: @vertex_index)` emits the six corners of
/// `rect` and the fragment stage is `fs_main`, returning a **premultiplied**
/// color. `aimer::clip::clip_alpha(pixel, clip_rect, clip_radii)` applies the
/// clip. When [`SAMPLES_BACKDROP`](Self::SAMPLES_BACKDROP) is `true` the shader
/// also declares `#[bind(0, 1)] texture` and `#[bind(0, 2)] sampler`, and the
/// texture holds the frame as drawn so far, in target pixel coordinates, valid
/// inside `rect`. A shader that breaks the contract is rejected when the
/// effect is registered, with a message naming what is wrong.
///
/// # Examples
///
/// ```
/// use aimer_effect_pipeline::{ParamWriter, ShaderEffect};
///
/// struct Water {
///     time: f32,
///     amplitude: f32,
/// }
///
/// impl ShaderEffect for Water {
///     const NAME: &'static str = "water";
///     const SOURCE: &'static str = include_str!("../tests/shaders/water.aimer");
///     const SAMPLES_BACKDROP: bool = true;
///
///     fn write_params(&self, params: &mut ParamWriter) {
///         // `time`, `amplitude`, `frequency`, `_pad`: 16 bytes.
///         params.f32(self.time).f32(self.amplitude).f32(12.0).pad(4);
///     }
/// }
///
/// let command = aimer_effect_pipeline::EffectCommand::new(
///     &Water { time: 0.0, amplitude: 4.0 },
///     [0.0, 0.0, 200.0, 120.0],
/// )
/// .unwrap();
/// assert_eq!(command.name, "water");
/// ```
pub trait ShaderEffect: 'static {
    /// The pipeline's unique name, which draw commands address it by.
    const NAME: &'static str;

    /// The effect's `.aimer` source.
    const SOURCE: &'static str;

    /// Whether the shader samples the frame behind the effect.
    const SAMPLES_BACKDROP: bool = false;

    /// Writes this effect's fields, in the order the shader declares them after
    /// the frame header.
    fn write_params(&self, params: &mut ParamWriter);
}

/// One draw of an effect: the pipeline to address and the bytes to send it.
#[derive(Clone, Debug)]
pub struct EffectCommand {
    /// The pipeline name for `draw_custom`.
    pub name: &'static str,
    /// The payload for `draw_custom`: the rectangle, then the parameters.
    pub payload: Arc<[u8]>,
}

impl EffectCommand {
    /// Encodes one draw of `effect` over `rect` (`[x, y, width, height]`, in
    /// the local space of the element that records the command).
    ///
    /// # Errors
    ///
    /// Returns a message when the effect's parameters are misaligned or are
    /// not a whole number of 16-byte rows.
    pub fn new<E: ShaderEffect>(effect: &E, rect: [f32; 4]) -> Result<Self, String> {
        if rect.iter().any(|value| !value.is_finite()) {
            return Err(format!("effect `{}`: the rectangle {rect:?} is not finite", E::NAME));
        }
        let mut writer = ParamWriter::new();
        effect.write_params(&mut writer);
        let params = writer
            .finish()
            .map_err(|error| format!("effect `{}`: {error}", E::NAME))?;
        let mut payload = Vec::with_capacity(RECT_BYTES + params.len());
        for value in rect {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        payload.extend_from_slice(&params);
        Ok(Self {
            name: E::NAME,
            payload: payload.into(),
        })
    }
}

/// Splits a payload into its rectangle and parameter bytes.
///
/// Returns `None` for anything that [`EffectCommand::new`] cannot have made.
pub fn decode_payload(payload: &[u8]) -> Option<([f32; 4], &[u8])> {
    if payload.len() < RECT_BYTES || (payload.len() - RECT_BYTES) % 16 != 0 {
        return None;
    }
    let (rect_bytes, params) = payload.split_at(RECT_BYTES);
    let mut rect = [0.0; 4];
    for (value, chunk) in rect.iter_mut().zip(rect_bytes.chunks_exact(4)) {
        *value = f32::from_le_bytes(chunk.try_into().ok()?);
    }
    Some((rect, params))
}

/// The rectangle at the front of a payload: four little-endian `f32`s.
const RECT_BYTES: usize = 16;

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe(f32);

    impl ShaderEffect for Probe {
        const NAME: &'static str = "probe";
        const SOURCE: &'static str = "";

        fn write_params(&self, params: &mut ParamWriter) {
            params.f32(self.0).pad(12);
        }
    }

    struct Misaligned;

    impl ShaderEffect for Misaligned {
        const NAME: &'static str = "misaligned";
        const SOURCE: &'static str = "";

        fn write_params(&self, params: &mut ParamWriter) {
            params.f32(1.0).vec4([0.0; 4]);
        }
    }

    #[test]
    fn a_payload_round_trips_the_rect_and_parameters() {
        let command = EffectCommand::new(&Probe(2.5), [1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(command.name, "probe");

        let (rect, params) = decode_payload(&command.payload).unwrap();
        assert_eq!(rect, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(params.len(), 16);
        assert_eq!(f32::from_le_bytes(params[..4].try_into().unwrap()), 2.5);
    }

    #[test]
    fn invalid_parameters_are_reported_when_the_command_is_built() {
        let error = EffectCommand::new(&Misaligned, [0.0; 4]).unwrap_err();
        assert!(error.contains("misaligned") && error.contains("vec4"), "{error}");
    }

    #[test]
    fn a_non_finite_rect_is_rejected() {
        for rect in [[f32::NAN, 0.0, 1.0, 1.0], [0.0, 0.0, f32::INFINITY, 1.0]] {
            assert!(EffectCommand::new(&Probe(0.0), rect).is_err());
        }
    }

    #[test]
    fn foreign_payloads_do_not_decode() {
        assert!(decode_payload(&[]).is_none());
        assert!(decode_payload(&[0; 15]).is_none());
        // A rectangle but a partial parameter row.
        assert!(decode_payload(&[0; 20]).is_none());
        assert!(decode_payload(&[0; 16]).is_some());
        assert!(decode_payload(&[0; 32]).is_some());
    }
}
