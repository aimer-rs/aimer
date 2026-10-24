use aimer_widget::base::Color;

pub use aimer_macro::Animatable;

/// A value that can be interpolated from `self` toward another value.
///
/// The framework provides implementations for common numeric types, colors,
/// and tuples. User-defined structs can derive this trait to interpolate every
/// field recursively:
///
/// ```
/// use aimer_animation::Animatable;
///
/// #[derive(Debug, PartialEq, Animatable)]
/// struct Offset {
///     x: f32,
///     y: f32,
/// }
///
/// let begin = Offset { x: 0.0, y: 4.0 };
/// let end = Offset { x: 8.0, y: 12.0 };
/// assert_eq!(begin.lerp(&end, 0.25), Offset { x: 2.0, y: 6.0 });
/// ```
///
/// Enums must select either `#[animatable(discrete)]`, which switches values
/// at `t = 0.5`, or `#[animatable(fieldwise)]`, which recursively interpolates
/// matching variants and uses that same switch for different variants.
/// Discrete switching selects the source only when `t < 0.5`; at the midpoint,
/// above it, or for `NaN`, it selects the target. Implement the trait manually
/// when variants need a custom mapping.
///
/// Implementations receive `t` unchanged. Each field's implementation decides
/// how endpoints, extrapolation, and non-finite factors behave.
pub trait Animatable {
    /// Interpolates from `self` to `other` by factor `t`.
    ///
    /// `t` is conventionally in `0.0..=1.0`, but the trait does not clamp or
    /// reject extrapolated or non-finite values.
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

impl Animatable for f32 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Animatable for f64 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let t = t as f64;
        self + (other - self) * t
    }
}

impl Animatable for i32 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (*self as f32 + (*other - *self) as f32 * t).round() as i32
    }
}

impl Animatable for i64 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (*self as f64 + (*other - *self) as f64 * t as f64).round() as i64
    }
}

impl Animatable for u8 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (*self as f32 + (*other as f32 - *self as f32) * t).round() as u8
    }
}
impl Animatable for Color {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        let rgba = self.to_rgba();
        let rgba2 = other.to_rgba();

        let red = rgba.0.lerp(&rgba2.0, t);
        let green = rgba.1.lerp(&rgba2.1, t);
        let blue = rgba.2.lerp(&rgba2.2, t);
        let alpha = rgba.3.lerp(&rgba2.3, t);

        Color::Rgba(red, green, blue, alpha)
    }
}

/// 2D point / offset interpolation.
impl Animatable for (f32, f32) {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (self.0.lerp(&other.0, t), self.1.lerp(&other.1, t))
    }
}

/// RGBA color component interpolation (each component 0.0–1.0).
impl Animatable for (f32, f32, f32, f32) {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (
            self.0.lerp(&other.0, t),
            self.1.lerp(&other.1, t),
            self.2.lerp(&other.2, t),
            self.3.lerp(&other.3, t),
        )
    }
}

