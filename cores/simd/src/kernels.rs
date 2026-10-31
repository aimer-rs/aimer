use core::fmt;

use crate::number::Number;
use crate::simd_traits::{Simd, SimdAdd};

mod unsigned;
pub use unsigned::{add_u8, add_u16};
mod unpremultiply;
pub use unpremultiply::unpremultiply_rgba8;
mod rgba;
pub use rgba::composite_rgba8;
mod coverage;
pub use coverage::{add_u8_in_place, normalize_coverage_u8};

#[cfg(any(
    feature = "force-scalar",
    not(any(target_arch = "aarch64", target_arch = "x86_64"))
))]
use crate::backend::Scalar;

/// The input and output slices passed to an elementwise kernel have different
/// lengths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LengthMismatch {
    /// Length of the left input.
    pub left: usize,
    /// Length of the right input.
    pub right: usize,
    /// Length of the output.
    pub output: usize,
}

impl fmt::Display for LengthMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "slice lengths differ: left {}, right {}, output {}",
            self.left, self.right, self.output
        )
    }
}

impl core::error::Error for LengthMismatch {}

#[inline]
fn validate_lengths(left: usize, right: usize, output: usize) -> Result<(), LengthMismatch> {
    if left != right || left != output {
        return Err(LengthMismatch {
            left,
            right,
            output,
        });
    }
    Ok(())
}

#[inline]
fn add_f32_tail(left: &[f32], right: &[f32], output: &mut [f32], start: usize) {
    for index in start..left.len() {
        output[index] = left[index] + right[index];
    }
}

#[inline(always)]
unsafe fn load_vector_chunk<T, K, const LANES: usize>(input: *const T) -> K::Vector
where
    T: Number,
    K: Simd<T, LANES>,
{
    // SAFETY: the caller guarantees input addresses LANES initialized values.
    let lanes = unsafe { &*input.cast::<[T; LANES]>() };
    <K as Simd<T, LANES>>::load(lanes)
}

/// # Safety
///
/// Output must be valid, aligned, and exclusively writable for LANES values.
#[inline(always)]
unsafe fn store_vector_chunk<T, K, const LANES: usize>(output: *mut T, value: K::Vector)
where
    T: Number,
    K: Simd<T, LANES>,
{
    // SAFETY: the caller guarantees output addresses LANES writable values.
    let lanes = unsafe { &mut *output.cast::<[T; LANES]>() };
    <K as Simd<T, LANES>>::store(value, lanes);
}

/// # Safety
///
/// Each input must be valid and aligned for LANES initialized values. Output
/// must be valid and aligned for LANES writable values, with no aliasing to
/// either input range.
#[inline(always)]
unsafe fn add_vector_chunk<T, K, const LANES: usize>(
    left: *const T,
    right: *const T,
    output: *mut T,
) where
    T: Number,
    K: Simd<T, LANES> + SimdAdd<T, LANES>,
{
    // SAFETY: upheld by this function's caller.
    let left_vector = unsafe { load_vector_chunk::<T, K, LANES>(left) };
    // SAFETY: upheld by this function's caller.
    let right_vector = unsafe { load_vector_chunk::<T, K, LANES>(right) };
    let sum = <K as SimdAdd<T, LANES>>::add(left_vector, right_vector);
    // SAFETY: upheld by this function's caller.
    unsafe { store_vector_chunk::<T, K, LANES>(output, sum) };
}

