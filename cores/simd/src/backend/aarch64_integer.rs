use core::arch::aarch64::{
    uint8x16_t, uint16x8_t, vaddq_u8, vaddq_u16, vbslq_u8, vbslq_u16, vcgtq_u8, vcgtq_u16,
    vcombine_u8, vdupq_n_u8, vdupq_n_u16, vget_high_u8, vget_high_u16, vget_low_u8, vget_low_u16,
    vld1q_u8, vld1q_u16, vmaxvq_u8, vmaxvq_u16, vmaxvq_u32, vmovn_u16, vmull_u8, vmull_u16,
    vmulq_u16, vst1q_u8, vst1q_u16, vsubq_u8, vsubq_u16,
};

#[cfg(not(feature = "force-scalar"))]
use core::arch::aarch64::vrshrn_n_u16;

use super::Neon;
use super::integer::{check_add_overflow, check_mul_overflow, check_sub_overflow};
use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdMul, SimdSelect, SimdSub};

#[cfg(not(feature = "force-scalar"))]
impl Neon {
    #[inline(always)]
    pub(crate) fn normalize_coverage_16<const SHIFT: i32>(counts: &mut [u8; 16]) {
        // SAFETY: NEON is baseline. The array supplies sixteen readable and
        // writable bytes at scalar alignment. Callers use SHIFT = 4 or 6;
        // count * 255 + rounding is at most 65057, which fits in u16.
        unsafe {
            let values = vld1q_u8(counts.as_ptr());
            let factor = core::arch::aarch64::vdup_n_u8(255);
            let low = vmull_u8(vget_low_u8(values), factor);
            let high = vmull_u8(vget_high_u8(values), factor);
            let result = vcombine_u8(vrshrn_n_u16::<SHIFT>(low), vrshrn_n_u16::<SHIFT>(high));
            vst1q_u8(counts.as_mut_ptr(), result);
        }
    }
}

macro_rules! impl_unsigned {
    ($number:ty, $lanes:expr, $vector:ty, $load:ident, $store:ident,
        $splat:ident, $add:ident, $sub:ident, $gt:ident, $max:ident, $select:ident) => {
        impl Simd<$number, $lanes> for Neon {
            type Vector = $vector;
            type Mask = $vector;

            #[inline(always)]
            fn load(values: &[$number; $lanes]) -> Self::Vector {
                // SAFETY: NEON is baseline on AArch64. The array supplies a
                // full vector; the load permits scalar alignment.
                unsafe { $load(values.as_ptr()) }
            }

            #[inline(always)]
            fn store(value: Self::Vector, out: &mut [$number; $lanes]) {
                // SAFETY: NEON is baseline and the exclusive array reference
                // supplies a full writable vector at scalar alignment.
                unsafe { $store(out.as_mut_ptr(), value) }
            }

            #[inline(always)]
            fn splat(value: $number) -> Self::Vector {
                // SAFETY: NEON is baseline on AArch64.
                unsafe { $splat(value) }
            }
        }

        impl SimdAdd<$number, $lanes> for Neon {
            #[inline(always)]
            fn add(left: Self::Vector, right: Self::Vector) -> Self::Vector {
                // SAFETY: NEON is baseline on AArch64.
                unsafe {
                    let sum = $add(left, right);
                    check_add_overflow($max($gt(left, sum)) != 0);
                    sum
                }
            }
        }

        impl SimdSub<$number, $lanes> for Neon {
            #[inline(always)]
            fn sub(left: Self::Vector, right: Self::Vector) -> Self::Vector {
                // SAFETY: NEON is baseline on AArch64.
                unsafe {
                    check_sub_overflow($max($gt(right, left)) != 0);
                    $sub(left, right)
                }
            }
        }

        impl SimdCompare<$number, $lanes> for Neon {
            #[inline(always)]
            fn gt(left: Self::Vector, right: Self::Vector) -> Self::Mask {
                // SAFETY: NEON is baseline on AArch64.
                unsafe { $gt(left, right) }
            }
        }

        impl SimdSelect<$number, $lanes> for Neon {
            #[inline(always)]
            fn select(mask: Self::Mask, yes: Self::Vector, no: Self::Vector) -> Self::Vector {
                // SAFETY: NEON is baseline. Comparison masks have either all
                // one or all zero bits in each lane.
                unsafe { $select(mask, yes, no) }
            }
        }
    };
}

impl_unsigned!(
    u8, 16, uint8x16_t, vld1q_u8, vst1q_u8, vdupq_n_u8, vaddq_u8, vsubq_u8, vcgtq_u8, vmaxvq_u8,
    vbslq_u8
);
impl_unsigned!(
    u16,
    8,
    uint16x8_t,
    vld1q_u16,
    vst1q_u16,
    vdupq_n_u16,
    vaddq_u16,
    vsubq_u16,
    vcgtq_u16,
    vmaxvq_u16,
    vbslq_u16
);

impl SimdMul<u8, 16> for Neon {
    #[inline(always)]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is baseline on AArch64. Widen before multiplying so
        // overflow can be detected before narrowing to the low byte.
        unsafe {
            let low = vmull_u8(vget_low_u8(left), vget_low_u8(right));
            let high = vmull_u8(vget_high_u8(left), vget_high_u8(right));
            check_mul_overflow(vmaxvq_u16(low).max(vmaxvq_u16(high)) > u16::from(u8::MAX));
            vcombine_u8(vmovn_u16(low), vmovn_u16(high))
        }
    }
}

impl SimdMul<u16, 8> for Neon {
    #[inline(always)]
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector {
        // SAFETY: NEON is baseline on AArch64. The widened products identify
        // overflow while the result retains the low sixteen bits per lane.
        unsafe {
            let low = vmull_u16(vget_low_u16(left), vget_low_u16(right));
            let high = vmull_u16(vget_high_u16(left), vget_high_u16(right));
            check_mul_overflow(vmaxvq_u32(low).max(vmaxvq_u32(high)) > u32::from(u16::MAX));
            vmulq_u16(left, right)
        }
    }
}
