use core::arch::aarch64::{
    float32x4_t, uint32x4_t, vbslq_f32, vcgtq_f32, vdivq_f32, vdupq_n_f32, vmulq_f32, vsubq_f32,
};
use core::arch::asm;

use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdDiv, SimdMul, SimdSelect, SimdSub};

/// AArch64 NEON backend for four f32, sixteen u8, or eight u16 lanes.
///
/// Integer arithmetic follows Rust's overflow-check setting: overflow panics
/// when checks are enabled, and wraps when they are disabled.
/// Unsigned lanes support addition, subtraction, multiplication, comparison,
/// and selection; integer division is not implemented.
#[derive(Clone, Copy, Debug, Default)]
pub struct Neon;

impl Neon {
    /// Adds sixteen f32 values with paired NEON loads and stores.
    ///
    /// # Safety
    ///
    /// Each input pointer must reference sixteen initialized f32 values. The
    /// output pointer must reference sixteen writable f32 values and must not
    /// alias either input range.
    #[inline(always)]
    pub(crate) unsafe fn add_block_16_f32(left: *const f32, right: *const f32, output: *mut f32) {
        let _left0: float32x4_t;
        let _left1: float32x4_t;
        let _left2: float32x4_t;
        let _left3: float32x4_t;
        let _right0: float32x4_t;
        let _right1: float32x4_t;
        let _right2: float32x4_t;
        let _right3: float32x4_t;

        // SAFETY: the caller guarantees that both inputs and the output cover
        // sixteen values. Paired loads/stores accept the f32 slice alignment.
        // FADD updates floating-point status, so the block is not marked pure.
        unsafe {
            asm!(
                "ldp {left0:q}, {left1:q}, [{left_ptr}]",
                "ldp {left2:q}, {left3:q}, [{left_ptr}, #32]",
                "ldp {right0:q}, {right1:q}, [{right_ptr}]",
                "ldp {right2:q}, {right3:q}, [{right_ptr}, #32]",
                "fadd {left0:v}.4s, {left0:v}.4s, {right0:v}.4s",
                "fadd {left1:v}.4s, {left1:v}.4s, {right1:v}.4s",
                "fadd {left2:v}.4s, {left2:v}.4s, {right2:v}.4s",
                "fadd {left3:v}.4s, {left3:v}.4s, {right3:v}.4s",
                "stp {left0:q}, {left1:q}, [{output_ptr}]",
                "stp {left2:q}, {left3:q}, [{output_ptr}, #32]",
                left_ptr = in(reg) left,
                right_ptr = in(reg) right,
                output_ptr = in(reg) output,
                left0 = out(vreg) _left0,
                left1 = out(vreg) _left1,
                left2 = out(vreg) _left2,
                left3 = out(vreg) _left3,
                right0 = out(vreg) _right0,
                right1 = out(vreg) _right1,
                right2 = out(vreg) _right2,
                right3 = out(vreg) _right3,
                options(nostack),
            );
        }
    }
}

impl Simd<f32, 4> for Neon {
    type Vector = float32x4_t;
    type Mask = uint32x4_t;

    #[inline(always)]
    fn load(values: &[f32; 4]) -> Self::Vector {
        let value: float32x4_t;
        // SAFETY: the array reference guarantees four initialized f32 values.
        unsafe {
            asm!(
                "ld1 {{{value:v}.4s}}, [{ptr}]",
                value = out(vreg) value,
                ptr = in(reg) values.as_ptr(),
                options(readonly, nostack),
            );
        }
        value
    }

    #[inline(always)]
    fn store(value: Self::Vector, out: &mut [f32; 4]) {
        // SAFETY: the array reference guarantees space for four f32 values.
        unsafe {
            asm!(
                "st1 {{{value:v}.4s}}, [{ptr}]",
                value = in(vreg) value,
                ptr = in(reg) out.as_mut_ptr(),
                options(nostack),
            );
        }
    }

    #[inline]
    fn splat(value: f32) -> Self::Vector {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vdupq_n_f32(value) }
    }
}

impl SimdAdd<f32, 4> for Neon {
    #[inline(always)]
    fn add(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        let sum: float32x4_t;
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        // Omit pure because floating-point status flags are observable state.
        unsafe {
            asm!(
                "fadd {sum:v}.4s, {left:v}.4s, {right:v}.4s",
                sum = out(vreg) sum,
                left = in(vreg) left,
                right = in(vreg) right,
                options(nomem, nostack),
            );
        }
        sum
    }
}

impl SimdSub<f32, 4> for Neon {
    #[inline]
    fn sub(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vsubq_f32(left, right) }
    }
}

impl SimdMul<f32, 4> for Neon {
    #[inline]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vmulq_f32(left, right) }
    }
}

impl SimdDiv<f32, 4> for Neon {
    #[inline]
    fn div(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vdivq_f32(left, right) }
    }
}

impl SimdCompare<f32, 4> for Neon {
    #[inline]
    fn gt(left: Self::Vector, right: Self::Vector) -> Self::Mask {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vcgtq_f32(left, right) }
    }
}

impl SimdSelect<f32, 4> for Neon {
    #[inline]
    fn select(mask: Self::Mask, yes: Self::Vector, no: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is part of the AArch64 baseline targeted by this module.
        unsafe { vbslq_f32(mask, yes, no) }
    }
}