/// Adds two slices with the specified backend.
///
/// The backend processes complete vectors, and the remaining elements use
/// scalar addition. All slices must have the same length.
///
/// Returns [LengthMismatch] when the slice lengths differ.
///
/// # Panics
///
/// Panics if LANES is zero.
pub fn add_with<T, K, const LANES: usize>(
    left: &[T],
    right: &[T],
    output: &mut [T],
) -> Result<(), LengthMismatch>
where
    T: Number,
    K: Simd<T, LANES> + SimdAdd<T, LANES>,
{
    assert!(LANES > 0, "SIMD lane count must be nonzero");

    validate_lengths(left.len(), right.len(), output.len())?;

    let vector_end = left.len() / LANES * LANES;
    let mut index = 0;
    let left_ptr = left.as_ptr();
    let right_ptr = right.as_ptr();
    let output_ptr = output.as_mut_ptr();
    if let Some(block_lanes) = LANES.checked_mul(4) {
        let unrolled_end = vector_end / block_lanes * block_lanes;
        while index < unrolled_end {
            // SAFETY: this block spans block_lanes elements and index is below
            // unrolled_end, so all input ranges fit in their slices.
            unsafe {
                let left0 = load_vector_chunk::<T, K, LANES>(left_ptr.add(index));
                let left1 = load_vector_chunk::<T, K, LANES>(left_ptr.add(index + LANES));
                let left2 = load_vector_chunk::<T, K, LANES>(left_ptr.add(index + LANES * 2));
                let left3 = load_vector_chunk::<T, K, LANES>(left_ptr.add(index + LANES * 3));
                let right0 = load_vector_chunk::<T, K, LANES>(right_ptr.add(index));
                let right1 = load_vector_chunk::<T, K, LANES>(right_ptr.add(index + LANES));
                let right2 = load_vector_chunk::<T, K, LANES>(right_ptr.add(index + LANES * 2));
                let right3 = load_vector_chunk::<T, K, LANES>(right_ptr.add(index + LANES * 3));

                let sum0 = <K as SimdAdd<T, LANES>>::add(left0, right0);
                let sum1 = <K as SimdAdd<T, LANES>>::add(left1, right1);
                let sum2 = <K as SimdAdd<T, LANES>>::add(left2, right2);
                let sum3 = <K as SimdAdd<T, LANES>>::add(left3, right3);

                // SAFETY: all output ranges fit the same block and the input
                // and output slices cannot alias under safe Rust borrowing.
                store_vector_chunk::<T, K, LANES>(output_ptr.add(index), sum0);
                store_vector_chunk::<T, K, LANES>(output_ptr.add(index + LANES), sum1);
                store_vector_chunk::<T, K, LANES>(output_ptr.add(index + LANES * 2), sum2);
                store_vector_chunk::<T, K, LANES>(output_ptr.add(index + LANES * 3), sum3);
            }
            index += block_lanes;
        }
    }

    while index < vector_end {
        // SAFETY: index is below vector_end, so one full vector fits in each
        // slice, and the output borrow remains exclusive.
        unsafe {
            add_vector_chunk::<T, K, LANES>(
                left_ptr.add(index),
                right_ptr.add(index),
                output_ptr.add(index),
            );
        }
        index += LANES;
    }

    for index in vector_end..left.len() {
        output[index] = left[index] + right[index];
    }

    Ok(())
}

/// Adds two f32 slices using the SIMD backend selected for the current target.
///
/// Complete vectors use the target backend when available; any remaining
/// elements use scalar addition. All slices must have the same length.
///
/// Returns [LengthMismatch] when the slice lengths differ.
#[inline]
pub fn add_f32(left: &[f32], right: &[f32], output: &mut [f32]) -> Result<(), LengthMismatch> {
    add_f32_selected(left, right, output)
}

#[cfg(feature = "force-scalar")]
#[inline]
fn add_f32_selected(left: &[f32], right: &[f32], output: &mut [f32]) -> Result<(), LengthMismatch> {
    add_with::<f32, Scalar, 1>(left, right, output)
}

#[cfg(all(not(feature = "force-scalar"), target_arch = "aarch64"))]
#[inline]
fn add_f32_selected(left: &[f32], right: &[f32], output: &mut [f32]) -> Result<(), LengthMismatch> {
    validate_lengths(left.len(), right.len(), output.len())?;

    let block_end = left.len() / 16 * 16;
    let left_ptr = left.as_ptr();
    let right_ptr = right.as_ptr();
    let output_ptr = output.as_mut_ptr();
    let mut index = 0;
    while index < block_end {
        // SAFETY: each loop iteration covers sixteen elements before block_end,
        // which is rounded down from all three equal-length slices.
        unsafe {
            crate::backend::Neon::add_block_16_f32(
                left_ptr.add(index),
                right_ptr.add(index),
                output_ptr.add(index),
            );
        }
        index += 16;
    }

    add_f32_tail(left, right, output, block_end);
    Ok(())
}

#[cfg(all(not(feature = "force-scalar"), target_arch = "x86_64"))]
#[inline]
fn add_f32_selected(left: &[f32], right: &[f32], output: &mut [f32]) -> Result<(), LengthMismatch> {
    validate_lengths(left.len(), right.len(), output.len())?;

    let block_end = left.len() / 16 * 16;
    let left_ptr = left.as_ptr();
    let right_ptr = right.as_ptr();
    let output_ptr = output.as_mut_ptr();
    let mut index = 0;
    while index < block_end {
        // SAFETY: each loop iteration covers sixteen elements before block_end,
        // which is rounded down from all three equal-length slices.
        unsafe {
            crate::backend::Sse2::add_block_16_f32(
                left_ptr.add(index),
                right_ptr.add(index),
                output_ptr.add(index),
            );
        }
        index += 16;
    }

    add_f32_tail(left, right, output, block_end);
    Ok(())
}

