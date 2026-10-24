use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_utils::cursor::CursorIcon;

use super::band::ResizeBand;
use super::direction::Direction;

/// One of the eight grab zones of a [`Resizable`](super::Resizable).
///
/// A handle is a band along one of the widget's own edges. It reaches
/// `thickness` logical pixels inside its bounds, and — given a [`ResizeBand`]
/// with an outer reach — a few pixels outside them as well. The four corners are
/// the overlap of two bands and win over either of them, which is what makes a
/// corner drag change both axes at once.
///
/// # Examples
///
/// ```
/// use aimer_attribute::bounds::Bounds;
/// use aimer_attribute::position::Vec2d;
/// use aimer_container::{CursorIcon, ResizeHandle};
///
/// let bounds = Bounds::new(0.0, 0.0, 200.0, 100.0);
///
/// let corner = ResizeHandle::hit(bounds, Vec2d { x: 198.0, y: 98.0 }, 6.0);
/// assert_eq!(corner, Some(ResizeHandle::BottomRight));
/// assert_eq!(corner.unwrap().cursor(), CursorIcon::NwseResize);
///
/// // The middle of the widget belongs to its child, not to a handle.
/// assert_eq!(
///     ResizeHandle::hit(bounds, Vec2d { x: 100.0, y: 50.0 }, 6.0),
///     None
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    /// The left edge, excluding its corners.
    Left,
    /// The right edge, excluding its corners.
    Right,
    /// The top edge, excluding its corners.
    Top,
    /// The bottom edge, excluding its corners.
    Bottom,
    /// The top-left corner.
    TopLeft,
    /// The top-right corner.
    TopRight,
    /// The bottom-left corner.
    BottomLeft,
    /// The bottom-right corner.
    BottomRight,
}

impl ResizeHandle {
    /// The handle `point` lands on, or `None` when it lands on neither.
    ///
    /// `point` and `thickness` are both in the same space as `bounds`, which is
    /// logical pixels everywhere in the framework. A `thickness` of zero or less
    /// disables every handle, and a point outside `bounds` never hits one — a
    /// drag that leaves the widget keeps the handle it started on instead of
    /// asking again.
    ///
    /// When a widget is narrower or shorter than two bands the bands overlap; the
    /// nearer edge wins, and a dead tie goes to the right or bottom one, so a
    /// collapsed widget can still be dragged back open.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::bounds::Bounds;
    /// use aimer_attribute::position::Vec2d;
    /// use aimer_container::ResizeHandle;
    ///
    /// let bounds = Bounds::new(10.0, 10.0, 100.0, 100.0);
    ///
    /// assert_eq!(
    ///     ResizeHandle::hit(bounds, Vec2d { x: 108.0, y: 60.0 }, 4.0),
    ///     Some(ResizeHandle::Right)
    /// );
    /// assert_eq!(
    ///     ResizeHandle::hit(bounds, Vec2d { x: 500.0, y: 60.0 }, 4.0),
    ///     None
    /// );
    /// ```
    #[inline]
    pub fn hit(bounds: Bounds, point: Vec2d, thickness: f32) -> Option<Self> {
        Self::hit_in(bounds, point, thickness, Direction::ALL)
    }

