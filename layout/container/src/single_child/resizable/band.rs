use aimer_attribute::bounds::Bounds;

/// How far a grab band of a [`Resizable`](super::Resizable) reaches from the
/// edge it belongs to.
///
/// A band is measured from the edge line: `inner` logical pixels *inside* the
/// widget and `outer` logical pixels *outside* it. The outer reach is what makes
/// the cursor change before the pointer has crossed the border, the way a window
/// edge behaves — without it the pointer has to be inside the box already, which
/// is a pixel late.
///
/// Distances along an axis are signed: positive inside the widget, negative
/// outside it. A distance of `d` is in the band when `-outer <= d <= inner`.
///
/// # Examples
///
/// ```
/// use aimer_container::ResizeBand;
///
/// // Six pixels either side of the border.
/// let band = ResizeBand::new(6.0, 6.0);
///
/// assert!(band.holds(0.0), "the border itself");
/// assert!(band.holds(5.0), "inside");
/// assert!(band.holds(-5.0), "outside");
/// assert!(!band.holds(7.0));
/// ```
///
/// A band that stops at the border:
///
/// ```
/// use aimer_container::ResizeBand;
///
/// let band = ResizeBand::inside(4.0);
///
/// assert!(band.holds(4.0));
/// assert!(!band.holds(-1.0), "nothing outside the widget belongs to it");
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResizeBand {
    inner: f32,
    outer: f32,
}

impl ResizeBand {
    /// A band reaching `inner` pixels into the widget and `outer` pixels out of
    /// it.
    ///
    /// Negative reaches are read as zero, so a mistake in a builder narrows the
    /// band instead of turning it inside out.
    #[inline]
    pub const fn new(inner: f32, outer: f32) -> Self {
        Self {
            inner: if inner > 0.0 { inner } else { 0.0 },
            outer: if outer > 0.0 { outer } else { 0.0 },
        }
    }

    /// A band lying wholly inside the widget, `inner` pixels wide.
    #[inline]
    pub const fn inside(inner: f32) -> Self {
        Self::new(inner, 0.0)
    }

    /// How far the band reaches into the widget.
    #[inline]
    pub const fn inner(self) -> f32 {
        self.inner
    }

    /// How far the band reaches out of the widget.
    #[inline]
    pub const fn outer(self) -> f32 {
        self.outer
    }

    /// Whether the band has no width at all, in which case nothing can be
    /// grabbed.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::ResizeBand;
    ///
    /// assert!(ResizeBand::inside(0.0).is_empty());
    /// assert!(ResizeBand::new(-1.0, 0.0).is_empty());
    /// assert!(!ResizeBand::new(0.0, 3.0).is_empty());
    /// ```
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.inner <= 0.0 && self.outer <= 0.0
    }

    /// Whether a signed distance from the edge — positive inside the widget,
    /// negative outside it — falls in the band.
    #[inline]
    pub const fn holds(self, distance: f32) -> bool {
        distance >= -self.outer && distance <= self.inner
    }

    /// `bounds` grown by the outer reach on every side.
    ///
    /// This is the region a [`Resizable`](super::Resizable) must answer pointer
    /// events in: the framework offers an element the events that land within the
    /// bounds it reports, so a band reaching past the border is only reachable if
    /// the reported region reaches with it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::bounds::Bounds;
    /// use aimer_container::ResizeBand;
    ///
    /// let grown = ResizeBand::new(6.0, 4.0).grow(Bounds::new(10.0, 20.0, 100.0, 50.0));
    ///
    /// assert_eq!(grown.x, 6.0);
    /// assert_eq!(grown.y, 16.0);
    /// assert_eq!(grown.width, 108.0);
    /// assert_eq!(grown.height, 58.0);
    /// ```
    #[inline]
    pub const fn grow(self, bounds: Bounds) -> Bounds {
        Bounds::new(
            bounds.x - self.outer,
            bounds.y - self.outer,
            bounds.width + self.outer * 2.0,
            bounds.height + self.outer * 2.0,
        )
    }
}

impl Default for ResizeBand {
    /// An empty band, which grabs nothing.
    #[inline]
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_negative_reach_is_read_as_zero() {
        let band = ResizeBand::new(-4.0, -2.0);

        assert_eq!(band.inner(), 0.0);
        assert_eq!(band.outer(), 0.0);
        assert!(band.is_empty());
    }

    #[test]
    fn the_band_holds_the_border_and_both_of_its_reaches() {
        let band = ResizeBand::new(6.0, 4.0);

        assert!(band.holds(0.0));
        assert!(band.holds(6.0), "the innermost pixel counts");
        assert!(band.holds(-4.0), "the outermost pixel counts");
        assert!(!band.holds(6.5));
        assert!(!band.holds(-4.5));
    }

    #[test]
    fn an_inside_band_reaches_nothing_outside_the_widget() {
        let band = ResizeBand::inside(5.0);

        assert_eq!(band.outer(), 0.0);
        assert!(band.holds(0.0));
        assert!(!band.holds(-0.5));
    }

    #[test]
    fn growing_by_an_inside_band_leaves_the_bounds_alone() {
        let bounds = Bounds::new(10.0, 20.0, 100.0, 50.0);

        assert_eq!(ResizeBand::inside(8.0).grow(bounds), bounds);
    }

    #[test]
    fn growing_adds_the_outer_reach_on_every_side() {
        let grown = ResizeBand::new(2.0, 3.0).grow(Bounds::new(0.0, 0.0, 10.0, 10.0));

        assert_eq!(grown, Bounds::new(-3.0, -3.0, 16.0, 16.0));
    }
}
