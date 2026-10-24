//! Scalar and architecture-specific SIMD backends.

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
mod integer;
mod scalar;

pub use scalar::Scalar;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use aarch64::Neon;

#[cfg(target_arch = "aarch64")]
mod aarch64_integer;

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use x86_64::Sse2;

#[cfg(target_arch = "x86_64")]
mod x86_64_integer;

#[cfg(test)]
mod integer_tests;

#[cfg(all(
    not(feature = "force-scalar"),
    target_endian = "little",
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
pub(crate) mod rgba;
