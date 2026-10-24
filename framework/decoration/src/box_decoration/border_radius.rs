use aimer_attribute::Dimension;

use crate::border::resolve_dim;

/// Per-corner radii usable directly as a [`crate::Decoration`].
/// Containers clip children to these radii and round their fallback background.
/// Without a background this paints nothing; paint-only decoration wrappers
/// do not apply child clipping. Zero radii still declare a rectangular clip.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct BorderRadius {
    pub top_left: Dimension,
    pub top_right: Dimension,
    pub bottom_right: Dimension,
    pub bottom_left: Dimension,
}

impl BorderRadius {

    #[inline]
    pub fn all(radius: impl Into<Dimension>) -> Self {
        let radius = radius.into();
        Self {
            bottom_left: radius,
            bottom_right:radius,
            top_left: radius,
            top_right: radius
        }

    }

    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn top_left(mut self, top_left: impl Into<Dimension>) -> Self {
        self.top_left = top_left.into();
        self
    }

    #[inline]
    pub fn top_right(mut self, top_right: impl Into<Dimension>) -> Self {
        self.top_right = top_right.into();
        self
    }

    #[inline]
    pub fn bottom_right(mut self, bottom_right: impl Into<Dimension>) -> Self {
        self.bottom_right = bottom_right.into();
        self
    }

    #[inline]
    pub fn bottom_left(mut self, bottom_left: impl Into<Dimension>) -> Self {
        self.bottom_left = bottom_left.into();
        self
    }

    #[inline]
    pub fn resolve(&self, box_width: f32, box_height: f32, scale: f32) -> [f32; 4] {
        [
            resolve_dim(self.top_left, box_width, scale),
            resolve_dim(self.top_right, box_width, scale),
            resolve_dim(self.bottom_right, box_height, scale),
            resolve_dim(self.bottom_left, box_height, scale),
        ]
    }
}

impl From<i32> for BorderRadius {
    fn from(value: i32) -> Self {
        BorderRadius {
            top_left: value.into(),
            top_right: value.into(),
            bottom_right: value.into(),
            bottom_left: value.into(),
        }
    }
}

impl From<f32> for BorderRadius {
    fn from(value: f32) -> Self {
        BorderRadius {
            top_left: value.into(),
            top_right: value.into(),
            bottom_right: value.into(),
            bottom_left: value.into(),
        }
    }
}

impl From<(f32, f32, f32, f32)> for BorderRadius {
    fn from((tl, tr, br, bl): (f32, f32, f32, f32)) -> Self {
        BorderRadius {
            top_left: tl.into(),
            top_right: tr.into(),
            bottom_right: br.into(),
            bottom_left: bl.into(),
        }
    }
}

impl From<(i32, i32, i32, i32)> for BorderRadius {
    fn from((tl, tr, br, bl): (i32, i32, i32, i32)) -> Self {
        BorderRadius {
            top_left: tl.into(),
            top_right: tr.into(),
            bottom_right: br.into(),
            bottom_left: bl.into(),
        }
    }
}
