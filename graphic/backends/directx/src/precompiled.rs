//! Bytecode for the built-in HLSL, compiled by `build.rs` instead of at startup.

use crate::shader_hash::hash_source;

// `PRECOMPILED: &[(u64, &str, &str, &[u8])]` — source hash, entry point, shader
// target, DXBC. Empty when the build host cannot run FXC.
include!(concat!(env!("OUT_DIR"), "/precompiled_shaders.rs"));

/// Returns the build-time bytecode for `source`'s `entry_point`, if the source
/// is one of the built-in shaders.
///
/// Custom HLSL passed to `create_shader_module` never matches and is compiled
/// at runtime as before.
pub(crate) fn precompiled_bytecode(
    source: &str,
    entry_point: &str,
    target: &str,
) -> Option<&'static [u8]> {
    let key = hash_source(source);
    PRECOMPILED
        .iter()
        .find(|(hash, entry, tgt, _)| *hash == key && *entry == entry_point && *tgt == target)
        .map(|(_, _, _, bytecode)| *bytecode)
}
