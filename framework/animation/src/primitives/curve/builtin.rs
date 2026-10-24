//! The standard curves.

use super::{AnyCurve, Curve, CurveDescriptor};

/// A standard curve, as the handle stores it and the portable format names it.
///
/// This is private: the public surface is the curve types below. It exists so
/// that holding a standard curve takes no allocation and evaluating it takes no
/// virtual call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Standard {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    Decelerate,
    BounceOut,
    BounceIn,
    BounceInOut,
    ElasticIn,
    ElasticOut,
    ElasticInOut,
    FastOutSlowIn,
    LinearOutSlowIn,
    FastOutLinearIn,
}

/// The portable tag of a cubic Bézier.
pub(super) const BEZIER_TAG: i64 = 4;

impl Standard {
    /// The tag the portable widget format uses. These never change.
    pub(super) const fn tag(self) -> i64 {
        match self {
            Self::Linear => 0,
            Self::EaseIn => 1,
            Self::EaseOut => 2,
            Self::EaseInOut => 3,
            // 4 is the cubic Bézier.
            Self::Decelerate => 5,
            Self::BounceOut => 6,
            Self::BounceIn => 7,
            Self::BounceInOut => 8,
            Self::ElasticIn => 9,
            Self::ElasticOut => 10,
            Self::ElasticInOut => 11,
            Self::FastOutSlowIn => 12,
            Self::LinearOutSlowIn => 13,
            Self::FastOutLinearIn => 14,
        }
    }

    pub(super) const fn from_tag(tag: i64) -> Option<Self> {
        Some(match tag {
            0 => Self::Linear,
            1 => Self::EaseIn,
            2 => Self::EaseOut,
            3 => Self::EaseInOut,
            5 => Self::Decelerate,
            6 => Self::BounceOut,
            7 => Self::BounceIn,
            8 => Self::BounceInOut,
            9 => Self::ElasticIn,
            10 => Self::ElasticOut,
            11 => Self::ElasticInOut,
            12 => Self::FastOutSlowIn,
            13 => Self::LinearOutSlowIn,
            14 => Self::FastOutLinearIn,
            _ => return None,
        })
    }

    pub(super) const fn descriptor(self) -> CurveDescriptor {
        CurveDescriptor { tag: self.tag(), control_points: None }
    }

    /// Evaluates the curve at `t`, clamped to `0.0..=1.0`.
    pub(super) fn eval(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::EaseIn => t * t * t,
            Self::EaseOut => {
                let inv = 1.0 - t;
                1.0 - inv * inv * inv
            }
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let inv = -2.0 * t + 2.0;
                    1.0 - inv * inv * inv / 2.0
                }
            }
            Self::Decelerate => {
                let inv = 1.0 - t;
                1.0 - inv * inv
            }
            Self::BounceOut => bounce_out(t),
            Self::BounceIn => 1.0 - bounce_out(1.0 - t),
            Self::BounceInOut => {
                if t < 0.5 {
                    (1.0 - bounce_out(1.0 - 2.0 * t)) / 2.0
                } else {
                    (1.0 + bounce_out(2.0 * t - 1.0)) / 2.0
                }
            }
            Self::ElasticIn => elastic_in(t),
            Self::ElasticOut => elastic_out(t),
            Self::ElasticInOut => {
                if t < 0.5 {
                    elastic_in(2.0 * t) / 2.0
                } else {
                    elastic_out(2.0 * t - 1.0) / 2.0 + 0.5
                }
            }
            // Material Design curves: pre-computed cubic Bézier control points.
            Self::FastOutSlowIn => cubic_bezier_y_for_x(t, 0.4, 0.0, 0.2, 1.0),
            Self::LinearOutSlowIn => cubic_bezier_y_for_x(t, 0.0, 0.0, 0.2, 1.0),
            Self::FastOutLinearIn => cubic_bezier_y_for_x(t, 0.4, 0.0, 1.0, 1.0),
        }
    }
}

