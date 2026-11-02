// Shared by `build.rs` (via `include!`) and the runtime, so both sides derive
// the same key from HLSL text. FNV-1a is enough: the key only has to separate
// the handful of built-in sources, and a collision with custom HLSL would need
// that source to hash identically to a built-in one.

/// Hashes HLSL source text into the key under which its bytecode is embedded.
pub(crate) fn hash_source(source: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in source.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}
