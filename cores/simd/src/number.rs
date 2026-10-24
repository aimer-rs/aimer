//! Scalar numeric types used for SIMD lanes.

use core::ops::{Add, Div, Mul, Sub};

/// A scalar number that can be used as a SIMD lane value.
///
/// Implementors support basic arithmetic and provide additive and
/// multiplicative identity values. Implementations are provided for the
/// built-in signed and unsigned integer types and for f32 and f64.
pub trait Number:
    Copy
    + PartialEq
    + PartialOrd
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
{
    /// The additive identity.
    const ZERO: Self;

    /// The multiplicative identity.
    const ONE: Self;
}

macro_rules! impl_number_for_primitives {
    ($($number:ty),+ $(,)?) => {
        $(
            impl Number for $number {
                const ZERO: Self = 0 as $number;
                const ONE: Self = 1 as $number;
            }
        )+
    };
}

impl_number_for_primitives!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64,
);

#[cfg(test)]
mod tests {
    use super::Number;

    fn exercise_arithmetic<T: Number>(left: T, right: T) -> (T, T, T, T) {
        (left + right, left - right, left * right, left / right)
    }

    #[test]
    fn f32_implements_number_contract() {
        assert_eq!(<f32 as Number>::ZERO, 0.0);
        assert_eq!(<f32 as Number>::ONE, 1.0);
        assert_eq!(exercise_arithmetic(8.0_f32, 2.0), (10.0, 6.0, 16.0, 4.0));
    }

    #[test]
    fn all_standard_numeric_primitives_implement_number() {
        macro_rules! assert_number_types {
            ($($number:ty),+ $(,)?) => {
                $(
                    assert_eq!(<$number as Number>::ZERO, 0 as $number);
                    assert_eq!(<$number as Number>::ONE, 1 as $number);
                    assert_eq!(
                        exercise_arithmetic(8 as $number, 2 as $number),
                        (10 as $number, 6 as $number, 16 as $number, 4 as $number),
                    );
                )+
            };
        }

        assert_number_types!(
            i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64,
        );
    }
}
