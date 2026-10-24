use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::SafeAreaInsets;

/// The side of the anchor a floating panel is placed on.
///
/// The side selects the *main axis* of the placement: [`FloatingSide::Top`] and
/// [`FloatingSide::Bottom`] stack the panel vertically, while
/// [`FloatingSide::Left`] and [`FloatingSide::Right`] stack it horizontally.
/// The remaining axis is the *cross axis* and is controlled by
/// [`FloatingAlign`].
///
/// # Example
///
/// ```rust
/// use aimer_modal::FloatingSide;
///
/// assert_eq!(FloatingSide::Bottom.flipped(), FloatingSide::Top);
/// assert!(FloatingSide::Bottom.is_vertical());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatingSide {
    /// Above the anchor.
    Top,
    /// Below the anchor.
    Bottom,
    /// To the left of the anchor.
    Left,
    /// To the right of the anchor.
    Right,
}

impl FloatingSide {
    /// Returns the opposite side.
    #[inline]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// Returns whether this side stacks the panel along the vertical axis.
    #[inline]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }
}

/// Cross-axis alignment of a floating panel relative to its anchor.
///
/// For a vertical [`FloatingSide`] the alignment runs along the horizontal
/// axis, and vice versa.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloatingAlign {
    /// Align the leading edges of the panel and the anchor.
    Start,
    /// Center the panel on the anchor.
    Center,
    /// Align the trailing edges of the panel and the anchor.
    End,
}

/// What to do when the panel does not fit inside the viewport.
///
/// # Example
///
/// ```rust
/// use aimer_modal::OverflowPolicy;
///
/// assert_eq!(OverflowPolicy::default(), OverflowPolicy::Flip);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverflowPolicy {
    /// Move the panel to the opposite side when the preferred side does not
    /// fit and the opposite one does, then slide it inside the viewport.
    #[default]
    Flip,
    /// Keep the requested side and slide the panel inside the viewport.
    Shift,
    /// Leave the resolved position untouched, even when it leaves the
    /// viewport.
    Fixed,
}

/// The geometry request describing how a panel is placed around its anchor.
///
/// The spec is intentionally cheap to copy so widgets can store it inline and
/// resolve it on every frame against a fresh anchor rectangle.
///
/// # Example
///
/// ```rust
/// use aimer_modal::{FloatingAlign, FloatingSide, PlacementSpec};
///
/// let spec = PlacementSpec::new().side(FloatingSide::Right)
///                                .align(FloatingAlign::Center)
///                                .gap(8.0);
///
/// assert_eq!(spec.side_value(), FloatingSide::Right);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementSpec {
    side: FloatingSide,
    align: FloatingAlign,
    gap: f32,
    offset: Vec2d,
    overflow: OverflowPolicy,
    safe_area: SafeAreaInsets,
}

impl Default for PlacementSpec {
    fn default() -> Self {
        Self::new()
    }
}

impl PlacementSpec {
    /// Creates a spec that places the panel directly below the anchor with
    /// their leading edges aligned.
    #[inline]
    pub const fn new() -> Self {
        Self {
            side: FloatingSide::Bottom,
            align: FloatingAlign::Start,
            gap: 0.0,
            offset: Vec2d { x: 0.0, y: 0.0 },
            overflow: OverflowPolicy::Flip,
            safe_area: SafeAreaInsets::ZERO,
        }
    }

    /// Sets the preferred side of the anchor.
    #[inline]
    pub const fn side(mut self, side: FloatingSide) -> Self {
        self.side = side;
        self
    }

    /// Sets the cross-axis alignment.
    #[inline]
    pub const fn align(mut self, align: FloatingAlign) -> Self {
        self.align = align;
        self
    }

