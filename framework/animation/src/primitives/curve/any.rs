//! The handle holders store.

use std::fmt;
use std::sync::Arc;

use super::builtin::{CubicBezier, Standard};
use super::Curve;

/// A curve of any kind, cheap to clone and compare.
///
/// This is what the framework stores wherever it holds a curve: controllers,
/// widgets, keyframes and transitions all accept `impl Curve` and keep an
/// `AnyCurve`. The standard curves are held inline, so storing one allocates
/// nothing and evaluating one takes no virtual call; only a custom curve is
/// boxed. `AnyCurve` is itself a [`Curve`], so it can be passed back to anything
/// that takes one.
///
/// Two handles are equal when they are the same standard curve, a Bézier with
/// the same control points, or clones of the same custom curve.
#[derive(Clone)]
pub struct AnyCurve(Repr);

#[derive(Clone)]
enum Repr {
    Standard(Standard),
    Bezier(CubicBezier),
    Custom(Arc<dyn Curve>),
}

impl AnyCurve {
    /// Wraps `curve`. Equivalent to [`Curve::into_any`].
    pub fn new(curve: impl Curve) -> Self {
        curve.into_any()
    }

    pub(super) fn standard(curve: Standard) -> Self {
        Self(Repr::Standard(curve))
    }

    pub(super) fn bezier(curve: CubicBezier) -> Self {
        Self(Repr::Bezier(curve))
    }

    pub(super) fn custom(curve: impl Curve) -> Self {
        Self(Repr::Custom(Arc::new(curve)))
    }

    /// Whether the curve is held without an allocation.
    #[cfg(test)]
    pub(super) fn is_inline(&self) -> bool {
        !matches!(self.0, Repr::Custom(_))
    }
}

impl Default for AnyCurve {
    /// Linear progress.
    fn default() -> Self {
        Self::standard(Standard::Linear)
    }
}

impl Curve for AnyCurve {
    fn transform(&self, t: f32) -> f32 {
        match &self.0 {
            Repr::Standard(curve) => curve.eval(t),
            Repr::Bezier(curve) => curve.transform(t),
            // A custom curve is promised progress inside 0..=1.
            Repr::Custom(curve) => curve.transform(t.clamp(0.0, 1.0)),
        }
    }

    fn is_valid(&self) -> bool {
        match &self.0 {
            Repr::Standard(_) => true,
            Repr::Bezier(curve) => curve.is_valid(),
            Repr::Custom(curve) => curve.is_valid(),
        }
    }

    fn into_any(self) -> AnyCurve {
        self
    }
}

impl PartialEq for AnyCurve {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Standard(a), Repr::Standard(b)) => a == b,
            (Repr::Bezier(a), Repr::Bezier(b)) => a == b,
            (Repr::Custom(a), Repr::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for AnyCurve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Standard(curve) => write!(f, "{curve:?}"),
            Repr::Bezier(c) => write!(f, "CubicBezier({}, {}, {}, {})", c.x1, c.y1, c.x2, c.y2),
            Repr::Custom(_) => f.write_str("Custom"),
        }
    }
}