    /// The handle `point` lands on among the sides `direction` allows, or `None`
    /// when it lands on none of them.
    ///
    /// Behaves like [`ResizeHandle::hit`] with the sides left out of `direction`
    /// removed: a band that is not allowed is not a handle, so the point belongs
    /// to the child instead. A corner that is not allowed falls back to the edge
    /// it overlaps — the one whose band the point is nearer to — which is what
    /// keeps the right edge grabbable right down to a corner that is switched
    /// off.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::bounds::Bounds;
    /// use aimer_attribute::position::Vec2d;
    /// use aimer_container::{Direction, ResizeHandle};
    ///
    /// let bounds = Bounds::new(0.0, 0.0, 200.0, 100.0);
    /// let sides = Direction::RIGHT;
    ///
    /// assert_eq!(
    ///     ResizeHandle::hit_in(bounds, Vec2d { x: 198.0, y: 50.0 }, 6.0, sides),
    ///     Some(ResizeHandle::Right)
    /// );
    /// // The left edge is not a handle at all now.
    /// assert_eq!(
    ///     ResizeHandle::hit_in(bounds, Vec2d { x: 2.0, y: 50.0 }, 6.0, sides),
    ///     None
    /// );
    /// // The bottom-right corner is off, so its right edge answers instead.
    /// assert_eq!(
    ///     ResizeHandle::hit_in(bounds, Vec2d { x: 198.0, y: 98.0 }, 6.0, sides),
    ///     Some(ResizeHandle::Right)
    /// );
    /// ```
    pub fn hit_in(
        bounds: Bounds,
        point: Vec2d,
        thickness: f32,
        direction: Direction,
    ) -> Option<Self> {
        Self::hit_band(bounds, point, ResizeBand::inside(thickness), direction)
    }

    /// The handle `point` lands on when the grab bands are `band` wide, or `None`
    /// when it lands on none of the sides `direction` allows.
    ///
    /// The general form of [`ResizeHandle::hit_in`], which uses a band lying
    /// wholly inside the widget. A [`ResizeBand`] with an outer reach also claims
    /// the pixels just *outside* the border, the way a window edge does, so the
    /// cursor changes as the pointer arrives at the border rather than once it has
    /// crossed it.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::bounds::Bounds;
    /// use aimer_attribute::position::Vec2d;
    /// use aimer_container::{Direction, ResizeBand, ResizeHandle};
    ///
    /// let bounds = Bounds::new(0.0, 0.0, 200.0, 100.0);
    /// let band = ResizeBand::new(6.0, 6.0);
    ///
    /// // Four pixels short of the right border.
    /// assert_eq!(
    ///     ResizeHandle::hit_band(bounds, Vec2d { x: 204.0, y: 50.0 }, band, Direction::ALL),
    ///     Some(ResizeHandle::Right)
    /// );
    /// // Past the outer reach, the pointer belongs to whatever is out there.
    /// assert_eq!(
    ///     ResizeHandle::hit_band(bounds, Vec2d { x: 208.0, y: 50.0 }, band, Direction::ALL),
    ///     None
    /// );
    /// // Diagonally off the corner is still the corner.
    /// assert_eq!(
    ///     ResizeHandle::hit_band(bounds, Vec2d { x: 203.0, y: 103.0 }, band, Direction::ALL),
    ///     Some(ResizeHandle::BottomRight)
    /// );
    /// ```
    pub fn hit_band(
        bounds: Bounds,
        point: Vec2d,
        band: ResizeBand,
        direction: Direction,
    ) -> Option<Self> {
        // The outer reach makes a point beyond one border but level with the
        // middle of the widget a legitimate hit, so the grown rectangle — not the
        // widget's own — is what rules a far-away point out.
        if band.is_empty() || direction.is_empty() || !band.grow(bounds).is_inside(point.x, point.y)
        {
            return None;
        }

        let from_left = point.x - bounds.x;
        let from_right = bounds.x + bounds.width - point.x;
        let from_top = point.y - bounds.y;
        let from_bottom = bounds.y + bounds.height - point.y;

        let horizontal = nearest_edge(from_left, from_right, band);
        let vertical = nearest_edge(from_top, from_bottom, band);

        match (horizontal, vertical) {
            (Some((horizontal, across)), Some((vertical, down))) => {
                let corner = Self::corner(horizontal, vertical);
                if direction.allows(corner) {
                    return Some(corner);
                }

                let across_edge = Self::left_or_right(horizontal);
                let down_edge = Self::top_or_bottom(vertical);
                let (nearer, farther) = if across <= down {
                    (across_edge, down_edge)
                } else {
                    (down_edge, across_edge)
                };
                if direction.allows(nearer) {
                    Some(nearer)
                } else if direction.allows(farther) {
                    Some(farther)
                } else {
                    None
                }
            }
            (Some((horizontal, _)), None) => {
                Some(Self::left_or_right(horizontal)).filter(|handle| direction.allows(*handle))
            }
            (None, Some((vertical, _))) => {
                Some(Self::top_or_bottom(vertical)).filter(|handle| direction.allows(*handle))
            }
            (None, None) => None,
        }
    }

