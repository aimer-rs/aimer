use crate::bounds::Bounds;
use crate::position::Vec2d;
use crate::size::ResolvedSize;
use std::cell::Cell;

///
/// Represents a dimension type that can be used to define sizes in different
/// units.
///
/// The `Dimension` enum supports two primary units:
/// - Pixels (`Px`) - Defines the size in absolute pixel values.
/// - Percent (`Percent`) - Defines the size as a percentage of a parent
///   container.
///
/// It also supports an automatic size value, `Auto`, which can be used when the
/// size should be determined by layout or content rules.
///
/// `Px` and `Percent` use `f32` as their underlying type on every supported
/// target, including WebAssembly.
///
/// ### Traits
/// The `Dimension` enum derives the following traits:
/// - `Debug`: Enables formatted output for debugging purposes.
/// - `Clone`: Allows for creating a duplicate of `Dimension` instances.
/// - `Copy`: Allows for `Dimension` to be copied rather than moved.
/// - `PartialEq`: Enables equality comparisons between `Dimension` instances.
///
/// # Example
/// ```
/// use self::aimer_attribute::Dimension;
///
/// let px_dimension = Dimension::Px(100.0);
/// let percent_dimension = Dimension::Percent(50.0);
/// let auto_dimension = Dimension::Auto;
///
/// assert!(px_dimension != percent_dimension);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dimension {
    Px(f32),
    Percent(f32),
    Auto,
}
#[allow(clippy::derivable_impls)]
impl Default for Dimension {
    fn default() -> Self {
        Self::Auto
    }
}

impl From<f32> for Dimension {
    fn from(v: f32) -> Self {
        Self::Px(v)
    }
}

impl From<i32> for Dimension {
    fn from(v: i32) -> Self {
        Self::Px(v as f32)
    }
}

impl From<u32> for Dimension {
    fn from(v: u32) -> Self {
        Self::Px(v as f32)
    }
}

impl Dimension {
    pub fn resolve(&self, parent_value: f32, scale: f32) -> f32 {
        match self {
            Dimension::Px(v) => v * scale,
            Dimension::Percent(p) => parent_value * (p / 100.0),
            Dimension::Auto => parent_value,
        }
    }
}

/// Builds a [`Dimension`] straight from a number literal.
///
/// Implemented for every primitive integer and float, so `16.px()` and
/// `50.pc()` read as the unit they stand for, without naming `Dimension` or
/// wrapping the value in a variant.
///
/// The number becomes the variant's `f32` with an `as` cast. Integers beyond
/// `f32`'s 24-bit mantissa lose their low bits, and a value outside the `f32`
/// range saturates to infinity, exactly as `value as f32` does. Neither is
/// checked, because layout sizes are far inside that range.
///
/// # Examples
///
/// ```
/// use aimer_attribute::{Dimension, dimension::NumberExt};
///
/// assert_eq!(16.px(), Dimension::Px(16.0));
/// assert_eq!(50.pc(), Dimension::Percent(50.0));
/// assert_eq!(0.5f32.px(), Dimension::Px(0.5));
/// ```
pub trait NumberExt: Sized {
    /// Returns [`Dimension::Px`] holding this number, in logical pixels.
    fn px(self) -> Dimension;

    /// Returns [`Dimension::Percent`] holding this number, where `100` is the
    /// whole parent extent.
    fn pc(self) -> Dimension;
}

macro_rules! impl_number_ext {
    ($($number:ty),* $(,)?) => {$(
        impl NumberExt for $number {
            #[inline]
            fn px(self) -> Dimension {
                Dimension::Px(self as f32)
            }

            #[inline]
            fn pc(self) -> Dimension {
                Dimension::Percent(self as f32)
            }
        }
    )*};
}

impl_number_ext!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);

// `bound` is wrapped in a `Cell` so it can be mutated through a shared
// `&self` (during layout/draw) soundly. The previous version stored a bare
// `Option<Bounds>` and mutated it via a `&raw const ... as *mut` cast, which
// is undefined behaviour: the type was `Freeze`, so the optimizer was free to
// assume it never changed behind a `&self` and cache the stale value. In
// release/wasm-opt builds that meant `is_inside` kept reading the old `None`
// bounds after `save`, so hover hit-testing silently stopped working.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheBounds {
    bound: Cell<Option<Bounds>>,
}

