use core::arch::x86_64::{
    __m128i, _mm_add_epi8, _mm_add_epi16, _mm_and_si128, _mm_andnot_si128, _mm_cmpeq_epi16,
    _mm_cmpgt_epi8, _mm_cmpgt_epi16, _mm_loadu_si128, _mm_movemask_epi8, _mm_mulhi_epu16,
    _mm_mullo_epi16, _mm_or_si128, _mm_packus_epi16, _mm_set1_epi8, _mm_set1_epi16,
    _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_sub_epi8, _mm_sub_epi16,
    _mm_unpackhi_epi8, _mm_unpacklo_epi8, _mm_xor_si128,
};

use super::Sse2;
use super::integer::{check_add_overflow, check_mul_overflow, check_sub_overflow};
use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdMul, SimdSelect, SimdSub};

#[cfg(not(feature = "force-scalar"))]
impl Sse2 {
    #[inline(always)]
    pub(crate) fn normalize_coverage_16<const SHIFT: i32>(counts: &mut [u8; 16]) {
        // SAFETY: SSE2 is baseline. The array supplies sixteen readable and
        // writable bytes at scalar alignment. Callers use SHIFT = 4 or 6;
        // count * 255 + rounding is at most 65057, which fits in u16.
        unsafe {
            let values = _mm_loadu_si128(counts.as_ptr().cast());
            let zero = _mm_setzero_si128();
            let factor = _mm_set1_epi16(255);
            let round = _mm_set1_epi16(1 << (SHIFT - 1));
            let low = _mm_add_epi16(
                _mm_mullo_epi16(_mm_unpacklo_epi8(values, zero), factor),
                round,
            );
            let high = _mm_add_epi16(
                _mm_mullo_epi16(_mm_unpackhi_epi8(values, zero), factor),
                round,
            );
            let mask = _mm_set1_epi16(255);
            // Mask after shifting so PACKUS preserves byte truncation even
            // when a caller supplies a count greater than the sample count.
            let result = _mm_packus_epi16(
                _mm_and_si128(_mm_srli_epi16::<SHIFT>(low), mask),
                _mm_and_si128(_mm_srli_epi16::<SHIFT>(high), mask),
            );
            _mm_storeu_si128(counts.as_mut_ptr().cast(), result);
        }
    }
}

macro_rules! impl_unsigned {
    ($number:ty, $signed:ty, $lanes:expr, $splat:ident, $add:ident, $sub:ident, $gt:ident) => {
        impl Simd<$number, $lanes> for Sse2 {
            type Vector = __m128i;
            type Mask = __m128i;

            #[inline(always)]
            fn load(values: &[$number; $lanes]) -> Self::Vector {
                // SAFETY: SSE2 is baseline on x86_64. The array supplies all
                // sixteen bytes and this intrinsic permits scalar alignment.
                unsafe { _mm_loadu_si128(values.as_ptr().cast()) }
            }

            #[inline(always)]
            fn store(value: Self::Vector, out: &mut [$number; $lanes]) {
                // SAFETY: SSE2 is baseline and the exclusive array supplies
                // sixteen writable bytes at scalar alignment.
                unsafe { _mm_storeu_si128(out.as_mut_ptr().cast(), value) }
            }

            #[inline(always)]
            fn splat(value: $number) -> Self::Vector {
                // SAFETY: SSE2 is baseline; the cast preserves the lane bits.
                unsafe { $splat(value as $signed) }
            }
        }

        impl SimdAdd<$number, $lanes> for Sse2 {
            #[inline(always)]
            fn add(left: Self::Vector, right: Self::Vector) -> Self::Vector {
                // SAFETY: SSE2 is baseline on x86_64.
                unsafe {
                    let sum = $add(left, right);
                    let overflow = <Self as SimdCompare<$number, $lanes>>::gt(left, sum);
                    check_add_overflow(_mm_movemask_epi8(overflow) != 0);
                    sum
                }
            }
        }

        impl SimdSub<$number, $lanes> for Sse2 {
            #[inline(always)]
            fn sub(left: Self::Vector, right: Self::Vector) -> Self::Vector {
                // SAFETY: SSE2 is baseline on x86_64.
                unsafe {
                    let overflow = <Self as SimdCompare<$number, $lanes>>::gt(right, left);
                    check_sub_overflow(_mm_movemask_epi8(overflow) != 0);
                    $sub(left, right)
                }
            }
        }

        impl SimdCompare<$number, $lanes> for Sse2 {
            #[inline(always)]
            fn gt(left: Self::Vector, right: Self::Vector) -> Self::Mask {
                // SAFETY: SSE2 is baseline. Flipping the sign bit maps unsigned
                // order onto the signed comparison supported by SSE2.
                unsafe {
                    let bias = $splat(<$signed>::MIN);
                    $gt(_mm_xor_si128(left, bias), _mm_xor_si128(right, bias))
                }
            }
        }

        impl SimdSelect<$number, $lanes> for Sse2 {
            #[inline(always)]
            fn select(mask: Self::Mask, yes: Self::Vector, no: Self::Vector) -> Self::Vector {
                // SAFETY: SSE2 is baseline. Comparison masks have either all
                // one or all zero bits in each lane.
                unsafe { _mm_or_si128(_mm_and_si128(mask, yes), _mm_andnot_si128(mask, no)) }
            }
        }
    };
}

impl_unsigned!(
    u8,
    i8,
    16,
    _mm_set1_epi8,
    _mm_add_epi8,
    _mm_sub_epi8,
    _mm_cmpgt_epi8
);
impl_unsigned!(
    u16,
    i16,
    8,
    _mm_set1_epi16,
    _mm_add_epi16,
    _mm_sub_epi16,
    _mm_cmpgt_epi16
);

impl SimdMul<u8, 16> for Sse2 {
    #[inline(always)]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: SSE2 is baseline on x86_64. SSE2 has no byte multiply, so
        // zero-extend both halves to u16 before multiplying.
        unsafe {
            let zero = _mm_setzero_si128();
            let low = _mm_mullo_epi16(
                _mm_unpacklo_epi8(left, zero),
                _mm_unpacklo_epi8(right, zero),
            );
            let high = _mm_mullo_epi16(
                _mm_unpackhi_epi8(left, zero),
                _mm_unpackhi_epi8(right, zero),
            );
            let upper_bits = _mm_or_si128(_mm_srli_epi16::<8>(low), _mm_srli_epi16::<8>(high));
            check_mul_overflow(_mm_movemask_epi8(_mm_cmpeq_epi16(upper_bits, zero)) != 0xffff);
            // PACKUS treats its input as signed and saturates. Mask to the
            // low byte first so it produces wrapping rather than saturation.
            let byte_mask = _mm_set1_epi16(255);
            _mm_packus_epi16(
                _mm_and_si128(low, byte_mask),
                _mm_and_si128(high, byte_mask),
            )
        }
    }
}

impl SimdMul<u16, 8> for Sse2 {
    #[inline(always)]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: SSE2 is baseline. A nonzero high half of the unsigned
        // product indicates overflow; MULLO retains the low sixteen bits.
        unsafe {
            let high = _mm_mulhi_epu16(left, right);
            check_mul_overflow(
                _mm_movemask_epi8(_mm_cmpeq_epi16(high, _mm_setzero_si128())) != 0xffff,
            );
            _mm_mullo_epi16(left, right)
        }
    }
}