    /// Sets the distance between the anchor and the panel along the main axis.
    ///
    /// Non-finite values are treated as `0.0`.
    #[inline]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = if gap.is_finite() { gap } else { 0.0 };
        self
    }

    /// Sets an additional translation applied after the side and alignment are
    /// resolved.
    #[inline]
    pub fn offset(mut self, offset: Vec2d) -> Self {
        self.offset = Vec2d {
            x: if offset.x.is_finite() { offset.x } else { 0.0 },
            y: if offset.y.is_finite() { offset.y } else { 0.0 },
        };
        self
    }

    /// Sets the viewport overflow policy.
    #[inline]
    pub const fn overflow(mut self, overflow: OverflowPolicy) -> Self {
        self.overflow = overflow;
        self
    }

    /// Sets the edges of the viewport the panel must stay clear of.
    ///
    /// The insets are expressed in the same coordinate space as the anchor and
    /// the viewport, and they shrink the rectangle the panel is fitted into:
    /// [`OverflowPolicy::Flip`] flips away from a side that only fits *inside*
    /// a reserved edge, and the slide back into view stops at the edge instead
    /// of at the window. [`OverflowPolicy::Fixed`] ignores them, as it ignores
    /// the viewport.
    ///
    /// This is what keeps a panel out of the region the system draws over — the
    /// status bar, the notch, the home indicator — where a press never reaches
    /// the application. [`crate::Floating`] fills it in from the platform, so a
    /// panel only sets it to reserve something extra.
    ///
    /// # Example
    ///
    /// ```rust
    /// use aimer_modal::PlacementSpec;
    /// use aimer_widget::SafeAreaInsets;
    ///
    /// let spec = PlacementSpec::new().safe_area(SafeAreaInsets::new(0.0, 59.0, 0.0, 34.0));
    ///
    /// assert_eq!(spec.safe_area_value().top, 59.0);
    /// ```
    #[inline]
    pub const fn safe_area(mut self, safe_area: SafeAreaInsets) -> Self {
        self.safe_area = safe_area;
        self
    }

    /// Returns the preferred side.
    #[inline]
    pub const fn side_value(&self) -> FloatingSide {
        self.side
    }

    /// Returns the cross-axis alignment.
    #[inline]
    pub const fn align_value(&self) -> FloatingAlign {
        self.align
    }

    /// Returns the main-axis gap.
    #[inline]
    pub const fn gap_value(&self) -> f32 {
        self.gap
    }

    /// Returns the additional translation.
    #[inline]
    pub const fn offset_value(&self) -> Vec2d {
        self.offset
    }

    /// Returns the viewport overflow policy.
    #[inline]
    pub const fn overflow_value(&self) -> OverflowPolicy {
        self.overflow
    }

    /// Returns the reserved viewport edges.
    #[inline]
    pub const fn safe_area_value(&self) -> SafeAreaInsets {
        self.safe_area
    }
}

/// The resolved position of a floating panel.
///
/// `side` reports the side actually used, which differs from the requested one
/// when [`OverflowPolicy::Flip`] moved the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatingPlacement {
    /// Top-left corner of the panel in viewport coordinates.
    pub origin: Vec2d,
    /// The side the panel ended up on.
    pub side: FloatingSide,
}

