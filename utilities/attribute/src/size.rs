use std::ops::Mul;

use crate::dimension::Dimension;

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Size {
    pub width: Dimension,
    pub height: Dimension,
}

impl Default for Size {
    fn default() -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Auto,
        }
    }
}

impl Size {
    pub fn new(width: impl Into<Dimension>, height: impl Into<Dimension>) -> Self {
        Self {
            width: width.into(),
            height: height.into(),
        }
    }

    pub fn square(side: impl Into<Dimension>) -> Self {
        let d = side.into();
        Self {
            width: d,
            height: d,
        }
    }

    pub fn zero() -> Self {
        Self {
            width: Dimension::Px(0.0),
            height: Dimension::Px(0.0),
        }
    }

    pub fn is_auto_width(&self) -> bool {
        self.width == Dimension::Auto
    }

    pub fn is_auto_height(&self) -> bool {
        self.height == Dimension::Auto
    }

    pub fn resolve(&self, parent: &ResolvedSize, scale: f32) -> ResolvedSize {
        ResolvedSize {
            width: self.width.resolve(parent.width, scale),
            height: self.height.resolve(parent.height, scale),
        }
    }
}

/// # The resolved pixel size after layout.
///
/// The representation is `f32` on every supported target, including
/// WebAssembly.
#[derive(Copy, Clone, Default, Debug, PartialEq)]
pub struct ResolvedSize {
    pub width: f32,
    pub height: f32,
}

impl Mul<f32> for ResolvedSize {
    type Output = ResolvedSize;
    fn mul(self, rhs: f32) -> Self::Output {
        ResolvedSize {
            width: self.width * rhs,
            height: self.height * rhs,
        }
    }
}

impl ResolvedSize {}