    /// The single side this handle occupies, as a one-bit [`Direction`].
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{Direction, ResizeHandle};
    ///
    /// assert_eq!(ResizeHandle::Left.direction(), Direction::LEFT);
    /// assert_eq!(ResizeHandle::TopRight.direction(), Direction::TOP_RIGHT);
    /// ```
    #[inline]
    pub const fn direction(self) -> Direction {
        match self {
            Self::Left => Direction::LEFT,
            Self::Right => Direction::RIGHT,
            Self::Top => Direction::TOP,
            Self::Bottom => Direction::BOTTOM,
            Self::TopLeft => Direction::TOP_LEFT,
            Self::TopRight => Direction::TOP_RIGHT,
            Self::BottomLeft => Direction::BOTTOM_LEFT,
            Self::BottomRight => Direction::BOTTOM_RIGHT,
        }
    }

    /// The corner where the two bands meet.
    #[inline]
    const fn corner(horizontal: Edge, vertical: Edge) -> Self {
        match (horizontal, vertical) {
            (Edge::Start, Edge::Start) => Self::TopLeft,
            (Edge::End, Edge::Start) => Self::TopRight,
            (Edge::Start, Edge::End) => Self::BottomLeft,
            (Edge::End, Edge::End) => Self::BottomRight,
        }
    }

    /// The edge of the horizontal axis `horizontal` names.
    #[inline]
    const fn left_or_right(horizontal: Edge) -> Self {
        match horizontal {
            Edge::Start => Self::Left,
            Edge::End => Self::Right,
        }
    }

    /// The edge of the vertical axis `vertical` names.
    #[inline]
    const fn top_or_bottom(vertical: Edge) -> Self {
        match vertical {
            Edge::Start => Self::Top,
            Edge::End => Self::Bottom,
        }
    }

    /// The cursor this handle asks the window to show while it is hovered or
    /// dragged.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{CursorIcon, ResizeHandle};
    ///
    /// assert_eq!(ResizeHandle::Left.cursor(), CursorIcon::EwResize);
    /// assert_eq!(ResizeHandle::Top.cursor(), CursorIcon::NsResize);
    /// assert_eq!(ResizeHandle::TopRight.cursor(), CursorIcon::NeswResize);
    /// ```
    #[inline]
    pub const fn cursor(self) -> CursorIcon {
        match self {
            Self::Left | Self::Right => CursorIcon::EwResize,
            Self::Top | Self::Bottom => CursorIcon::NsResize,
            Self::TopLeft | Self::BottomRight => CursorIcon::NwseResize,
            Self::TopRight | Self::BottomLeft => CursorIcon::NeswResize,
        }
    }