/// Resolves the viewport-space position of a panel anchored to `anchor`.
///
/// All rectangles are expressed in the same logical coordinate space, with the
/// viewport starting at `(0, 0)`.
///
/// # Example
///
/// ```rust
/// use aimer_attribute::bounds::Bounds;
/// use aimer_attribute::size::ResolvedSize;
/// use aimer_modal::{FloatingSide, PlacementSpec, resolve_placement};
///
/// let anchor = Bounds::new(40.0, 60.0, 100.0, 20.0);
/// let panel = ResolvedSize { width: 120.0, height: 90.0 };
/// let viewport = ResolvedSize { width: 800.0, height: 600.0 };
///
/// let placement = resolve_placement(PlacementSpec::new(), anchor, panel, viewport);
///
/// assert_eq!(placement.side, FloatingSide::Bottom);
/// assert_eq!(placement.origin.x, 40.0);
/// assert_eq!(placement.origin.y, 80.0);
/// ```
pub fn resolve_placement(
    spec: PlacementSpec,
    anchor: Bounds,
    panel: ResolvedSize,
    viewport: ResolvedSize,
) -> FloatingPlacement {
    // What the panel may actually use: the viewport minus whatever the system —
    // or the caller — keeps for itself.
    let usable = if spec.overflow == OverflowPolicy::Fixed {
        SafeAreaInsets::ZERO
    } else {
        spec.safe_area
    };

    let mut side = spec.side;
    if spec.overflow == OverflowPolicy::Flip
        && !fits(side, anchor, panel, viewport, usable, spec.gap)
        && fits(side.flipped(), anchor, panel, viewport, usable, spec.gap)
    {
        side = side.flipped();
    }

    let mut origin = compose_origin(side, spec, anchor, panel);
    origin.x += spec.offset.x;
    origin.y += spec.offset.y;

    if spec.overflow != OverflowPolicy::Fixed {
        origin.x = shift_inside(
            origin.x,
            panel.width,
            usable.left,
            viewport.width - usable.right,
        );
        origin.y = shift_inside(
            origin.y,
            panel.height,
            usable.top,
            viewport.height - usable.bottom,
        );
    }

    FloatingPlacement { origin, side }
}

fn compose_origin(
    side: FloatingSide,
    spec: PlacementSpec,
    anchor: Bounds,
    panel: ResolvedSize,
) -> Vec2d {
    let cross = cross_origin(side, spec.align, anchor, panel);
    match side {
        FloatingSide::Top => Vec2d {
            x: cross,
            y: anchor.y - panel.height - spec.gap,
        },
        FloatingSide::Bottom => Vec2d {
            x: cross,
            y: anchor.y + anchor.height + spec.gap,
        },
        FloatingSide::Left => Vec2d {
            x: anchor.x - panel.width - spec.gap,
            y: cross,
        },
        FloatingSide::Right => Vec2d {
            x: anchor.x + anchor.width + spec.gap,
            y: cross,
        },
    }
}

fn cross_origin(
    side: FloatingSide,
    align: FloatingAlign,
    anchor: Bounds,
    panel: ResolvedSize,
) -> f32 {
    let (start, anchor_extent, panel_extent) = if side.is_vertical() {
        (anchor.x, anchor.width, panel.width)
    } else {
        (anchor.y, anchor.height, panel.height)
    };
    match align {
        FloatingAlign::Start => start,
        FloatingAlign::Center => start + (anchor_extent - panel_extent) / 2.0,
        FloatingAlign::End => start + anchor_extent - panel_extent,
    }
}

fn fits(
    side: FloatingSide,
    anchor: Bounds,
    panel: ResolvedSize,
    viewport: ResolvedSize,
    usable: SafeAreaInsets,
    gap: f32,
) -> bool {
    match side {
        FloatingSide::Top => anchor.y - panel.height - gap >= usable.top,
        FloatingSide::Bottom => {
            anchor.y + anchor.height + gap + panel.height <= viewport.height - usable.bottom
        }
        FloatingSide::Left => anchor.x - panel.width - gap >= usable.left,
        FloatingSide::Right => {
            anchor.x + anchor.width + gap + panel.width <= viewport.width - usable.right
        }
    }
}

