pub type Padding = LayoutSpacing;
pub type Margin = LayoutSpacing;

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayoutSpacing {
    pub top: Spacing,
    pub bottom: Spacing,
    pub left: Spacing,
    pub right: Spacing,
}

impl LayoutSpacing {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn top(mut self, top: impl Into<Spacing>) -> Self {
        self.top = top.into();
        self
    }

    #[inline]
    pub fn bottom(mut self, bottom: impl Into<Spacing>) -> Self {
        self.bottom = bottom.into();
        self
    }

    #[inline]
    pub fn left(mut self, left: impl Into<Spacing>) -> Self {
        self.left = left.into();
        self
    }

    #[inline]
    pub fn right(mut self, right: impl Into<Spacing>) -> Self {
        self.right = right.into();
        self
    }

    /// For Top and Bottom
    #[inline]
    pub fn vertical(space: impl Into<Spacing>) -> Self {
        let space = space.into();
        Self {
            top: space,
            bottom: space,
            left: Spacing::DEFAULT_VALUE,
            right: Spacing::DEFAULT_VALUE,
        }
    }

    /// For Left and right
    #[inline]
    pub const fn horizontal(space: Spacing) -> Self {
        Self {
            left: space,
            right: space,
            top: Spacing::DEFAULT_VALUE,
            bottom: Spacing::DEFAULT_VALUE,
        }
    }

    #[inline]
    pub fn all(space: impl Into<Spacing>) -> Self {
        let space = space.into();
        Self {
            left: space,
            right: space,
            top: space,
            bottom: space,
        }
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spacing {
    Px(u32),
    Percent(u32),
    #[default]
    None,
}

impl From<i32> for Spacing {
    fn from(value: i32) -> Self {
        Spacing::Px(value as u32)
    }
}

impl From<u32> for Spacing {
    fn from(value: u32) -> Self {
        Spacing::Px(value)
    }
}

impl Spacing {
    pub const DEFAULT_VALUE: Spacing = Spacing::None;

    #[inline]
    pub fn value(&self, total: f32, scale: f32) -> f32 {
        match self {
            Spacing::Px(px) => *px as f32 * scale,
            Spacing::Percent(p) => total * (*p as f32 / 100.0),
            Spacing::None => 0.0,
        }
    }
}

impl From<f64> for Spacing {
    fn from(value: f64) -> Self {
        Spacing::Px(value as u32)
    }
}
