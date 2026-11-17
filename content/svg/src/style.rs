use aimer_cupid::svg::{SvgColor, SvgTransform};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SvgStyle {
    pub fill: Option<Option<SvgColor>>,
    pub stroke: Option<Option<SvgColor>>,
    /// Replaces the authored `stroke-width`; `None` keeps the document's.
    pub stroke_width: Option<f32>,
    /// Replaces the authored `stroke-dashoffset`; `None` keeps the document's.
    pub stroke_dash_offset: Option<f32>,
    /// Limits the stroke to this `[start, end]` fraction of each subpath.
    pub stroke_dash_offset_percent: Option<[f32; 2]>,
    pub opacity: Option<f32>,
    pub transform: Option<SvgTransform>,
}

impl SvgStyle {
    pub const fn new() -> Self {
        Self {
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_dash_offset: None,
            stroke_dash_offset_percent: None,
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

    /// Sets the stroke width, in the SVG's own user units.
    ///
    /// Negative and NaN widths become `0.0`, which draws no stroke. The width only
    /// matters on nodes that have a stroke; it does not add one.
    #[inline]
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = Some(width.max(0.0));
        self
    }

    /// Sets the dash offset, the distance into the dash pattern where the stroke
    /// starts, in the SVG's user units. A NaN offset becomes `0.0`.
    ///
    /// Only a stroke that already has a `stroke-dasharray` is affected. To reveal
    /// part of a solid stroke use [`SvgStyle::stroke_dash_offset_percent`].
    #[inline]
    pub fn stroke_dash_offset(mut self, offset: f32) -> Self {
        self.stroke_dash_offset = Some(if offset.is_nan() { 0.0 } else { offset });
        self
    }

    /// Draws only part of the stroke: the `range` of each subpath's length, as
    /// fractions from `0.0` (its start) to `1.0` (its end).
    ///
    /// `0.0..0.5` draws the first half of every subpath and `0.25..0.75` the
    /// middle half, so animating the end of the range draws a path in. Each
    /// subpath is measured on its own. A dashed stroke keeps its dashes in place
    /// and is clipped to the range. Ends are clamped to `0.0..=1.0` with NaN as
    /// `0.0`, and an empty or inverted range draws nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_svg::SvgStyle;
    ///
    /// let half = SvgStyle::new().stroke_dash_offset_percent(0.0..0.5);
    /// assert_eq!(half.stroke_dash_offset_percent, Some([0.0, 0.5]));
    /// ```
    #[inline]
    pub fn stroke_dash_offset_percent(mut self, range: std::ops::Range<f32>) -> Self {
        let fraction = |end: f32| if end.is_nan() { 0.0 } else { end.clamp(0.0, 1.0) };
        self.stroke_dash_offset_percent = Some([fraction(range.start), fraction(range.end)]);
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
