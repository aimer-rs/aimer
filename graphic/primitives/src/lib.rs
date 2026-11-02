#![doc = include_str!("../README.md")]
pub mod color;
pub mod id_hasher;
pub mod mat3;
pub mod rgba8;

pub use color::*;
pub use id_hasher::{IdBuildHasher, IdHasher};
pub use mat3::*;
pub use rgba8::Rgba8;

/// Common types used throughout the cupid render engine.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl Rect {
    /// Returns this rectangle moved by `(x, y)`.
    #[inline]
    pub fn translated(self, x: f32, y: f32) -> Self {
        Self::new(self.x + x, self.y + y, self.width, self.height)
    }

    /// Returns the x coordinate of the right edge.
    #[inline]
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    /// Returns the y coordinate of the bottom edge.
    #[inline]
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    /// Returns whether every component is finite and the size is non-negative.
    #[inline]
    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width >= 0.0
            && self.height >= 0.0
    }

    /// Returns the overlap of two valid rectangles, or `None` when they do not overlap by area.
    #[inline]
    pub fn intersection(self, other: Self) -> Option<Self> {
        if !self.is_valid() || !other.is_valid() {
            return None;
        }
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Self::new(x, y, right - x, bottom - y))
    }

    /// Tests whether a point lies within the rectangle's half-open bounds.
    #[inline]
    pub fn contains_point(self, x: f32, y: f32) -> bool {
        self.is_valid() && x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// Returns the smallest rectangle containing both rectangles.
    #[inline]
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(x, y, right - x, bottom - y)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Vec2d {
    pub x: f32,
    pub y: f32,
}

impl Vec2d {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

pub type TextureId = u32;
