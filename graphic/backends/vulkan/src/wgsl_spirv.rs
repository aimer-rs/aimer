//! WGSL to SPIR-V for sources that arrive at run time.
//!
//! Vulkan consumes only SPIR-V, and the built-in shaders are checked in as
//! `.spv` files that `examples/vulkan_shadergen.rs` regenerates from WGSL. A
//! shader supplied by an application has no offline step, so the backend
//! compiles it here with the same options the generator uses; the two paths
//! produce identical bytes for identical WGSL.

use std::borrow::Cow;

/// The first word of every SPIR-V module.
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// Compiles WGSL source to little-endian SPIR-V bytes.
///
/// The module is validated first, so a shader that would crash a driver is
/// rejected with a message instead. Entry points keep their WGSL names, and a
/// `@group(g) @binding(b)` resource becomes descriptor set `g`, binding `b`.
///
/// # Errors
///
/// Returns the parser's, validator's or writer's message.
pub fn compile_wgsl_to_spirv(source: &str) -> Result<Vec<u8>, String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|error| error.to_string())?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|error| error.to_string())?;
    // ADJUST_COORDINATE_SPACE flips Y so WGSL's clip space matches Vulkan's.
    let options = naga::back::spv::Options {
        flags: naga::back::spv::WriterFlags::ADJUST_COORDINATE_SPACE
            | naga::back::spv::WriterFlags::LABEL_VARYINGS
            | naga::back::spv::WriterFlags::CLAMP_FRAG_DEPTH,
        ..Default::default()
    };
    let words = naga::back::spv::write_vec(&module, &info, &options, None)
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(words.len() * 4);
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    Ok(bytes)
}

/// Whether `bytes` begin with the SPIR-V magic number.
pub(crate) fn is_spirv(bytes: &[u8]) -> bool {
    bytes
        .first_chunk::<4>()
        .is_some_and(|word| u32::from_le_bytes(*word) == SPIRV_MAGIC)
}

/// Returns `source` unchanged when it is SPIR-V and compiles it when it is WGSL.
///
/// # Errors
///
/// Returns a message when SPIR-V is truncated, or when the bytes are neither
/// SPIR-V nor WGSL that compiles. `label` names the module in the message.
pub(crate) fn resolve_spirv<'a>(source: &'a [u8], label: &str) -> Result<Cow<'a, [u8]>, String> {
    if is_spirv(source) {
        // The smallest module is the five-word header.
        if source.len() < 20 || source.len() % 4 != 0 {
            return Err(format!("{label} is not a complete SPIR-V module"));
        }
        return Ok(Cow::Borrowed(source));
    }
    let text = std::str::from_utf8(source)
        .map_err(|_| format!("{label} has an invalid SPIR-V magic number"))?;
    compile_wgsl_to_spirv(text)
        .map(Cow::Owned)
        .map_err(|error| format!("{label} is not SPIR-V and did not compile as WGSL: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT_WGSL: &str = include_str!("../../../cupid_gpu/shaders/wgsl/rect.wgsl");
    const RECT_SPIRV: &[u8] = include_bytes!("../shaders/vulkan/rect.spv");

    const TRIVIAL: &str = "@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {\n    return vec4<f32>(f32(i), 0.0, 0.0, 1.0);\n}\n@fragment fn fs_main() -> @location(0) vec4<f32> {\n    return vec4<f32>(1.0);\n}\n";

    #[test]
    fn compiles_wgsl_to_a_spirv_module() {
        let bytes = compile_wgsl_to_spirv(TRIVIAL).unwrap();
        assert_eq!(bytes.len() % 4, 0);
        assert!(is_spirv(&bytes));
    }

    #[test]
    fn runtime_output_matches_the_checked_in_module() {
        // The runtime path and the offline generator must agree byte for byte.
        assert_eq!(compile_wgsl_to_spirv(RECT_WGSL).unwrap().as_slice(), RECT_SPIRV);
    }

    #[test]
    fn invalid_wgsl_reports_an_error() {
        let error = compile_wgsl_to_spirv("fn main( {").unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn a_shader_that_fails_validation_is_rejected() {
        let error = compile_wgsl_to_spirv(
            "@fragment fn fs_main() -> @location(0) vec4<f32> { return 1.0; }",
        )
        .unwrap_err();
        assert!(!error.is_empty());
    }

    #[test]
    fn magic_detection_needs_a_full_word() {
        assert!(is_spirv(&0x0723_0203_u32.to_le_bytes()));
        assert!(!is_spirv(&[0x03, 0x02, 0x23]));
        assert!(!is_spirv(&[]));
        assert!(!is_spirv(b"@vertex fn"));
    }

    #[test]
    fn spirv_passes_through_without_a_copy() {
        let resolved = resolve_spirv(RECT_SPIRV, "rect").unwrap();
        assert!(matches!(resolved, Cow::Borrowed(_)));
        assert_eq!(&*resolved, RECT_SPIRV);
    }

    #[test]
    fn wgsl_text_is_compiled() {
        let resolved = resolve_spirv(TRIVIAL.as_bytes(), "trivial").unwrap();
        assert!(is_spirv(&resolved));
    }

    #[test]
    fn truncated_spirv_is_not_mistaken_for_wgsl() {
        let truncated = &RECT_SPIRV[..RECT_SPIRV.len() - 1];
        let error = resolve_spirv(truncated, "rect").unwrap_err();
        assert!(error.contains("complete SPIR-V"), "{error}");
        let tiny = 0x0723_0203_u32.to_le_bytes();
        assert!(resolve_spirv(&tiny, "tiny").unwrap_err().contains("complete SPIR-V"));
    }

    #[test]
    fn bytes_that_are_neither_report_the_label() {
        let error = resolve_spirv(&[0xff, 0xfe, 0xfd], "bad").unwrap_err();
        assert!(error.contains("bad"), "{error}");
        let error = resolve_spirv(b"not wgsl at all", "junk").unwrap_err();
        assert!(error.contains("junk"), "{error}");
    }
}
