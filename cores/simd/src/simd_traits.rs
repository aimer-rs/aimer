//! Traits for portable SIMD backends and generic lane-wise algorithms.
//!
//! A backend implements [Simd] for a scalar number type and lane count, then
//! implements the operation traits it supports. Algorithms can be generic
//! over those capabilities without depending on a particular instruction set.

use crate::number::Number;

/// Shared vector representation and lane transfer operations for a backend.
///
/// Implementations must use a nonzero LANES count. Vector represents exactly
/// that many values of T; Mask is an opaque per-lane predicate used by
/// [SimdCompare] and [SimdSelect].
pub trait Simd<T: Number, const LANES: usize> {
    /// Backend representation of LANES packed scalar values.
    type Vector: Copy;

    /// Backend representation of a per-lane comparison result.
    type Mask: Copy;

    /// Loads exactly LANES values into a backend vector.
    fn load(values: &[T; LANES]) -> Self::Vector;

    /// Stores exactly LANES values from a backend vector.
    fn store(value: Self::Vector, out: &mut [T; LANES]);

    /// Creates a vector with value in every lane.
    fn splat(value: T) -> Self::Vector;
}

/// Lane-wise addition for a Simd backend.
pub trait SimdAdd<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Adds corresponding lanes using the scalar addition semantics of T.
    fn add(left: Self::Vector, right: Self::Vector) -> Self::Vector;
}

/// Lane-wise subtraction for a Simd backend.
pub trait SimdSub<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Subtracts corresponding lanes using the scalar subtraction semantics
    /// of T.
    fn sub(left: Self::Vector, right: Self::Vector) -> Self::Vector;
}

/// Lane-wise multiplication for a Simd backend.
pub trait SimdMul<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Multiplies corresponding lanes using the scalar multiplication
    /// semantics of T. This operation does not imply fusion with addition.
    fn mul(left: Self::Vector, right: Self::Vector) -> Self::Vector;
}

/// Lane-wise division for a Simd backend.
pub trait SimdDiv<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Divides corresponding lanes using the scalar division semantics of T.
    fn div(left: Self::Vector, right: Self::Vector) -> Self::Vector;
}

/// Ordered greater-than comparison for a Simd backend.
pub trait SimdCompare<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Compares corresponding lanes. A floating-point lane containing NaN
    /// compares false.
    fn gt(left: Self::Vector, right: Self::Vector) -> Self::Mask;
}

/// Per-lane selection for a Simd backend.
pub trait SimdSelect<T: Number, const LANES: usize>: Simd<T, LANES> {
    /// Returns the yes lane where mask is true and the no lane otherwise.
    fn select(mask: Self::Mask, yes: Self::Vector, no: Self::Vector) -> Self::Vector;
}