impl CacheBounds {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            bound: Cell::new(None),
        }
    }

    pub const fn with_vec2d(vec2d: Vec2d) -> Self {
        Self {
            bound: Cell::new(Some(Bounds::new(vec2d.x, vec2d.y, 0.0, 0.0))),
        }
    }

    pub fn is_cached(&self) -> bool {
        self.bound.get().is_some()
    }

    pub fn get_bounds(&self) -> Option<Bounds> {
        self.bound.get()
    }

    pub fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bound.get().map(|b| {
            (
                Vec2d { x: b.x, y: b.y },
                Vec2d {
                    x: b.x + b.width,
                    y: b.y + b.height,
                },
            )
        })
    }

    pub fn set_bounds(&self, bounds: Bounds) {
        self.bound.set(Some(bounds));
    }

    /// Forgets the cached rectangle, so nothing is inside it.
    pub fn clear(&self) {
        self.bound.set(None);
    }

    pub fn set_size(&self, size: ResolvedSize) {
        if let Some(mut bound) = self.bound.get() {
            bound.width = size.width;
            bound.height = size.height;
            self.bound.set(Some(bound));
        }
    }

    pub fn save(&self, scale: f32, x: f32, y: f32, width: f32, height: f32) {
        let cache_x = x / scale;
        let cache_y = y / scale;
        let cache_w = width / scale;
        let cache_h = height / scale;
        let bound = Bounds::new(cache_x, cache_y, cache_w, cache_h);
        self.set_bounds(bound);
    }

    pub fn is_inside(&self, x: f32, y: f32) -> bool {
        let Some(bound) = self.bound.get() else {
            return false;
        };
        bound.x <= x && x <= bound.x + bound.width && bound.y <= y && y <= bound.y + bound.height
    }
}

#[cfg(test)]
mod number_ext_tests {
    use super::*;

    #[test]
    fn px_and_pc_build_the_matching_dimension() {
        assert_eq!(100.px(), Dimension::Px(100.0));
        assert_eq!(50.pc(), Dimension::Percent(50.0));
        assert_eq!(1.5f32.px(), Dimension::Px(1.5));
        assert_eq!(33.3f64.pc(), Dimension::Percent(33.3f64 as f32));
    }

    #[test]
    fn every_primitive_number_implements_the_trait() {
        assert_eq!(7i8.px(), Dimension::Px(7.0));
        assert_eq!(7i16.px(), Dimension::Px(7.0));
        assert_eq!(7i32.px(), Dimension::Px(7.0));
        assert_eq!(7i64.px(), Dimension::Px(7.0));
        assert_eq!(7i128.px(), Dimension::Px(7.0));
        assert_eq!(7isize.px(), Dimension::Px(7.0));
        assert_eq!(7u8.px(), Dimension::Px(7.0));
        assert_eq!(7u16.px(), Dimension::Px(7.0));
        assert_eq!(7u32.px(), Dimension::Px(7.0));
        assert_eq!(7u64.px(), Dimension::Px(7.0));
        assert_eq!(7u128.px(), Dimension::Px(7.0));
        assert_eq!(7usize.px(), Dimension::Px(7.0));
        assert_eq!(7f32.pc(), Dimension::Percent(7.0));
        assert_eq!(7f64.pc(), Dimension::Percent(7.0));
    }

    #[test]
    fn boundary_values_convert_without_panicking() {
        assert_eq!(0.px(), Dimension::Px(0.0));
        assert_eq!((-8).px(), Dimension::Px(-8.0));
        assert_eq!(u64::MAX.px(), Dimension::Px(u64::MAX as f32));
        assert_eq!(f64::MAX.px(), Dimension::Px(f32::INFINITY));
        match f32::NAN.pc() {
            Dimension::Percent(value) => assert!(value.is_nan()),
            other => panic!("expected Percent, got {other:?}"),
        }
    }

    #[test]
    fn resolves_like_the_equivalent_variant() {
        assert_eq!(40.px().resolve(200.0, 2.0), 80.0);
        assert_eq!(25.pc().resolve(200.0, 2.0), 50.0);
    }
}

#[cfg(test)]
mod cache_bounds_tests {
    use super::*;

    // Regression: `save` mutates through a shared `&self`, then `is_inside`
    // reads it back. With the old `Freeze` field + raw-pointer write the
    // optimizer cached the stale `None` in release/wasm-opt builds and hover
    // hit-testing silently failed. This must hold in release too.
    #[test]
    fn save_is_visible_to_is_inside_through_shared_ref() {
        let bounds = CacheBounds::new();
        assert!(
            !bounds.is_inside(50.0, 50.0),
            "empty bounds contain nothing"
        );

        bounds.save(1.0, 10.0, 20.0, 100.0, 50.0);
        assert!(
            bounds.is_inside(50.0, 40.0),
            "point inside must be detected after save"
        );
        assert!(
            !bounds.is_inside(200.0, 40.0),
            "point outside must be rejected"
        );
    }
}
