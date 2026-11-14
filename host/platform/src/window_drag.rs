//! Dragging an edge or a corner of the window, as a person does to resize it.
//!
//! A window is resized from the side that moves: pulling the east edge changes
//! the width and leaves the window's left side where it was, while pulling the
//! west edge changes the width and moves the window, because it is the right
//! side that stays. [`WindowEdge`] knows which sides those are, so the event
//! loop only has to apply the answer.

/// A change of the window's state that a person makes with its title-bar buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowAction {
    /// Fills the screen's work area.
    Maximize,
    /// Leaves the maximized state, back to the size it had.
    Unmaximize,
    /// Hides the window in the dock or taskbar.
    Minimize,
    /// Brings a minimized window back.
    Unminimize,
}

/// Which edge or corner of the window is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEdge {
    /// The top edge.
    North,
    /// The bottom edge.
    South,
    /// The right edge.
    East,
    /// The left edge.
    West,
    /// The top-right corner.
    NorthEast,
    /// The top-left corner.
    NorthWest,
    /// The bottom-right corner.
    SouthEast,
    /// The bottom-left corner.
    SouthWest,
}

/// Where a drag of an edge is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDragPhase {
    /// The edge was grabbed: the window's rectangle at this moment is what the
    /// distances of the steps that follow are counted from.
    Started,
    /// The edge moved to a distance from where it was grabbed.
    Moved,
    /// The edge moved to its last distance and was let go.
    Ended,
}

impl WindowEdge {
    /// Whether this edge moves with the pointer horizontally.
    #[inline]
    pub const fn moves_x(self) -> bool {
        !matches!(self, Self::North | Self::South)
    }

    /// Whether this edge moves with the pointer vertically.
    #[inline]
    pub const fn moves_y(self) -> bool {
        !matches!(self, Self::East | Self::West)
    }

    /// Whether dragging this edge moves the window's top-left corner: the left
    /// and top edges do, because the far side is the one that stays.
    #[inline]
    pub const fn moves_origin(self) -> bool {
        self.left() || self.top()
    }

    #[inline]
    const fn left(self) -> bool {
        matches!(self, Self::West | Self::NorthWest | Self::SouthWest)
    }

    #[inline]
    const fn top(self) -> bool {
        matches!(self, Self::North | Self::NorthWest | Self::NorthEast)
    }

    /// The size and position of the window after this edge was dragged by
    /// `(dx, dy)` from where `origin` (`x, y, width, height`) had it.
    ///
    /// The distances are the pointer's travel, so dragging an east edge right
    /// widens the window and dragging a west edge right narrows it. A distance
    /// along an axis the edge does not move is ignored. The window never gets
    /// smaller than `min` (`width, height`), and the side opposite the held
    /// edge stays exactly where it was, which is why the position changes for
    /// the west and north edges.
    ///
    /// All values are in the same unit, which the caller chooses.
    pub fn dragged(self, origin: (f64, f64, f64, f64), delta: (f64, f64), min: (f64, f64)) -> (f64, f64, f64, f64) {
        let (_, _, width, height) = origin;
        let size = (
            if self.moves_x() { (width + if self.left() { -delta.0 } else { delta.0 }).max(min.0) } else { width },
            if self.moves_y() { (height + if self.top() { -delta.1 } else { delta.1 }).max(min.1) } else { height },
        );
        let (x, y) = self.anchored(origin, size);
        (x, y, size.0, size.1)
    }

