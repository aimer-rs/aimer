use aimer_cupid::svg::{SvgColor, SvgTransform};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvgStyle {
    pub fill: Option<Option<SvgColor>>,
    pub stroke: Option<Option<SvgColor>>,
    pub opacity: Option<f32>,
    pub transform: Option<SvgTransform>,
}

impl SvgStyle {
    pub const fn new() -> Self {
        Self {
            fill: None,
            stroke: None,
            opacity: None,
            transform: None,
        }
    }

    pub fn fill(mut self, fill: impl Into<SvgColor>) -> Self {
        self.fill = Some(Some(fill.into()));
        self
    }

    pub const fn no_fill(mut self) -> Self {
        self.fill = Some(None);
        self
    }

    pub fn stroke(mut self, stroke: impl Into<SvgColor>) -> Self {
        self.stroke = Some(Some(stroke.into()));
        self
    }

    pub const fn no_stroke(mut self) -> Self {
        self.stroke = Some(None);
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = Some(opacity.clamp(0.0, 1.0));
        self
    }

    pub const fn transform(mut self, transform: SvgTransform) -> Self {
        self.transform = Some(transform);
        self
    }
}