    /// `size` grown by a pointer displacement of `delta` on this handle.
    ///
    /// A widget is placed by its parent, so resizing changes the size alone and
    /// the top-left corner stays where the layout put it. Dragging the right or
    /// bottom edge outwards therefore grows the widget in the direction of the
    /// pointer, while dragging the left or top edge outwards — to smaller `x` or
    /// `y` — grows it *away* from the pointer, since there is no other direction
    /// for it to grow in.
    ///
    /// The result is not clamped; [`Resizable`](super::Resizable) applies its
    /// own bounds afterwards.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_attribute::position::Vec2d;
    /// use aimer_attribute::size::ResolvedSize;
    /// use aimer_container::ResizeHandle;
    ///
    /// let size = ResolvedSize { width: 100.0, height: 50.0 };
    ///
    /// let wider = ResizeHandle::Right.resize(size, Vec2d { x: 20.0, y: 0.0 });
    /// assert_eq!(wider.width, 120.0);
    ///
    /// // Pulling the left edge leftwards is also a request for more width.
    /// let also_wider = ResizeHandle::Left.resize(size, Vec2d { x: -20.0, y: 0.0 });
    /// assert_eq!(also_wider.width, 120.0);
    /// ```
    #[inline]
    pub fn resize(self, size: ResolvedSize, delta: Vec2d) -> ResolvedSize {
        let width = match self {
            Self::Right | Self::TopRight | Self::BottomRight => size.width + delta.x,
            Self::Left | Self::TopLeft | Self::BottomLeft => size.width - delta.x,
            Self::Top | Self::Bottom => size.width,
        };
        let height = match self {
            Self::Bottom | Self::BottomLeft | Self::BottomRight => size.height + delta.y,
            Self::Top | Self::TopLeft | Self::TopRight => size.height - delta.y,
            Self::Left | Self::Right => size.height,
        };

        ResolvedSize { width, height }
    }
}

/// Which end of one axis a point is near, used to pair the two axes into a
/// handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Start,
    End,
}

