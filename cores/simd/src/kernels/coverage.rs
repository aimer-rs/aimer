#[cfg(all(
    not(feature = "force-scalar"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
use crate::simd_traits::{Simd, SimdAdd};

/// Adds a constant to every byte in place using the target's SIMD backend.
///
/// Uses NEON on AArch64, SSE2 on x86_64, and scalar addition elsewhere or with
/// `force-scalar`. Short buffers and partial vectors use scalar arithmetic.
/// No allocation or vector alignment is required. Adding zero does no work.
///
/// # Panics
///
/// Panics on overflow when Rust overflow checks are enabled; otherwise each
/// byte wraps. An overflow panic may leave preceding elements written.
///
/// ```
/// let mut counts = [0, 4, 8];
/// aimer_simd::kernels::add_u8_in_place(&mut counts, 4);
/// assert_eq!(counts, [4, 8, 12]);
/// ```
#[inline]
pub fn add_u8_in_place(values: &mut [u8], amount: u8) {
    if amount == 0 {
        return;
    }
    #[cfg(all(not(feature = "force-scalar"), target_arch = "aarch64"))]
    {
        add_constant_with::<crate::backend::Neon>(values, amount);
    }
    #[cfg(all(not(feature = "force-scalar"), target_arch = "x86_64"))]
    {
        add_constant_with::<crate::backend::Sse2>(values, amount);
    }
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    {
        for value in values {
            *value += amount;
        }
    }
}

#[cfg(all(
    not(feature = "force-scalar"),
    any(target_arch = "aarch64", target_arch = "x86_64")
))]
#[inline]
fn add_constant_with<K: Simd<u8, 16> + SimdAdd<u8, 16>>(values: &mut [u8], amount: u8) {
    let vector_end = values.len() / 16 * 16;
    let mut index = 0;
    if vector_end != 0 {
        let addend = K::splat(amount);
        let pointer = values.as_mut_ptr();
        while index < vector_end {
            // SAFETY: each exclusive array borrow covers sixteen valid bytes
            // before vector_end. The input is loaded before the same block is
            // overwritten; no input/output slice aliasing is introduced.
            let block = unsafe { &mut *pointer.add(index).cast::<[u8; 16]>() };
            let sum = K::add(K::load(block), addend);
            K::store(sum, block);
            index += 16;
        }
    }
    for value in &mut values[index..] {
        *value += amount;
    }
}

/// Converts coverage sample counts into rounded alpha bytes in place.
///
/// Each result is `(count * 255 + grid² / 2) / grid²`, truncated to a byte.
/// A zero grid writes zero. Counts above grid² retain byte truncation rather
/// than saturation. Grids four and eight use widened integer SIMD arithmetic
/// on AArch64/x86_64 unless `force-scalar` is enabled; other grids use scalar
/// arithmetic. No allocation or vector alignment is required.
///
/// # Panics
///
/// Panics if squaring `sample_grid` overflows with Rust overflow checks enabled.
/// With checks disabled, the squared grid wraps.
///
/// ```
/// let mut counts = [0, 8, 16];
/// aimer_simd::kernels::normalize_coverage_u8(&mut counts, 4);
/// assert_eq!(counts, [0, 128, 255]);
/// ```
#[inline]
pub fn normalize_coverage_u8(counts: &mut [u8], sample_grid: u32) {
    let denominator = sample_grid * sample_grid;
    match denominator {
        16 => normalize_power_of_two::<4>(counts),
        64 => normalize_power_of_two::<6>(counts),
        _ => {
            for count in counts {
                *count = (u32::from(*count) * 255 + denominator / 2)
                    .checked_div(denominator)
                    .unwrap_or(0) as u8;
            }
        }
    }
}

#[inline]
fn normalize_power_of_two<const SHIFT: i32>(counts: &mut [u8]) {
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    let mut index = 0;
    #[cfg(any(
        feature = "force-scalar",
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    ))]
    let index = 0;
    #[cfg(all(
        not(feature = "force-scalar"),
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    {
        let vector_end = counts.len() / 16 * 16;
        let pointer = counts.as_mut_ptr();
        while index < vector_end {
            // SAFETY: index is below vector_end, rounded down from the slice
            // length. Each exclusive array borrow covers sixteen valid bytes.
            let block = unsafe { &mut *pointer.add(index).cast::<[u8; 16]>() };
            #[cfg(target_arch = "aarch64")]
            crate::backend::Neon::normalize_coverage_16::<SHIFT>(block);
            #[cfg(target_arch = "x86_64")]
            crate::backend::Sse2::normalize_coverage_16::<SHIFT>(block);
            index += 16;
        }
    }
    for count in &mut counts[index..] {
        *count = ((u32::from(*count) * 255 + (1 << (SHIFT - 1))) >> SHIFT) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::{add_u8_in_place, normalize_coverage_u8};

    #[test]
    fn in_place_add_preserves_tails_alignment_and_overflow_behavior() {
        use std::hint::black_box;
        use std::panic::{AssertUnwindSafe, catch_unwind};

        for len in 0..=65 {
            for offset in 0..16 {
                let mut storage = vec![99; len + 16];
                storage[offset..offset + len].fill(247);
                add_u8_in_place(&mut storage[offset..offset + len], 8);
                assert_eq!(&storage[offset..offset + len], vec![255; len]);
                assert!(
                    storage[..offset]
                        .iter()
                        .chain(&storage[offset + len..])
                        .all(|&x| x == 99)
                );
                add_u8_in_place(&mut storage[offset..offset + len], 0);
                assert_eq!(&storage[offset..offset + len], vec![255; len]);
            }
        }
        let panics = catch_unwind(|| black_box(255_u8) + black_box(1_u8)).is_err();
        for len in [1, 15, 16, 17, 32, 33, 65] {
            for index in 0..len {
                let mut values = vec![0; len];
                values[index] = 255;
                let result = catch_unwind(AssertUnwindSafe(|| add_u8_in_place(&mut values, 1)));
                assert_eq!(result.is_err(), panics, "length {len}, index {index}");
                if result.is_ok() {
                    let mut expected = vec![1; len];
                    expected[index] = 0;
                    assert_eq!(values, expected);
                }
            }
        }
    }

    #[test]
    fn coverage_normalization_matches_integer_rounding_for_every_byte() {
        for grid in [0_u32, 1, 2, 3, 4, 8, 15] {
            let denominator = grid * grid;
            for len in [0, 1, 7, 15, 16, 17, 31, 32, 33, 256, 257] {
                for offset in 0..16 {
                    let mut storage = vec![99; len + 16];
                    let values = &mut storage[offset..offset + len];
                    for (i, count) in values.iter_mut().enumerate() {
                        *count = i as u8;
                    }
                    let expected: Vec<u8> = values
                        .iter()
                        .map(|&count| {
                            ((u32::from(count) * 255 + denominator / 2)
                                .checked_div(denominator)
                                .unwrap_or(0)) as u8
                        })
                        .collect();
                    normalize_coverage_u8(values, grid);
                    assert_eq!(
                        values, expected,
                        "grid {grid}, length {len}, offset {offset}"
                    );
                    assert!(
                        storage[..offset]
                            .iter()
                            .chain(&storage[offset + len..])
                            .all(|&x| x == 99)
                    );
                }
            }
        }
    }
}