macro_rules! standard_curves {
    ($($(#[$doc:meta])* $name:ident, $constant:ident => $variant:ident;)*) => {$(
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name;

        #[doc = concat!("The [`", stringify!($name), "`] curve.")]
        pub const $constant: $name = $name;

        impl Curve for $name {
            fn transform(&self, t: f32) -> f32 {
                Standard::$variant.eval(t)
            }

            fn descriptor(&self) -> Option<CurveDescriptor> {
                Some(Standard::$variant.descriptor())
            }

            fn into_any(self) -> AnyCurve {
                AnyCurve::standard(Standard::$variant)
            }
        }
    )*};
}

standard_curves! {
    /// No easing: progress is returned unchanged.
    Linear, LINEAR => Linear;
    /// Starts slowly and accelerates (`t³`).
    EaseIn, EASE_IN => EaseIn;
    /// Starts quickly and decelerates (`1 - (1 - t)³`).
    EaseOut, EASE_OUT => EaseOut;
    /// Slow at both ends, quick in the middle.
    EaseInOut, EASE_IN_OUT => EaseInOut;
    /// Decelerates (`1 - (1 - t)²`).
    Decelerate, DECELERATE => Decelerate;
    /// Bounces against the end.
    BounceOut, BOUNCE_OUT => BounceOut;
    /// Bounces away from the start.
    BounceIn, BOUNCE_IN => BounceIn;
    /// Bounces at both the start and the end.
    BounceInOut, BOUNCE_IN_OUT => BounceInOut;
    /// Springs back before moving forward, like a stretched band let go.
    ElasticIn, ELASTIC_IN => ElasticIn;
    /// Overshoots the end and springs back.
    ElasticOut, ELASTIC_OUT => ElasticOut;
    /// Springy at both the start and the end.
    ElasticInOut, ELASTIC_IN_OUT => ElasticInOut;
    /// The Material Design standard curve: cubic Bézier `(0.4, 0.0, 0.2, 1.0)`.
    FastOutSlowIn, FAST_OUT_SLOW_IN => FastOutSlowIn;
    /// The Material Design decelerate curve: cubic Bézier `(0.0, 0.0, 0.2, 1.0)`.
    LinearOutSlowIn, LINEAR_OUT_SLOW_IN => LinearOutSlowIn;
    /// The Material Design accelerate curve: cubic Bézier `(0.4, 0.0, 1.0, 1.0)`.
    FastOutLinearIn, FAST_OUT_LINEAR_IN => FastOutLinearIn;
}

/// A cubic Bézier easing defined by its two control points, as in CSS
/// `cubic-bezier(x1, y1, x2, y2)`.
///
/// The curve starts at `(0, 0)` and ends at `(1, 1)`. Control points must be
/// finite; [`is_valid`](Curve::is_valid) reports whether they are.
///
/// # Examples
///
/// ```
/// use aimer_animation::{Curve, curves};
///
/// let curve = curves::cubic_bezier(0.17, 0.74, 0.30, 1.0);
/// assert!(curve.transform(0.5) > 0.5);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// Creates a [`CubicBezier`] curve from its control points.
pub const fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> CubicBezier {
    CubicBezier { x1, y1, x2, y2 }
}

impl CubicBezier {
    pub(super) fn control_points(self) -> [f64; 4] {
        [self.x1 as f64, self.y1 as f64, self.x2 as f64, self.y2 as f64]
    }
}

impl Curve for CubicBezier {
    fn transform(&self, t: f32) -> f32 {
        cubic_bezier_y_for_x(t.clamp(0.0, 1.0), self.x1, self.y1, self.x2, self.y2)
    }

    fn descriptor(&self) -> Option<CurveDescriptor> {
        Some(CurveDescriptor { tag: BEZIER_TAG, control_points: Some(self.control_points()) })
    }

    fn is_valid(&self) -> bool {
        self.x1.is_finite() && self.y1.is_finite() && self.x2.is_finite() && self.y2.is_finite()
    }

    fn into_any(self) -> AnyCurve {
        AnyCurve::bezier(self)
    }
}

fn bounce_out(t: f32) -> f32 {
    const N1: f32 = 7.5625;
    const D1: f32 = 2.75;
    if t < 1.0 / D1 {
        N1 * t * t
    } else if t < 2.0 / D1 {
        let t = t - 1.5 / D1;
        N1 * t * t + 0.75
    } else if t < 2.5 / D1 {
        let t = t - 2.25 / D1;
        N1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / D1;
        N1 * t * t + 0.984375
    }
}

fn elastic_in(t: f32) -> f32 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let p = 0.3;
    let s = p / 4.0;
    let t = t - 1.0;
    -(2.0f32.powf(10.0 * t) * ((t - s) * std::f32::consts::TAU / p).sin())
}

fn elastic_out(t: f32) -> f32 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let p = 0.3;
    let s = p / 4.0;
    2.0f32.powf(-10.0 * t) * ((t - s) * std::f32::consts::TAU / p).sin() + 1.0
}

/// Approximate cubic bezier: find y for a given x using Newton's method.
fn cubic_bezier_y_for_x(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    // Find t such that bezier_x(t) ≈ x, then return bezier_y(t).
    let mut t = x; // initial guess
    for _ in 0..8 {
        let bx = bezier(t, x1, x2);
        let dx = bezier_derivative(t, x1, x2);
        if dx.abs() < 1e-12 {
            break;
        }
        t -= (bx - x) / dx;
        t = t.clamp(0.0, 1.0);
    }
    bezier(t, y1, y2)
}

/// Evaluate cubic bezier at parameter t with control points p1, p2 (p0=0,
/// p3=1).
fn bezier(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * t * p1 + 3.0 * inv * t * t * p2 + t * t * t
}

/// Derivative of the cubic bezier.
fn bezier_derivative(t: f32, p1: f32, p2: f32) -> f32 {
    let inv = 1.0 - t;
    3.0 * inv * inv * p1 + 6.0 * inv * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}