/// Slides `origin` into `min..max`, giving up on the trailing edge when the
/// panel is too large to fit between them: half a panel reachable beats none.
fn shift_inside(origin: f32, extent: f32, min: f32, max: f32) -> f32 {
    if extent >= max - min {
        return min;
    }
    origin.clamp(min, max - extent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: ResolvedSize = ResolvedSize {
        width: 800.0,
        height: 600.0,
    };

    fn panel(width: f32, height: f32) -> ResolvedSize {
        ResolvedSize { width, height }
    }

    fn assert_origin(placement: FloatingPlacement, x: f32, y: f32) {
        assert!(
            (placement.origin.x - x).abs() < 1e-4 && (placement.origin.y - y).abs() < 1e-4,
            "expected origin ({x}, {y}), got ({}, {})",
            placement.origin.x,
            placement.origin.y
        );
    }

    #[test]
    fn bottom_start_places_the_panel_under_the_anchor() {
        let placement = resolve_placement(
            PlacementSpec::new(),
            Bounds::new(40.0, 60.0, 100.0, 20.0),
            panel(120.0, 90.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Bottom);
        assert_origin(placement, 40.0, 80.0);
    }

    #[test]
    fn center_alignment_centers_the_panel_on_the_anchor() {
        let placement = resolve_placement(
            PlacementSpec::new().align(FloatingAlign::Center),
            Bounds::new(300.0, 60.0, 100.0, 20.0),
            panel(60.0, 40.0),
            VIEWPORT,
        );

        assert_origin(placement, 320.0, 80.0);
    }

    #[test]
    fn end_alignment_aligns_the_trailing_edges() {
        let placement = resolve_placement(
            PlacementSpec::new().align(FloatingAlign::End),
            Bounds::new(300.0, 60.0, 100.0, 20.0),
            panel(60.0, 40.0),
            VIEWPORT,
        );

        assert_origin(placement, 340.0, 80.0);
    }

    #[test]
    fn right_side_places_the_panel_beside_the_anchor() {
        let placement = resolve_placement(
            PlacementSpec::new()
                .side(FloatingSide::Right)
                .align(FloatingAlign::Center),
            Bounds::new(100.0, 200.0, 50.0, 100.0),
            panel(80.0, 40.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Right);
        assert_origin(placement, 150.0, 230.0);
    }

    #[test]
    fn gap_and_offset_translate_the_resolved_origin() {
        let placement = resolve_placement(
            PlacementSpec::new()
                .gap(8.0)
                .offset(Vec2d { x: 4.0, y: -2.0 }),
            Bounds::new(40.0, 60.0, 100.0, 20.0),
            panel(120.0, 90.0),
            VIEWPORT,
        );

        assert_origin(placement, 44.0, 86.0);
    }

    #[test]
    fn gap_and_offset_reject_non_finite_values() {
        let spec = PlacementSpec::new().gap(f32::NAN).offset(Vec2d {
            x: f32::INFINITY,
            y: 5.0,
        });

        assert_eq!(spec.gap_value(), 0.0);
        assert_eq!(spec.offset_value().x, 0.0);
        assert_eq!(spec.offset_value().y, 5.0);
    }

    #[test]
    fn flip_moves_the_panel_above_an_anchor_near_the_bottom_edge() {
        let placement = resolve_placement(
            PlacementSpec::new().gap(4.0),
            Bounds::new(40.0, 540.0, 100.0, 20.0),
            panel(120.0, 90.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Top);
        assert_origin(placement, 40.0, 446.0);
    }

    #[test]
    fn flip_keeps_the_requested_side_when_neither_side_fits() {
        let placement = resolve_placement(
            PlacementSpec::new(),
            Bounds::new(40.0, 260.0, 100.0, 20.0),
            panel(120.0, 500.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Bottom);
    }

    #[test]
    fn shift_pulls_an_overflowing_panel_back_into_the_viewport() {
        let placement = resolve_placement(
            PlacementSpec::new().overflow(OverflowPolicy::Shift),
            Bounds::new(760.0, 60.0, 30.0, 20.0),
            panel(120.0, 90.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Bottom);
        assert_origin(placement, 680.0, 80.0);
    }

    #[test]
    fn fixed_keeps_a_panel_that_leaves_the_viewport() {
        let placement = resolve_placement(
            PlacementSpec::new().overflow(OverflowPolicy::Fixed),
            Bounds::new(760.0, 540.0, 30.0, 20.0),
            panel(120.0, 90.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Bottom);
        assert_origin(placement, 760.0, 560.0);
    }

    /// An iPhone in portrait: the status bar and the notch own the top of the
    /// window, the home indicator the bottom.
    const PHONE: SafeAreaInsets = SafeAreaInsets {
        left: 0.0,
        top: 59.0,
        right: 0.0,
        bottom: 34.0,
    };

    #[test]
    fn a_panel_above_an_anchor_under_the_status_bar_flips_below_it() {
        // The text being selected sits right under the status bar, so the
        // callout's preferred place — above it — is where touches belong to the
        // system.
        let placement = resolve_placement(
            PlacementSpec::new()
                .side(FloatingSide::Top)
                .gap(8.0)
                .safe_area(PHONE),
            Bounds::new(40.0, 70.0, 100.0, 20.0),
            panel(180.0, 44.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Bottom);
        assert_origin(placement, 40.0, 98.0);
    }

    #[test]
    fn a_panel_that_fits_above_the_status_bar_line_stays_there() {
        let placement = resolve_placement(
            PlacementSpec::new()
                .side(FloatingSide::Top)
                .gap(8.0)
                .safe_area(PHONE),
            Bounds::new(40.0, 140.0, 100.0, 20.0),
            panel(180.0, 44.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Top);
        assert_origin(placement, 40.0, 88.0);
    }

    #[test]
    fn a_panel_that_fits_nowhere_is_pushed_clear_of_the_reserved_edges() {
        // Neither side fits, so the panel keeps its side and slides inside —
        // and "inside" now stops at the status bar rather than at the window.
        let placement = resolve_placement(
            PlacementSpec::new()
                .side(FloatingSide::Top)
                .safe_area(PHONE),
            Bounds::new(40.0, 20.0, 100.0, 20.0),
            panel(180.0, 560.0),
            VIEWPORT,
        );

        assert!(placement.origin.y >= PHONE.top - 1e-4);
    }

    #[test]
    fn a_panel_at_the_bottom_edge_clears_the_home_indicator() {
        let placement = resolve_placement(
            PlacementSpec::new().safe_area(PHONE),
            Bounds::new(40.0, 580.0, 100.0, 20.0),
            panel(180.0, 44.0),
            VIEWPORT,
        );

        assert_origin(placement, 40.0, 522.0);
    }

    #[test]
    fn reserved_side_edges_hold_a_panel_off_them() {
        let landscape = SafeAreaInsets::new(59.0, 0.0, 59.0, 21.0);
        let placement = resolve_placement(
            PlacementSpec::new().safe_area(landscape),
            Bounds::new(780.0, 60.0, 20.0, 20.0),
            panel(180.0, 44.0),
            VIEWPORT,
        );

        assert_origin(placement, 561.0, 80.0);
    }

    #[test]
    fn a_fixed_panel_ignores_the_reserved_edges() {
        let placement = resolve_placement(
            PlacementSpec::new()
                .side(FloatingSide::Top)
                .overflow(OverflowPolicy::Fixed)
                .safe_area(PHONE),
            Bounds::new(40.0, 70.0, 100.0, 20.0),
            panel(180.0, 44.0),
            VIEWPORT,
        );

        assert_eq!(placement.side, FloatingSide::Top);
        assert_origin(placement, 40.0, 26.0);
    }

    #[test]
    fn a_panel_taller_than_the_usable_height_starts_at_the_reserved_edge() {
        let placement = resolve_placement(
            PlacementSpec::new().safe_area(PHONE),
            Bounds::new(400.0, 300.0, 30.0, 20.0),
            panel(900.0, 700.0),
            VIEWPORT,
        );

        assert_origin(placement, 0.0, 59.0);
    }

    #[test]
    fn a_panel_larger_than_the_viewport_starts_at_the_origin() {
        let placement = resolve_placement(
            PlacementSpec::new(),
            Bounds::new(400.0, 300.0, 30.0, 20.0),
            panel(900.0, 700.0),
            VIEWPORT,
        );

        assert_origin(placement, 0.0, 0.0);
    }
}