#[cfg(all(
    not(feature = "force-scalar"),
    not(any(target_arch = "aarch64", target_arch = "x86_64"))
))]
#[inline]
fn add_f32_selected(left: &[f32], right: &[f32], output: &mut [f32]) -> Result<(), LengthMismatch> {
    add_with::<f32, Scalar, 1>(left, right, output)
}

#[cfg(test)]
mod tests {
    use core::hint::black_box;
    use aimer_time::Instant;

    use crate::backend::Scalar;
    use crate::kernels::{LengthMismatch, add_f32, add_with};
    use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdDiv, SimdMul, SimdSelect, SimdSub};

    #[test]
    fn scalar_backend_adds_every_number_type() {
        macro_rules! assert_scalar_add {
            ($number:ty) => {{
                let mut output = [0 as $number; 3];
                add_with::<$number, Scalar, 1>(
                    &[1 as $number, 3 as $number, 5 as $number],
                    &[2 as $number; 3],
                    &mut output,
                )
                .unwrap();
                assert_eq!(output, [3 as $number, 5 as $number, 7 as $number]);
            }};
        }

        assert_scalar_add!(i8);
        assert_scalar_add!(i16);
        assert_scalar_add!(i32);
        assert_scalar_add!(i64);
        assert_scalar_add!(i128);
        assert_scalar_add!(isize);
        assert_scalar_add!(u8);
        assert_scalar_add!(u16);
        assert_scalar_add!(u32);
        assert_scalar_add!(u64);
        assert_scalar_add!(u128);
        assert_scalar_add!(usize);
        assert_scalar_add!(f32);
        assert_scalar_add!(f64);
    }

    #[test]
    fn f32_add_handles_empty_full_and_tail_lengths() {
        for len in [0, 1, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33] {
            let left: Vec<f32> = (0..len).map(|index| index as f32).collect();
            let right = vec![2.0; len];
            let mut output = vec![0.0; len];

            add_f32(&left, &right, &mut output).unwrap();

            let expected: Vec<f32> = (0..len).map(|index| index as f32 + 2.0).collect();
            assert_eq!(output, expected, "length {len}");
        }
    }

    #[test]
    fn f32_add_accepts_f32_aligned_subslices() {
        const LEN: usize = 17;
        let mut left_storage = vec![-1.0; LEN + 4];
        let mut right_storage = vec![-1.0; LEN + 4];
        let mut output_storage = vec![-1.0; LEN + 4];

        fn offset_for_mod_16(storage: &[f32]) -> usize {
            let remainder = storage.as_ptr() as usize % 16;
            ((4 + 16 - remainder) % 16) / 4
        }

        let left_start = offset_for_mod_16(&left_storage);
        let right_start = offset_for_mod_16(&right_storage);
        let output_start = offset_for_mod_16(&output_storage);
        for index in 0..LEN {
            left_storage[left_start + index] = index as f32;
            right_storage[right_start + index] = 2.0;
        }

        let left = &left_storage[left_start..left_start + LEN];
        let right = &right_storage[right_start..right_start + LEN];
        let output = &mut output_storage[output_start..output_start + LEN];
        assert_eq!((left.as_ptr() as usize) % 16, 4);
        assert_eq!((right.as_ptr() as usize) % 16, 4);
        assert_eq!((output.as_ptr() as usize) % 16, 4);

        add_f32(left, right, output).unwrap();

        let expected: Vec<f32> = (0..LEN).map(|index| index as f32 + 2.0).collect();
        assert_eq!(output, expected);
    }

    #[test]
    fn add_rejects_mismatched_lengths_without_writing_output() {
        let mut output = [9.0, 9.0];

        let error = add_f32(&[1.0, 2.0], &[3.0], &mut output).unwrap_err();

        assert_eq!(
            error,
            LengthMismatch {
                left: 2,
                right: 1,
                output: 2,
            }
        );
        assert_eq!(output, [9.0, 9.0]);
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn native_backend_supports_lane_operations_and_tail_addition() {
        fn store<K: Simd<f32, 4>>(value: K::Vector) -> [f32; 4] {
            let mut output = [0.0; 4];
            <K as Simd<f32, 4>>::store(value, &mut output);
            output
        }

        fn verify_backend<K>()
        where
            K: Simd<f32, 4>
                + SimdAdd<f32, 4>
                + SimdSub<f32, 4>
                + SimdMul<f32, 4>
                + SimdDiv<f32, 4>
                + SimdCompare<f32, 4>
                + SimdSelect<f32, 4>,
        {
            let left = <K as Simd<f32, 4>>::load(&[6.0, 3.0, 8.0, -1.0]);
            let right = <K as Simd<f32, 4>>::load(&[2.0, 4.0, 2.0, -2.0]);

            assert_eq!(
                store::<K>(<K as SimdAdd<f32, 4>>::add(left, right)),
                [8.0, 7.0, 10.0, -3.0],
            );
            assert_eq!(
                store::<K>(<K as SimdSub<f32, 4>>::sub(left, right)),
                [4.0, -1.0, 6.0, 1.0],
            );
            assert_eq!(
                store::<K>(<K as SimdMul<f32, 4>>::mul(left, right)),
                [12.0, 12.0, 16.0, 2.0],
            );
            assert_eq!(
                store::<K>(<K as SimdDiv<f32, 4>>::div(left, right)),
                [3.0, 0.75, 4.0, 0.5],
            );

            let mask = <K as SimdCompare<f32, 4>>::gt(left, right);
            let selected = <K as SimdSelect<f32, 4>>::select(mask, left, right);
            assert_eq!(store::<K>(selected), [6.0, 4.0, 8.0, -1.0]);

            let mut output = [0.0; 5];
            add_with::<f32, K, 4>(
                &[1.0, 2.0, 3.0, 4.0, 5.0],
                &[10.0, 20.0, 30.0, 40.0, 50.0],
                &mut output,
            )
            .unwrap();
            assert_eq!(output, [11.0, 22.0, 33.0, 44.0, 55.0]);
        }

        #[cfg(target_arch = "aarch64")]
        verify_backend::<crate::backend::Neon>();
        #[cfg(target_arch = "x86_64")]
        verify_backend::<crate::backend::Sse2>();
    }

    fn measure_add_variant<F>(
        left: &[f32],
        right: &[f32],
        output: &mut [f32],
        mut kernel: F,
    ) -> (f64, f64, f32)
    where
        F: FnMut(&[f32], &[f32], &mut [f32]) -> Result<(), LengthMismatch>,
    {
        const MEASURED: usize = 128;
        const WARMUP: usize = 16;
        const ROUNDS: usize = 31;

        let mut samples = Vec::with_capacity(ROUNDS);
        let mut checksum = 0.0;

        for _ in 0..ROUNDS {
            for _ in 0..WARMUP {
                kernel(black_box(left), black_box(right), black_box(&mut *output)).unwrap();
                black_box(&*output);
                if !output.is_empty() {
                    checksum = black_box(checksum + output[output.len() / 2]);
                }
            }

            let start = Instant::now();
            for _ in 0..MEASURED {
                kernel(black_box(left), black_box(right), black_box(&mut *output)).unwrap();
                black_box(&*output);
                if !output.is_empty() {
                    checksum = black_box(checksum + output[output.len() / 2]);
                }
            }
            samples.push(start.elapsed().as_secs_f64() * 1e9 / MEASURED as f64);
        }

        samples.sort_by(f64::total_cmp);
        let p50 = samples[ROUNDS / 2];
        let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
        (p50, p95, checksum)
    }

    #[inline]
    fn scalar_add_reference(
        left: &[f32],
        right: &[f32],
        output: &mut [f32],
    ) -> Result<(), LengthMismatch> {
        if left.len() != right.len() || left.len() != output.len() {
            return Err(LengthMismatch {
                left: left.len(),
                right: right.len(),
                output: output.len(),
            });
        }

        for ((left, right), output) in left.iter().zip(right).zip(output) {
            *output = *left + *right;
        }

        Ok(())
    }

    #[test]
    #[ignore = "manual scalar versus SIMD kernel benchmark"]
    fn profile_add_f32_scalar_vs_simd() {
        for len in [0, 1, 2, 3, 4, 5, 8, 16, 64, 256, 1024, 4096] {
            let left: Vec<f32> = (0..len).map(|index| index as f32 * 0.5).collect();
            let right = vec![2.0; len];
            let mut scalar_output = vec![0.0; len];
            let mut simd_output = vec![0.0; len];

            let (scalar_p50, scalar_p95, scalar_checksum) =
                measure_add_variant(&left, &right, &mut scalar_output, scalar_add_reference);
            let (simd_p50, simd_p95, simd_checksum) =
                measure_add_variant(&left, &right, &mut simd_output, add_f32);

            assert_eq!(scalar_output, simd_output);
            assert!(scalar_checksum.is_finite() && simd_checksum.is_finite());
            println!(
                "len={len:4}: scalar p50={scalar_p50:8.2} ns p95={scalar_p95:8.2} ns; \
                 SIMD p50={simd_p50:8.2} ns p95={simd_p95:8.2} ns; speedup={:.2}x",
                scalar_p50 / simd_p50,
            );
        }
    }
}
