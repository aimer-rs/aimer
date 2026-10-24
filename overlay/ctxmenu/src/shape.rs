//! The two shapes a context menu is drawn in, and where each one lands.
//!
//! A finger has no cursor to aim with and hides the screen where it touches, so
//! a touch menu is the *pill* every mobile system draws: a short horizontal bar
//! of verbs floating clear of the thing it acts on. A mouse points at an exact
//! spot and expects the menu to appear *there*, growing down and to the right
//! like every desktop menu since 1988 — the *list*.
//!
//! The shape is the input device's choice, not the platform's, so a phone
//! browser and a desktop browser each get the shape their user is holding.

use aimer_events::pointer::PointerSource;
use aimer_modal::{FloatingAlign, FloatingSide, OverflowPolicy, PlacementSpec};

/// Which of the two shapes a menu is drawn in.
///
/// # Examples
///
/// ```
/// use aimer_ctxmenu::ContextMenuShape;
/// use aimer_events::pointer::PointerSource;
///
/// assert_eq!(
///     ContextMenuShape::for_source(PointerSource::Touch),
///     ContextMenuShape::Pill
/// );
/// assert_eq!(
///     ContextMenuShape::for_source(PointerSource::Mouse),
///     ContextMenuShape::List
/// );
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ContextMenuShape {
    /// The horizontal bar a touch screen offers above the selection.
    #[default]
    Pill,
    /// The vertical list a desktop opens at the pointer.
    List,
}

impl ContextMenuShape {
    /// The shape the pointer that asked for the menu expects.
    #[inline]
    pub const fn for_source(source: PointerSource) -> Self {
        match source {
            PointerSource::Mouse => Self::List,
            PointerSource::Touch => Self::Pill,
        }
    }

    /// Whether the items are tiled side by side rather than stacked.
    #[inline]
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Self::Pill)
    }

    /// How the panel is placed around its anchor, `gap` logical pixels away.
    ///
    /// The pill floats *above* the anchor, centred on it, and flips underneath
    /// when there is no room. The list hangs its top-left corner off the
    /// anchor — which for a right-click is the click itself — and flips above
    /// it rather than sliding up, since a menu sliding up under the cursor
    /// would open with an item already beneath it, ready to be triggered by
    /// the release of the very click that opened it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_ctxmenu::ContextMenuShape;
    /// use aimer_modal::{FloatingAlign, FloatingSide};
    ///
    /// let pill = ContextMenuShape::Pill.placement(10.0);
    /// assert_eq!(pill.side_value(), FloatingSide::Top);
    /// assert_eq!(pill.align_value(), FloatingAlign::Center);
    /// assert_eq!(pill.gap_value(), 10.0);
    ///
    /// let list = ContextMenuShape::List.placement(0.0);
    /// assert_eq!(list.side_value(), FloatingSide::Bottom);
    /// assert_eq!(list.align_value(), FloatingAlign::Start);
    /// ```
    #[inline]
    pub fn placement(self, gap: f32) -> PlacementSpec {
        let spec = PlacementSpec::new().gap(gap).overflow(OverflowPolicy::Flip);
        match self {
            Self::Pill => spec.side(FloatingSide::Top).align(FloatingAlign::Center),
            Self::List => spec.side(FloatingSide::Bottom).align(FloatingAlign::Start),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pill_tiles_sideways_and_the_list_stacks() {
        assert!(ContextMenuShape::Pill.is_horizontal());
        assert!(!ContextMenuShape::List.is_horizontal());
    }

    #[test]
    fn the_pill_hangs_above_its_anchor_and_the_list_below_it() {
        assert_eq!(
            ContextMenuShape::Pill.placement(8.0).side_value(),
            FloatingSide::Top
        );
        assert_eq!(
            ContextMenuShape::List.placement(0.0).side_value(),
            FloatingSide::Bottom
        );
    }

    #[test]
    fn both_shapes_flip_rather_than_leave_the_window() {
        for shape in [ContextMenuShape::Pill, ContextMenuShape::List] {
            assert_eq!(
                shape.placement(0.0).overflow_value(),
                OverflowPolicy::Flip,
                "a menu that cannot fit turns over instead of being cut off"
            );
        }
    }
}
