//! Numeric values accepted by range controls.

/// A scalar value that can be represented on a range-control track.
///
/// Implement this trait for application-specific scalar types when a slider
/// should expose values other than the built-in integer and floating-point
/// types. The conversion is used only for track geometry and snapping; the
/// callback and public value methods continue to use the original `T`.
pub trait RangeValue: Copy + PartialOrd + 'static {
    /// Converts this value to the floating-point representation used by the
    /// geometry and validation policy.
    fn to_f64(self) -> f64;

    /// Converts a canonical track value back into this scalar type.
    fn from_f64(value: f64) -> Self;

    /// Returns the additive identity used by [`Slider::new`](crate::Slider::new)
    /// and [`RangeSlider::new`](crate::RangeSlider::new).
    fn zero() -> Self;

    /// Returns the multiplicative identity used by the default range and step.
    fn one() -> Self;
}

macro_rules! impl_range_value_float {
    ($type:ty) => {
        impl RangeValue for $type {
            #[inline]
            fn to_f64(self) -> f64 {
                self as f64
            }

            #[inline]
            fn from_f64(value: f64) -> Self {
                value as $type
            }

            #[inline]
            fn zero() -> Self {
                0.0
            }

            #[inline]
            fn one() -> Self {
                1.0
            }
        }
    };
}

macro_rules! impl_range_value_integer {
    ($type:ty) => {
        impl RangeValue for $type {
            #[inline]
            fn to_f64(self) -> f64 {
                self as f64
            }

            #[inline]
            fn from_f64(value: f64) -> Self {
                value.round() as $type
            }

            #[inline]
            fn zero() -> Self {
                0
            }

            #[inline]
            fn one() -> Self {
                1
            }
        }
    };
}

impl_range_value_float!(f32);
impl_range_value_float!(f64);
impl_range_value_integer!(i8);
impl_range_value_integer!(i16);
impl_range_value_integer!(i32);
impl_range_value_integer!(i64);
impl_range_value_integer!(i128);
impl_range_value_integer!(isize);
impl_range_value_integer!(u8);
impl_range_value_integer!(u16);
impl_range_value_integer!(u32);
impl_range_value_integer!(u64);
impl_range_value_integer!(u128);
impl_range_value_integer!(usize);
