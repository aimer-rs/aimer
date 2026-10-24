use core::arch::asm;
use core::arch::x86_64::{
    __m128, _mm_add_ps, _mm_and_ps, _mm_andnot_ps, _mm_cmpgt_ps, _mm_div_ps, _mm_loadu_ps,
    _mm_mul_ps, _mm_or_ps, _mm_set1_ps, _mm_storeu_ps, _mm_sub_ps,
};

use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdDiv, SimdMul, SimdSelect, SimdSub};

/// x86_64 SSE2 backend for four f32, sixteen u8, or eight u16 lanes.
///
/// Integer arithmetic follows Rust's overflow-check setting: overflow panics
/// when checks are enabled, and wraps when they are disabled.
/// Unsigned lanes support addition, subtraction, multiplication, comparison,
/// and selection; integer division is not implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sse2;

impl Sse2 {
    /// Adds sixteen f32 values with four unaligned SSE loads and stores.
    ///
    /// # Safety
    ///
    /// Each input pointer must reference sixteen initialized f32 values. The
    /// output pointer must reference sixteen writable f32 values and must not
    /// alias either input range.
    #[inline(always)]
    pub(crate) unsafe fn add_block_16_f32(left: *const f32, right: *const f32, output: *mut f32) {
        let _left0: __m128;
        let _left1: __m128;
        let _left2: __m128;
        let _left3: __m128;
        let _right0: __m128;
        let _right1: __m128;
        let _right2: __m128;
        let _right3: __m128;

        // SAFETY: the caller guarantees all three sixteen-value ranges are
        // valid and disjoint. MOVUPS permits the alignment of f32 slices.
        // ADDPS updates floating-point status, so the block is not pure.
        unsafe {
            asm!(
                "movups {left0}, xmmword ptr [{left_ptr}]",
                "movups {left1}, xmmword ptr [{left_ptr} + 16]",
                "movups {left2}, xmmword ptr [{left_ptr} + 32]",
                "movups {left3}, xmmword ptr [{left_ptr} + 48]",
                "movups {right0}, xmmword ptr [{right_ptr}]",
                "movups {right1}, xmmword ptr [{right_ptr} + 16]",
                "movups {right2}, xmmword ptr [{right_ptr} + 32]",
                "movups {right3}, xmmword ptr [{right_ptr} + 48]",
                "addps {left0}, {right0}",
                "addps {left1}, {right1}",
                "addps {left2}, {right2}",
                "addps {left3}, {right3}",
                "movups xmmword ptr [{output_ptr}], {left0}",
                "movups xmmword ptr [{output_ptr} + 16], {left1}",
                "movups xmmword ptr [{output_ptr} + 32], {left2}",
                "movups xmmword ptr [{output_ptr} + 48], {left3}",
                left_ptr = in(reg) left,
                right_ptr = in(reg) right,
                output_ptr = in(reg) output,
                left0 = out(xmm_reg) _left0,
                left1 = out(xmm_reg) _left1,
                left2 = out(xmm_reg) _left2,
                left3 = out(xmm_reg) _left3,
                right0 = out(xmm_reg) _right0,
                right1 = out(xmm_reg) _right1,
                right2 = out(xmm_reg) _right2,
                right3 = out(xmm_reg) _right3,
                options(nostack),
            );
        }
    }
}

impl Simd<f32, 4> for Sse2 {
    type Vector = __m128;
    type Mask = __m128;

    #[inline(always)]
    fn load(values: &[f32; 4]) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support, and the array reference
        // provides four initialized values for the unaligned load.
        unsafe { _mm_loadu_ps(values.as_ptr()) }
    }

    #[inline(always)]
    fn store(value: Self::Vector, out: &mut [f32; 4]) {
        // SAFETY: x86_64 guarantees SSE support, and the array reference
        // provides space for four values for the unaligned store.
        unsafe { _mm_storeu_ps(out.as_mut_ptr(), value) }
    }

    #[inline]
    fn splat(value: f32) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_set1_ps(value) }
    }
}

impl SimdAdd<f32, 4> for Sse2 {
    #[inline(always)]
    fn add(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_add_ps(left, right) }
    }
}

impl SimdSub<f32, 4> for Sse2 {
    #[inline]
    fn sub(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_sub_ps(left, right) }
    }
}

impl SimdMul<f32, 4> for Sse2 {
    #[inline]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_mul_ps(left, right) }
    }
}

impl SimdDiv<f32, 4> for Sse2 {
    #[inline]
    fn div(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_div_ps(left, right) }
    }
}

impl SimdCompare<f32, 4> for Sse2 {
    #[inline]
    fn gt(left: Self::Vector, right: Self::Vector) -> Self::Mask {
        // SAFETY: x86_64 guarantees SSE support.
        unsafe { _mm_cmpgt_ps(left, right) }
    }
}

impl SimdSelect<f32, 4> for Sse2 {
    #[inline]
    fn select(mask: Self::Mask, yes: Self::Vector, no: Self::Vector) -> Self::Vector {
        // SAFETY: x86_64 guarantees SSE support. Comparison masks contain
        // either all one or all zero bits in each lane.
        unsafe { _mm_or_ps(_mm_and_ps(mask, yes), _mm_andnot_ps(mask, no)) }
    }
}