/// 3D vector interpolation.
impl Animatable for (f32, f32, f32) {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        (
            self.0.lerp(&other.0, t),
            self.1.lerp(&other.1, t),
            self.2.lerp(&other.2, t),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;

    #[derive(Debug, PartialEq, Animatable)]
    struct Insets {
        horizontal: f32,
        vertical: f32,
    }

    #[derive(Debug, PartialEq, Animatable)]
    struct Point(f32, f32);

    #[derive(Debug, PartialEq, Animatable)]
    struct Marker;

    #[derive(Debug, PartialEq)]
    struct NonClone(f32);

    impl Animatable for NonClone {
        fn lerp(&self, other: &Self, t: f32) -> Self {
            Self(self.0.lerp(&other.0, t))
        }
    }

    #[derive(Debug, PartialEq, Animatable)]
    struct GenericValue<T>
    where
        T: PartialEq,
    {
        value: T,
    }

    #[derive(Debug, PartialEq, Animatable)]
    struct Composite {
        insets: Insets,
        offset: (f32, f32),
        color: Color,
    }

    #[derive(Debug, PartialEq, Animatable)]
    #[animatable(discrete)]
    enum LabelState {
        Hidden,
        Visible(String),
    }

    #[derive(Debug, PartialEq, Animatable)]
    #[animatable(fieldwise)]
    enum Shape {
        Circle { radius: f32 },
        Point(f32, f32),
        Hidden,
    }

    #[test]
    fn derived_named_struct_interpolates_every_field() {
        let begin = Insets {
            horizontal: 2.0,
            vertical: 10.0,
        };
        let end = Insets {
            horizontal: 6.0,
            vertical: 18.0,
        };

        assert_eq!(
            begin.lerp(&end, 0.25),
            Insets {
                horizontal: 3.0,
                vertical: 12.0,
            }
        );
    }

    #[test]
    fn derived_tuple_struct_interpolates_fields_by_position() {
        let begin = Point(4.0, 8.0);
        let end = Point(12.0, 28.0);

        assert_eq!(begin.lerp(&end, 0.5), Point(8.0, 18.0));
    }

    #[test]
    fn derived_unit_struct_returns_the_unit_value() {
        assert_eq!(Marker.lerp(&Marker, f32::NAN), Marker);
    }

    #[test]
    fn derived_nested_and_generic_structs_preserve_recursive_bounds() {
        let begin = GenericValue {
            value: NonClone(2.0),
        };
        let end = GenericValue {
            value: NonClone(10.0),
        };
        assert_eq!(
            begin.lerp(&end, 0.25),
            GenericValue {
                value: NonClone(4.0),
            }
        );

        let begin = Composite {
            insets: Insets {
                horizontal: 0.0,
                vertical: 10.0,
            },
            offset: (2.0, 4.0),
            color: Color::Rgba(0, 20, 40, 60),
        };
        let end = Composite {
            insets: Insets {
                horizontal: 8.0,
                vertical: 18.0,
            },
            offset: (6.0, 12.0),
            color: Color::Rgba(100, 120, 140, 160),
        };
        assert_eq!(
            begin.lerp(&end, 0.5),
            Composite {
                insets: Insets {
                    horizontal: 4.0,
                    vertical: 14.0,
                },
                offset: (4.0, 8.0),
                color: Color::Rgba(50, 70, 90, 110),
            }
        );
    }

    #[test]
    fn derived_struct_delegates_endpoint_extrapolation_and_non_finite_factors() {
        let begin = Insets {
            horizontal: 2.0,
            vertical: 10.0,
        };
        let end = Insets {
            horizontal: 6.0,
            vertical: 18.0,
        };

        assert_eq!(begin.lerp(&end, 0.0), begin);
        assert_eq!(begin.lerp(&end, 1.0), end);
        assert_eq!(
            begin.lerp(&end, -1.0),
            Insets {
                horizontal: -2.0,
                vertical: 2.0,
            }
        );
        assert_eq!(
            begin.lerp(&end, 2.0),
            Insets {
                horizontal: 10.0,
                vertical: 26.0,
            }
        );
        let nan = begin.lerp(&end, f32::NAN);
        assert!(nan.horizontal.is_nan());
        assert!(nan.vertical.is_nan());
    }

    #[test]
    fn derived_discrete_enum_switches_at_the_midpoint_without_animatable_fields() {
        let begin = LabelState::Hidden;
        let end = LabelState::Visible(String::from("ready"));

        assert_eq!(begin.lerp(&end, 0.499), LabelState::Hidden);
        assert_eq!(
            begin.lerp(&end, 0.5),
            LabelState::Visible(String::from("ready"))
        );
        assert_eq!(begin.lerp(&end, f32::NEG_INFINITY), LabelState::Hidden);
        assert_eq!(
            begin.lerp(&end, f32::INFINITY),
            LabelState::Visible(String::from("ready"))
        );
        assert_eq!(
            begin.lerp(&end, f32::NAN),
            LabelState::Visible(String::from("ready"))
        );
    }

    #[test]
    fn derived_fieldwise_enum_interpolates_matching_variants_and_switches_others() {
        let small = Shape::Circle { radius: 2.0 };
        let large = Shape::Circle { radius: 10.0 };
        assert_eq!(small.lerp(&large, 0.25), Shape::Circle { radius: 4.0 });

        let point = Shape::Point(8.0, 12.0);
        assert_eq!(small.lerp(&point, 0.499), Shape::Circle { radius: 2.0 });
        assert_eq!(small.lerp(&point, 0.5), Shape::Point(8.0, 12.0));

        let other_point = Shape::Point(12.0, 20.0);
        assert_eq!(point.lerp(&other_point, 0.5), Shape::Point(10.0, 16.0));

        assert_eq!(small.lerp(&large, -1.0), Shape::Circle { radius: -6.0 });
        let nan = small.lerp(&large, f32::NAN);
        assert!(matches!(nan, Shape::Circle { radius } if radius.is_nan()));
        assert_eq!(small.lerp(&point, f32::NAN), Shape::Point(8.0, 12.0));

        let hidden = Shape::Hidden;
        assert_eq!(hidden.lerp(&Shape::Hidden, f32::NAN), Shape::Hidden);
    }

    #[test]
    fn test_f32_lerp() {
        assert!((0.0f32.lerp(&10.0, 0.0) - 0.0).abs() < 1e-9);
        assert!((0.0f32.lerp(&10.0, 0.5) - 5.0).abs() < 1e-9);
        assert!((0.0f32.lerp(&10.0, 1.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_f64_lerp() {
        assert!((0.0f64.lerp(&10.0, 0.5) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_i32_lerp_rounds() {
        assert_eq!(0i32.lerp(&10, 0.5), 5);
        assert_eq!(0i32.lerp(&10, 0.3), 3);
    }

    #[test]
    fn test_tuple2_lerp() {
        let a = (0.0, 0.0);
        let b = (10.0, 20.0);
        let r = a.lerp(&b, 0.5);
        assert!((r.0 - 5.0).abs() < 1e-9);
        assert!((r.1 - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_tuple4_lerp() {
        let a = (0.0, 0.0, 0.0, 1.0);
        let b = (1.0, 1.0, 1.0, 0.0);
        let r = a.lerp(&b, 0.5);
        assert!((r.0 - 0.5).abs() < 1e-9);
        assert!((r.3 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_u8_lerp() {
        assert_eq!(0u8.lerp(&255, 0.5), 128);
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_animatable_interpolation() {
        const MEASURED: usize = 4_096;
        const WARMUP: usize = 512;
        const ROUNDS: usize = 7;

        let mut samples = Vec::with_capacity(ROUNDS);
        let mut checksum = 0.0;
        for _ in 0..ROUNDS {
            let begin = 0.0f32;
            let end = 100.0f32;
            for index in 0..WARMUP {
                let t = index as f32 / WARMUP as f32;
                checksum = black_box(checksum + begin.lerp(&end, t));
            }

            let start = Instant::now();
            for index in 0..MEASURED {
                let t = index as f32 / MEASURED as f32;
                checksum = black_box(checksum + begin.lerp(&end, t));
            }
            samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
        }
        samples.sort_by(f64::total_cmp);
        println!(
            "f32: p50 {:.3} us, p95 {:.3} us",
            samples[ROUNDS / 2],
            samples[(ROUNDS * 95).div_ceil(100) - 1]
        );

        let mut tuple_samples = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            let begin = (0.0f32, 10.0, 20.0, 1.0);
            let end = (100.0f32, 110.0, 120.0, 0.0);
            for index in 0..WARMUP {
                let t = index as f32 / WARMUP as f32;
                let value = begin.lerp(&end, t);
                checksum = black_box(checksum + value.0 + value.1 + value.2 + value.3);
            }

            let start = Instant::now();
            for index in 0..MEASURED {
                let t = index as f32 / MEASURED as f32;
                let value = begin.lerp(&end, t);
                checksum = black_box(checksum + value.0 + value.1 + value.2 + value.3);
            }
            tuple_samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
        }
        tuple_samples.sort_by(f64::total_cmp);
        println!(
            "tuple4: p50 {:.3} us, p95 {:.3} us",
            tuple_samples[ROUNDS / 2],
            tuple_samples[(ROUNDS * 95).div_ceil(100) - 1]
        );
        assert!(checksum.is_finite());
    }
}
