use crate::number::Number;
use crate::simd_traits::{Simd, SimdAdd, SimdCompare, SimdDiv, SimdMul, SimdSelect, SimdSub};

/// Scalar implementation of the SIMD backend traits.
///
/// This backend uses one lane and supports every type implementing [Number].
#[derive(Clone, Copy, Debug, Default)]
pub struct Scalar;

impl<T: Number> Simd<T, 1> for Scalar {
    type Vector = T;
    type Mask = bool;

    #[inline]
    fn load(values: &[T; 1]) -> Self::Vector {
        values[0]
    }

    #[inline]
    fn store(value: Self::Vector, out: &mut [T; 1]) {
        out[0] = value;
    }

    #[inline]
    fn splat(value: T) -> Self::Vector {
        value
    }
}

impl<T: Number> SimdAdd<T, 1> for Scalar {
    #[inline]
    fn add(left: T, right: T) -> T {
        left + right
    }
}

impl<T: Number> SimdSub<T, 1> for Scalar {
    #[inline]
    fn sub(left: T, right: T) -> T {
        left - right
    }
}

impl<T: Number> SimdMul<T, 1> for Scalar {
    #[inline]
    fn mul(left: T, right: T) -> T {
        left * right
    }
}

impl<T: Number> SimdDiv<T, 1> for Scalar {
    #[inline]
    fn div(left: T, right: T) -> T {
        left / right
    }
}

impl<T: Number> SimdCompare<T, 1> for Scalar {
    #[inline]
    fn gt(left: T, right: T) -> bool {
        left > right
    }
}

impl<T: Number> SimdSelect<T, 1> for Scalar {
    #[inline]
    fn select(mask: bool, yes: T, no: T) -> T {
        if mask { yes } else { no }
    }
}