    /// Where the window's top-left corner has to be for it to be `size` with the
    /// side opposite this edge exactly where `origin` (`x, y, width, height`) had
    /// it.
    ///
    /// [`dragged`](Self::dragged) asks for a size and the platform may give
    /// another, because of a minimum it keeps or a size it refuses; this is where
    /// to put the window for the size it actually has.
    pub fn anchored(self, origin: (f64, f64, f64, f64), size: (f64, f64)) -> (f64, f64) {
        let (x, y, width, height) = origin;
        (
            if self.left() { x + width - size.0 } else { x },
            if self.top() { y + height - size.1 } else { y },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: (f64, f64, f64, f64) = (100.0, 50.0, 800.0, 600.0);
    const MIN: (f64, f64) = (200.0, 150.0);

    #[test]
    fn the_east_edge_widens_the_window_and_leaves_its_left_side() {
        assert_eq!(WindowEdge::East.dragged(ORIGIN, (40.0, 0.0), MIN), (100.0, 50.0, 840.0, 600.0));
    }

    #[test]
    fn the_west_edge_dragged_left_widens_and_keeps_the_right_side() {
        let (x, _, width, _) = WindowEdge::West.dragged(ORIGIN, (-40.0, 0.0), MIN);
        assert_eq!((x, width), (60.0, 840.0));
        assert_eq!(x + width, 100.0 + 800.0, "the right side did not move");
    }

    #[test]
    fn the_north_and_south_edges_change_the_height_only() {
        assert_eq!(WindowEdge::South.dragged(ORIGIN, (0.0, -30.0), MIN), (100.0, 50.0, 800.0, 570.0));
        let (_, y, _, height) = WindowEdge::North.dragged(ORIGIN, (0.0, 30.0), MIN);
        assert_eq!((y, height), (80.0, 570.0));
        assert_eq!(y + height, 50.0 + 600.0, "the bottom did not move");
    }

    #[test]
    fn a_distance_along_an_axis_the_edge_does_not_move_is_ignored() {
        assert_eq!(WindowEdge::East.dragged(ORIGIN, (10.0, 999.0), MIN), (100.0, 50.0, 810.0, 600.0));
        assert_eq!(WindowEdge::North.dragged(ORIGIN, (999.0, 10.0), MIN), (100.0, 60.0, 800.0, 590.0));
    }

    #[test]
    fn each_corner_moves_both_of_its_sides() {
        assert_eq!(WindowEdge::SouthEast.dragged(ORIGIN, (10.0, 20.0), MIN), (100.0, 50.0, 810.0, 620.0));
        assert_eq!(WindowEdge::NorthWest.dragged(ORIGIN, (-10.0, -20.0), MIN), (90.0, 30.0, 810.0, 620.0));
        assert_eq!(WindowEdge::NorthEast.dragged(ORIGIN, (10.0, -20.0), MIN), (100.0, 30.0, 810.0, 620.0));
        assert_eq!(WindowEdge::SouthWest.dragged(ORIGIN, (-10.0, 20.0), MIN), (90.0, 50.0, 810.0, 620.0));
    }

    #[test]
    fn the_window_stops_at_its_minimum_and_the_far_side_stays_put() {
        let (x, _, width, _) = WindowEdge::West.dragged(ORIGIN, (5000.0, 0.0), MIN);
        assert_eq!(width, 200.0);
        assert_eq!(x + width, 900.0, "pushed past the minimum, the right side still did not move");
        let (_, _, _, height) = WindowEdge::South.dragged(ORIGIN, (0.0, -5000.0), MIN);
        assert_eq!(height, 150.0);
    }

    #[test]
    fn anchoring_follows_the_size_the_platform_really_gave() {
        // Asked for 840 wide, given 820: the right side is still at 900.
        let (x, y) = WindowEdge::NorthWest.anchored(ORIGIN, (820.0, 590.0));
        assert_eq!((x, y), (80.0, 60.0));
        assert_eq!(WindowEdge::SouthEast.anchored(ORIGIN, (820.0, 590.0)), (100.0, 50.0));
    }

    #[test]
    fn no_distance_is_no_change() {
        for edge in [
            WindowEdge::North,
            WindowEdge::South,
            WindowEdge::East,
            WindowEdge::West,
            WindowEdge::NorthEast,
            WindowEdge::NorthWest,
            WindowEdge::SouthEast,
            WindowEdge::SouthWest,
        ] {
            assert_eq!(edge.dragged(ORIGIN, (0.0, 0.0), MIN), ORIGIN, "{edge:?}");
        }
    }
}
