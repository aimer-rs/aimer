use std::ops::{Div, Mul};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    pub fn is_inside(&self, x: f32, y: f32) -> bool {
        self.x <= x && x <= self.x + self.width && self.y <= y && y <= self.y + self.height
    }
}

impl Mul<f32> for Bounds {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
            width: self.width * rhs,
            height: self.height * rhs,
        }
    }
}

impl Div<f32> for Bounds {
    type Output = Self;
    fn div(self, rhs: f32) -> Self::Output {
        Self {
            x: self.x / rhs,
            y: self.y / rhs,
            width: self.width / rhs,
            height: self.height / rhs,
        }
    }
}

impl Bounds {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}
impl Default for Bounds {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }
}
