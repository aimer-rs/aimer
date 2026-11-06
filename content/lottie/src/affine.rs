use crate::model::Vec2;

/// A 2D affine transform in SVG's `matrix(a b c d e f)` layout:
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    #[inline]
    pub fn translate(x: f32, y: f32) -> Affine {
        Affine { e: x, f: y, ..Self::IDENTITY }
    }

    #[inline]
    pub fn scale(x: f32, y: f32) -> Affine {
        Affine { a: x, d: y, ..Self::IDENTITY }
    }

    /// Rotation by `degrees`, clockwise on a y-down canvas.
    #[inline]
    pub fn rotate_degrees(degrees: f32) -> Affine {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Affine { a: cos, b: sin, c: -sin, d: cos, e: 0.0, f: 0.0 }
    }

    #[inline]
    pub fn apply(&self, point: Vec2) -> Vec2 {
        [
            self.a * point[0] + self.c * point[1] + self.e,
            self.b * point[0] + self.d * point[1] + self.f,
        ]
    }
}

impl std::ops::Mul for Affine {
    type Output = Affine;

    /// `self * rhs` applies `rhs` first, then `self`.
    #[inline]
    fn mul(self, rhs: Affine) -> Affine {
        Affine {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }
}
