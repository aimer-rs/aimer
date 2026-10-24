// All operands are integer-valued f32s. Products and sums are at most
// 255³ + 127 = 16,581,502 < 2²⁴, so they are represented exactly. Division
// uses hardware FDIV/DIVPS, not reciprocal approximation. For division by
// 255, a nonintegral quotient is at least 1/255 from an integer, greater
// than one f32 ULP at the maximum quotient (65,025). For final division,
// the quotient is below 383 and the divisor at most 255. Thus truncating
// each quotient reproduces integer division without rounding corrections.
#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub(crate) use aarch64::composite;
#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub(crate) use x86_64::composite;

// Unpremultiplication has numerator <= 255² + 127 = 65,152, exactly
// representable in f32. With alpha 1..=254, a fractional quotient is at
// least 1/254 from an integer, larger than one f32 ULP (<= 1/256).
// Hardware division followed by truncation therefore reproduces integer
// division exactly; clamping to 255 preserves malformed channel handling.
#[cfg(target_arch = "aarch64")]
mod aarch64_unpremultiply;
#[cfg(target_arch = "aarch64")]
pub(crate) use aarch64_unpremultiply::unpremultiply;
#[cfg(target_arch = "x86_64")]
mod x86_64_unpremultiply;
#[cfg(target_arch = "x86_64")]
pub(crate) use x86_64_unpremultiply::unpremultiply;
