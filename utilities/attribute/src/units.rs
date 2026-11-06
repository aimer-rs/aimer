use crate::dimension::Dimension;

/// The surroundings the CSS context-relative length units are measured
/// against.
///
/// `em`, `rem`, `vw`, `vh`, `vmin` and `vmax` have no fixed size: `2em` is two
/// font sizes, `50vw` is half the viewport. [`Dimension`] deliberately has no
/// variant for them, so each is resolved *when it is written* into a
/// [`Dimension::Px`], using this snapshot. Layout then sees an ordinary pixel
/// size and pays nothing extra.
///
/// The consequence is that a value is only as current as its snapshot: build it
/// again when the viewport or font size changes. Inside a widget build,
/// `aimer_element::RelativeUnitExt` (`2.vw()`, `1.5.em()`) takes the snapshot
/// from the current build and rebuilds the widget when the window is resized.
///
/// All lengths are logical pixels.
///
/// # Examples
///
/// ```
/// use aimer_attribute::{Dimension, UnitContext};
///
/// let units = UnitContext::new(800.0, 600.0).with_font_size(20.0);
///
/// assert_eq!(units.vw(50.0), Dimension::Px(400.0));
/// assert_eq!(units.vh(10.0), Dimension::Px(60.0));
/// assert_eq!(units.vmin(10.0), Dimension::Px(60.0));
/// assert_eq!(units.vmax(10.0), Dimension::Px(80.0));
/// assert_eq!(units.em(1.5), Dimension::Px(30.0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitContext {
    /// The font size `em` is measured in.
    pub font_size: f32,
    /// The root font size `rem` is measured in.
    pub root_font_size: f32,
    /// Viewport width, the extent `vw` is a percentage of.
    pub viewport_width: f32,
    /// Viewport height, the extent `vh` is a percentage of.
    pub viewport_height: f32,
}

impl UnitContext {
    /// The CSS initial font size, in pixels.
    pub const DEFAULT_FONT_SIZE: f32 = 16.0;

    /// Creates a context for a viewport, with the [default font
    /// size](Self::DEFAULT_FONT_SIZE) for both `em` and `rem`.
    #[inline]
    pub const fn new(viewport_width: f32, viewport_height: f32) -> Self {
        Self {
            font_size: Self::DEFAULT_FONT_SIZE,
            root_font_size: Self::DEFAULT_FONT_SIZE,
            viewport_width,
            viewport_height,
        }
    }

    /// Sets the font size `em` is measured in.
    #[inline]
    pub const fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// Sets the root font size `rem` is measured in.
    #[inline]
    pub const fn with_root_font_size(mut self, root_font_size: f32) -> Self {
        self.root_font_size = root_font_size;
        self
    }

    /// `value` font sizes.
    #[inline]
    pub fn em(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.font_size)
    }

    /// `value` root font sizes.
    #[inline]
    pub fn rem(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.root_font_size)
    }

    /// `value` percent of the viewport width.
    #[inline]
    pub fn vw(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.viewport_width / 100.0)
    }

    /// `value` percent of the viewport height.
    #[inline]
    pub fn vh(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.viewport_height / 100.0)
    }

    /// `value` percent of the smaller viewport extent.
    #[inline]
    pub fn vmin(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.viewport_width.min(self.viewport_height) / 100.0)
    }

    /// `value` percent of the larger viewport extent.
    #[inline]
    pub fn vmax(&self, value: f32) -> Dimension {
        Dimension::Px(value * self.viewport_width.max(self.viewport_height) / 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units() -> UnitContext {
        UnitContext::new(800.0, 600.0)
    }

    #[test]
    fn defaults_to_the_css_initial_font_size() {
        let units = units();
        assert_eq!(units.font_size, 16.0);
        assert_eq!(units.root_font_size, 16.0);
        assert_eq!(units.em(2.0), Dimension::Px(32.0));
        assert_eq!(units.rem(0.5), Dimension::Px(8.0));
    }

    #[test]
    fn em_and_rem_use_their_own_font_sizes() {
        let units = units().with_font_size(20.0).with_root_font_size(10.0);
        assert_eq!(units.em(1.5), Dimension::Px(30.0));
        assert_eq!(units.rem(1.5), Dimension::Px(15.0));
    }

    #[test]
    fn viewport_units_are_percentages_of_the_viewport() {
        let units = units();
        assert_eq!(units.vw(100.0), Dimension::Px(800.0));
        assert_eq!(units.vw(50.0), Dimension::Px(400.0));
        assert_eq!(units.vh(100.0), Dimension::Px(600.0));
        assert_eq!(units.vmin(100.0), Dimension::Px(600.0));
        assert_eq!(units.vmax(100.0), Dimension::Px(800.0));
    }

    #[test]
    fn vmin_and_vmax_swap_with_orientation() {
        let portrait = UnitContext::new(400.0, 900.0);
        assert_eq!(portrait.vmin(50.0), Dimension::Px(200.0));
        assert_eq!(portrait.vmax(50.0), Dimension::Px(450.0));
    }

    #[test]
    fn zero_negative_and_empty_viewports_do_not_panic() {
        let empty = UnitContext::new(0.0, 0.0);
        assert_eq!(empty.vw(50.0), Dimension::Px(0.0));
        assert_eq!(empty.vmax(50.0), Dimension::Px(0.0));
        assert_eq!(units().vw(-10.0), Dimension::Px(-80.0));
        assert_eq!(units().em(0.0), Dimension::Px(0.0));
    }

    #[test]
    fn results_resolve_as_plain_pixels() {
        assert_eq!(units().vw(25.0).resolve(0.0, 1.0), 200.0);
    }
}
