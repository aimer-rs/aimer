use core::fmt;

/// A finite point in a shape's local coordinate space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Point {
    /// Creates a point without silently normalizing its values.
    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Returns whether both coordinates are finite.
    #[inline]
    pub const fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Returns the squared Euclidean distance to `other`.
    #[inline]
    pub fn distance_squared(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }

    #[inline]
    pub(crate) fn approx_eq(self, other: Self) -> bool {
        let scale = self
            .x
            .abs()
            .max(self.y.abs())
            .max(other.x.abs())
            .max(other.y.abs())
            .max(1.0);
        self.distance_squared(other) <= (1.0e-4 * scale).powi(2)
    }
}

/// A finite target size used by [`crate::ShapeFit`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShapeSize {
    /// Width in local or target units.
    pub width: f32,
    /// Height in local or target units.
    pub height: f32,
}

impl ShapeSize {
    /// Creates a size. [`crate::ShapeFit::transform`] validates it before use.
    #[inline]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Returns whether the size is finite and non-negative.
    #[inline]
    pub const fn is_valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width >= 0.0 && self.height >= 0.0
    }
}

/// The finite axis-aligned bounds of a validated path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeBounds {
    /// Minimum x/y coordinate.
    pub min: Point,
    /// Maximum x/y coordinate.
    pub max: Point,
}

impl ShapeBounds {
    /// Constructs bounds when both corners are finite and ordered.
    pub fn new(min: Point, max: Point) -> Option<Self> {
        if min.is_finite() && max.is_finite() && min.x <= max.x && min.y <= max.y {
            Some(Self { min, max })
        } else {
            None
        }
    }

    /// Returns the extent of the bounds.
    #[inline]
    pub const fn size(self) -> ShapeSize {
        ShapeSize::new(self.max.x - self.min.x, self.max.y - self.min.y)
    }

    /// Returns the width.
    #[inline]
    pub const fn width(self) -> f32 {
        self.max.x - self.min.x
    }

    /// Returns the height.
    #[inline]
    pub const fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    /// Returns the center point.
    #[inline]
    pub const fn center(self) -> Point {
        Point::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
        )
    }

    /// Returns whether `point` is inside or on the bounds.
    #[inline]
    pub fn contains(self, point: Point) -> bool {
        point.is_finite()
            && point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }

    /// Expands the bounds by a finite non-negative amount.
    pub fn expand(self, amount: f32) -> Option<Self> {
        if !amount.is_finite() || amount < 0.0 {
            return None;
        }
        Self::new(
            Point::new(self.min.x - amount, self.min.y - amount),
            Point::new(self.max.x + amount, self.max.y + amount),
        )
    }

    /// Returns the union of two bounds.
    #[inline]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }
}

impl fmt::Display for ShapeBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({:.3},{:.3})..({:.3},{:.3})",
            self.min.x, self.min.y, self.max.x, self.max.y
        )
    }
}

/// A finite affine transform used by shape draw requests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeTransform {
    /// Scale on the x axis before rotation.
    pub sx: f32,
    /// Scale on the y axis before rotation.
    pub sy: f32,
    /// Rotation in radians.
    pub rotation: f32,
    /// Translation on the x axis.
    pub tx: f32,
    /// Translation on the y axis.
    pub ty: f32,
}

impl Default for ShapeTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl ShapeTransform {
    /// Returns the identity transform.
    #[inline]
    pub const fn identity() -> Self {
        Self {
            sx: 1.0,
            sy: 1.0,
            rotation: 0.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Creates a scale-and-translation transform without rotation.
    #[inline]
    pub const fn scale_translate(sx: f32, sy: f32, tx: f32, ty: f32) -> Self {
        Self {
            sx,
            sy,
            rotation: 0.0,
            tx,
            ty,
        }
    }

    /// Returns whether all values are finite and the scale is non-zero.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.sx.is_finite()
            && self.sy.is_finite()
            && self.rotation.is_finite()
            && self.tx.is_finite()
            && self.ty.is_finite()
            && self.sx.abs() > f32::EPSILON
            && self.sy.abs() > f32::EPSILON
    }

    /// Applies the transform to a local point.
    #[inline]
    pub fn transform_point(self, point: Point) -> Point {
        let (sin, cos) = self.rotation.sin_cos();
        Point::new(
            cos * self.sx * point.x - sin * self.sy * point.y + self.tx,
            sin * self.sx * point.x + cos * self.sy * point.y + self.ty,
        )
    }

    /// Returns the six affine matrix coefficients `[a, b, c, d, e, f]`.
    #[inline]
    pub fn to_matrix(self) -> [f32; 6] {
        let (sin, cos) = self.rotation.sin_cos();
        [
            cos * self.sx,
            sin * self.sx,
            -sin * self.sy,
            cos * self.sy,
            self.tx,
            self.ty,
        ]
    }
}
