use std::ops::{Add, Mul, Sub, SubAssign};

use crate::size::ResolvedSize;

macro_rules! impl_from_num {
    ($t:ty) => {
        impl From<$t> for Vec2d {
            fn from(x: $t) -> Self {
                Self {
                    x: x as f32,
                    y: x as f32,
                }
            }
        }
    };
}

macro_rules! impl_from_tuple {
    ($t:ty) => {
        impl From<$t> for Vec2d {
            fn from((x, y): $t) -> Self {
                Self {
                    x: x as f32,
                    y: y as f32,
                }
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2d {
    pub x: f32,
    pub y: f32,
}

impl Vec2d {
    pub const ZERO: Vec2d = Vec2d { x: 0.0, y: 0.0 };

    pub fn magnitude(&self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn scale(&self, s: f32) -> Vec2d {
        Vec2d {
            x: self.x * s,
            y: self.y * s,
        }
    }

    pub fn abs(&self) -> Vec2d {
        Vec2d {
            x: self.x.abs(),
            y: self.y.abs(),
        }
    }
}

impl Vec2d {
    pub const fn round(self) -> Self {
        Self {
            x: self.x.round(),
            y: self.y.round(),
        }
    }
}

impl Mul<Vec2d> for f32 {
    type Output = Vec2d;
    fn mul(self, rhs: Vec2d) -> Self::Output {
        Self::Output {
            x: self * rhs.x,
            y: self * rhs.y,
        }
    }
}

impl Mul<f32> for Vec2d {
    type Output = Vec2d;
    fn mul(self, rhs: f32) -> Self::Output {
        Self::Output {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl Add<Vec2d> for Vec2d {
    type Output = Vec2d;
    fn add(self, rhs: Vec2d) -> Self::Output {
        Self::Output {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl Add<f32> for Vec2d {
    type Output = Vec2d;
    fn add(self, rhs: f32) -> Self::Output {
        Self::Output {
            x: self.x + rhs,
            y: self.y + rhs,
        }
    }
}

impl Sub<(f32, f32)> for Vec2d {
    type Output = Vec2d;
    fn sub(self, rhs: (f32, f32)) -> Self::Output {
        Self::Output {
            x: self.x - rhs.0,
            y: self.y - rhs.1,
        }
    }
}

impl SubAssign<(f32, f32)> for Vec2d {
    fn sub_assign(&mut self, rhs: (f32, f32)) {
        self.x -= rhs.0;
        self.y -= rhs.1;
    }
}

impl Vec2d {
    pub fn get_end(&self, size: ResolvedSize) -> Vec2d {
        Self {
            x: self.x + size.width,
            y: self.y + size.height,
        }
    }
}

impl From<(f32, f32)> for Vec2d {
    fn from((x, y): (f32, f32)) -> Self {
        Self { x, y }
    }
}

impl From<f32> for Vec2d {
    fn from(x: f32) -> Self {
        Self { x, y: x }
    }
}

impl_from_num!(usize);
impl_from_num!(i8);
impl_from_num!(u8);
impl_from_num!(i16);
impl_from_num!(u16);
impl_from_num!(u32);
impl_from_num!(i32);
impl_from_num!(i64);
impl_from_num!(u64);

impl_from_tuple!((usize, usize));
impl_from_tuple!((i8, i8));
impl_from_tuple!((u8, u8));
impl_from_tuple!((i16, i16));
impl_from_tuple!((u16, u16));
impl_from_tuple!((u32, u32));
impl_from_tuple!((i32, i32));
impl_from_tuple!((i64, i64));
impl_from_tuple!((u64, u64));