/// The end of one axis whose band `start` and `end` fall in, with the distance
/// to it, or `None` when the point is in the middle of that axis.
///
/// `start` and `end` are signed distances to the two edge lines — negative once
/// the point is past one of them — so the answer is ranked by how far the point
/// is from the line either way.
///
/// When a widget is thinner than two bands both are hit at once; the nearer end
/// wins, and a dead tie goes to the end one, so a collapsed widget can still be
/// dragged back open.
#[inline]
fn nearest_edge(start: f32, end: f32, band: ResizeBand) -> Option<(Edge, f32)> {
    match (band.holds(start), band.holds(end)) {
        (true, true) if start.abs() < end.abs() => Some((Edge::Start, start.abs())),
        (true, true) => Some((Edge::End, end.abs())),
        (true, false) => Some((Edge::Start, start.abs())),
        (false, true) => Some((Edge::End, end.abs())),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Bounds = Bounds::new(10.0, 20.0, 200.0, 100.0);
    const THICKNESS: f32 = 8.0;

    fn hit(x: f32, y: f32) -> Option<ResizeHandle> {
        ResizeHandle::hit(BOUNDS, Vec2d { x, y }, THICKNESS)
    }

    fn hit_in(x: f32, y: f32, direction: Direction) -> Option<ResizeHandle> {
        ResizeHandle::hit_in(BOUNDS, Vec2d { x, y }, THICKNESS, direction)
    }

    #[test]
    fn the_interior_belongs_to_the_child() {
        assert_eq!(hit(110.0, 70.0), None);
    }

    #[test]
    fn each_edge_band_reports_its_own_handle() {
        assert_eq!(hit(12.0, 70.0), Some(ResizeHandle::Left));
        assert_eq!(hit(208.0, 70.0), Some(ResizeHandle::Right));
        assert_eq!(hit(110.0, 22.0), Some(ResizeHandle::Top));
        assert_eq!(hit(110.0, 118.0), Some(ResizeHandle::Bottom));
    }

    #[test]
    fn a_corner_wins_over_the_two_edges_it_overlaps() {
        assert_eq!(hit(12.0, 22.0), Some(ResizeHandle::TopLeft));
        assert_eq!(hit(208.0, 22.0), Some(ResizeHandle::TopRight));
        assert_eq!(hit(12.0, 118.0), Some(ResizeHandle::BottomLeft));
        assert_eq!(hit(208.0, 118.0), Some(ResizeHandle::BottomRight));
    }

    #[test]
    fn a_point_outside_the_bounds_hits_nothing() {
        assert_eq!(hit(5.0, 70.0), None);
        assert_eq!(hit(300.0, 70.0), None);
        assert_eq!(hit(110.0, 5.0), None);
    }

    #[test]
    fn a_band_with_an_outer_reach_claims_the_pixels_before_the_border() {
        let bounds = Bounds::new(10.0, 10.0, 200.0, 100.0);
        let band = ResizeBand::new(6.0, 6.0);

        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 214.0, y: 60.0 }, band, Direction::ALL),
            Some(ResizeHandle::Right),
            "four pixels short of the right border"
        );
        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 6.0, y: 60.0 }, band, Direction::ALL),
            Some(ResizeHandle::Left)
        );
        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 100.0, y: 6.0 }, band, Direction::ALL),
            Some(ResizeHandle::Top)
        );
        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 213.0, y: 113.0 }, band, Direction::ALL),
            Some(ResizeHandle::BottomRight),
            "diagonally off a corner is still that corner"
        );
    }

    // Without the grown rectangle a point level with the middle of the widget but
    // an inch beyond its border would still be within one band of the top edge.
    #[test]
    fn a_point_past_the_outer_reach_hits_nothing() {
        let bounds = Bounds::new(10.0, 10.0, 200.0, 100.0);
        let band = ResizeBand::new(6.0, 6.0);

        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 217.0, y: 60.0 }, band, Direction::ALL),
            None
        );
        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 400.0, y: 12.0 }, band, Direction::ALL),
            None,
            "level with the top band, but nowhere near the widget"
        );
    }

    #[test]
    fn a_band_left_out_of_the_direction_is_no_handle_outside_the_border_either() {
        let bounds = Bounds::new(10.0, 10.0, 200.0, 100.0);
        let band = ResizeBand::new(6.0, 6.0);

        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 6.0, y: 60.0 }, band, Direction::RIGHT),
            None
        );
        assert_eq!(
            ResizeHandle::hit_band(bounds, Vec2d { x: 214.0, y: 60.0 }, band, Direction::RIGHT),
            Some(ResizeHandle::Right)
        );
    }

    #[test]
    fn an_inside_only_band_answers_exactly_as_the_thickness_does() {
        let bounds = Bounds::new(10.0, 10.0, 200.0, 100.0);

        for point in [
            Vec2d { x: 12.0, y: 60.0 },
            Vec2d { x: 208.0, y: 108.0 },
            Vec2d { x: 100.0, y: 60.0 },
            Vec2d { x: 8.0, y: 60.0 },
        ] {
            assert_eq!(
                ResizeHandle::hit_band(bounds, point, ResizeBand::inside(6.0), Direction::ALL),
                ResizeHandle::hit(bounds, point, 6.0),
                "{point:?}"
            );
        }
    }

    #[test]
    fn a_thickness_of_zero_disables_every_handle() {
        assert_eq!(
            ResizeHandle::hit(BOUNDS, Vec2d { x: 10.0, y: 20.0 }, 0.0),
            None
        );
    }

    // A widget dragged down to a sliver has overlapping bands: the far edge must
    // stay reachable, otherwise the sliver can never be dragged back open.
    #[test]
    fn overlapping_bands_resolve_to_the_nearer_edge() {
        let sliver = Bounds::new(0.0, 0.0, 10.0, 100.0);

        assert_eq!(
            ResizeHandle::hit(sliver, Vec2d { x: 1.0, y: 50.0 }, THICKNESS),
            Some(ResizeHandle::Left)
        );
        assert_eq!(
            ResizeHandle::hit(sliver, Vec2d { x: 9.0, y: 50.0 }, THICKNESS),
            Some(ResizeHandle::Right)
        );
        assert_eq!(
            ResizeHandle::hit(sliver, Vec2d { x: 5.0, y: 50.0 }, THICKNESS),
            Some(ResizeHandle::Right),
            "a dead tie goes to the growing edge"
        );
    }

    #[test]
    fn a_band_left_out_of_the_direction_is_not_a_handle() {
        let sides = Direction::RIGHT_EDGES;

        assert_eq!(hit_in(208.0, 70.0, sides), Some(ResizeHandle::Right));
        assert_eq!(hit_in(12.0, 70.0, sides), None);
        assert_eq!(hit_in(110.0, 22.0, sides), None);
        assert_eq!(hit_in(208.0, 118.0, sides), Some(ResizeHandle::BottomRight));
    }

    // Switching a corner off must not punch a hole in the edges it overlaps,
    // otherwise the last few pixels of a live edge stop responding.
    #[test]
    fn a_corner_that_is_off_falls_back_to_the_nearer_live_edge() {
        let edges = Direction::EDGES;

        assert_eq!(
            hit_in(209.0, 114.0, edges),
            Some(ResizeHandle::Right),
            "nearer to the right edge than to the bottom one"
        );
        assert_eq!(
            hit_in(204.0, 119.0, edges),
            Some(ResizeHandle::Bottom),
            "nearer to the bottom edge than to the right one"
        );
        assert_eq!(
            hit_in(209.0, 114.0, Direction::BOTTOM),
            Some(ResizeHandle::Bottom),
            "the nearer edge is off, so the farther live one answers"
        );
    }

    #[test]
    fn an_empty_direction_disables_every_handle() {
        for (x, y) in [(12.0, 70.0), (208.0, 70.0), (110.0, 22.0), (208.0, 118.0)] {
            assert_eq!(hit_in(x, y, Direction::NONE), None);
        }
    }

    #[test]
    fn the_default_direction_hits_exactly_what_hit_does() {
        for (x, y) in [
            (110.0, 70.0),
            (12.0, 70.0),
            (208.0, 70.0),
            (110.0, 22.0),
            (110.0, 118.0),
            (12.0, 22.0),
            (208.0, 118.0),
        ] {
            assert_eq!(hit(x, y), hit_in(x, y, Direction::default()));
        }
    }

    #[test]
    fn every_handle_asks_for_the_cursor_of_its_axis() {
        assert_eq!(ResizeHandle::Left.cursor(), CursorIcon::EwResize);
        assert_eq!(ResizeHandle::Right.cursor(), CursorIcon::EwResize);
        assert_eq!(ResizeHandle::Top.cursor(), CursorIcon::NsResize);
        assert_eq!(ResizeHandle::Bottom.cursor(), CursorIcon::NsResize);
        assert_eq!(ResizeHandle::TopLeft.cursor(), CursorIcon::NwseResize);
        assert_eq!(ResizeHandle::BottomRight.cursor(), CursorIcon::NwseResize);
        assert_eq!(ResizeHandle::TopRight.cursor(), CursorIcon::NeswResize);
        assert_eq!(ResizeHandle::BottomLeft.cursor(), CursorIcon::NeswResize);
    }

    #[test]
    fn a_corner_drag_changes_both_axes_at_once() {
        let size = ResolvedSize {
            width: 100.0,
            height: 50.0,
        };
        let delta = Vec2d { x: -10.0, y: -5.0 };

        let resized = ResizeHandle::TopLeft.resize(size, delta);

        assert_eq!(resized.width, 110.0);
        assert_eq!(resized.height, 55.0);
    }

    #[test]
    fn an_edge_drag_leaves_the_other_axis_untouched() {
        let size = ResolvedSize {
            width: 100.0,
            height: 50.0,
        };
        let delta = Vec2d { x: 25.0, y: 40.0 };

        assert_eq!(
            ResizeHandle::Right.resize(size, delta),
            ResolvedSize {
                width: 125.0,
                height: 50.0
            }
        );
        assert_eq!(
            ResizeHandle::Bottom.resize(size, delta),
            ResolvedSize {
                width: 100.0,
                height: 90.0
            }
        );
    }
}
