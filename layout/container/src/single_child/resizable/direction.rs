use std::ops::{
    BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not, Sub, SubAssign,
};

use aimer_macro::PortableValue;

use super::handle::ResizeHandle;

/// The set of sides a [`Resizable`](super::Resizable) may be dragged by.
///
/// A resizable has eight grab zones — four edges and four corners — and any
/// combination of them can be live at once, so this is a set of bit flags
/// rather than a single choice. Each of the eight [associated
/// constants](#associated-constants) is one bit; combine them with `|`, remove
/// them with `-`, intersect them with `&`, and invert them with `!`.
///
/// A zone that is not in the set is not a handle at all: the pointer that lands
/// on it gets the child's cursor and the child's events, exactly as if the band
/// were not there.
///
/// # Examples
///
/// A side panel the user may only widen, from its right edge:
///
/// ```
/// use aimer_container::Direction;
///
/// let sides = Direction::RIGHT;
///
/// assert!(sides.contains(Direction::RIGHT));
/// assert!(!sides.contains(Direction::LEFT));
/// ```
///
/// The bottom edge together with both of its corners:
///
/// ```
/// use aimer_container::Direction;
///
/// let sides = Direction::BOTTOM | Direction::BOTTOM_LEFT | Direction::BOTTOM_RIGHT;
///
/// assert!(sides.contains(Direction::BOTTOM_RIGHT));
/// assert!(!sides.intersects(Direction::TOP_EDGES));
/// ```
///
/// Everything except the top:
///
/// ```
/// use aimer_container::Direction;
///
/// let sides = Direction::ALL - Direction::TOP_EDGES;
///
/// assert!(sides.contains(Direction::BOTTOM));
/// assert!(!sides.contains(Direction::TOP_LEFT));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PortableValue)]
#[portable_value(
    id = "aimer.value:aimer_container::single_child::Direction",
    max_encoded_bytes = 16
)]
pub struct Direction(u8);

impl Direction {
    /// No side at all, which disables resizing.
    pub const NONE: Self = Self(0);
    /// The left edge, excluding its corners.
    pub const LEFT: Self = Self(1 << 0);
    /// The right edge, excluding its corners.
    pub const RIGHT: Self = Self(1 << 1);
    /// The top edge, excluding its corners.
    pub const TOP: Self = Self(1 << 2);
    /// The bottom edge, excluding its corners.
    pub const BOTTOM: Self = Self(1 << 3);
    /// The top-left corner.
    pub const TOP_LEFT: Self = Self(1 << 4);
    /// The top-right corner.
    pub const TOP_RIGHT: Self = Self(1 << 5);
    /// The bottom-left corner.
    pub const BOTTOM_LEFT: Self = Self(1 << 6);
    /// The bottom-right corner.
    pub const BOTTOM_RIGHT: Self = Self(1 << 7);

    /// The four edges, without any corner.
    pub const EDGES: Self = Self(Self::LEFT.0 | Self::RIGHT.0 | Self::TOP.0 | Self::BOTTOM.0);
    /// The four corners, without any edge.
    pub const CORNERS: Self =
        Self(Self::TOP_LEFT.0 | Self::TOP_RIGHT.0 | Self::BOTTOM_LEFT.0 | Self::BOTTOM_RIGHT.0);
    /// Every side that changes the width: both vertical edges and all four
    /// corners.
    pub const HORIZONTAL: Self = Self(Self::LEFT.0 | Self::RIGHT.0 | Self::CORNERS.0);
    /// Every side that changes the height: both horizontal edges and all four
    /// corners.
    pub const VERTICAL: Self = Self(Self::TOP.0 | Self::BOTTOM.0 | Self::CORNERS.0);
    /// The top edge and the two corners it meets.
    pub const TOP_EDGES: Self = Self(Self::TOP.0 | Self::TOP_LEFT.0 | Self::TOP_RIGHT.0);
    /// The bottom edge and the two corners it meets.
    pub const BOTTOM_EDGES: Self =
        Self(Self::BOTTOM.0 | Self::BOTTOM_LEFT.0 | Self::BOTTOM_RIGHT.0);
    /// The left edge and the two corners it meets.
    pub const LEFT_EDGES: Self = Self(Self::LEFT.0 | Self::TOP_LEFT.0 | Self::BOTTOM_LEFT.0);
    /// The right edge and the two corners it meets.
    pub const RIGHT_EDGES: Self = Self(Self::RIGHT.0 | Self::TOP_RIGHT.0 | Self::BOTTOM_RIGHT.0);
    /// All eight sides, which is what a [`Resizable`](super::Resizable) uses
    /// unless another set is asked for.
    pub const ALL: Self = Self(Self::EDGES.0 | Self::CORNERS.0);

    /// The raw bits of the set.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::Direction;
    ///
    /// assert_eq!(Direction::NONE.bits(), 0);
    /// assert_eq!(Direction::ALL.bits(), u8::MAX);
    /// ```
    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The set the raw `bits` describe.
    ///
    /// Every bit of a `u8` names one of the eight sides, so no bit has to be
    /// discarded and every input is a valid set.
    #[inline]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether the set holds no side, in which case nothing can be dragged.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::Direction;
    ///
    /// assert!(Direction::NONE.is_empty());
    /// assert!(!Direction::LEFT.is_empty());
    /// ```
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether **every** side of `other` is in this set.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::Direction;
    ///
    /// let sides = Direction::LEFT | Direction::RIGHT;
    ///
    /// assert!(sides.contains(Direction::LEFT));
    /// assert!(sides.contains(Direction::LEFT | Direction::RIGHT));
    /// assert!(!sides.contains(Direction::EDGES));
    /// ```
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether **any** side of `other` is in this set.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::Direction;
    ///
    /// assert!(Direction::CORNERS.intersects(Direction::TOP_EDGES));
    /// assert!(!Direction::EDGES.intersects(Direction::CORNERS));
    /// ```
    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// This set with every side of `other` added.
    ///
    /// The `const` counterpart of [`BitOr`], for building a set in a `const`
    /// item.
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The sides present in both sets.
    #[inline]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// This set with every side of `other` taken out.
    ///
    /// The `const` counterpart of [`Sub`].
    #[inline]
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Whether the handle `handle` may be dragged, which is exactly whether its
    /// own side is in this set.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_container::{Direction, ResizeHandle};
    ///
    /// let sides = Direction::RIGHT_EDGES;
    ///
    /// assert!(sides.allows(ResizeHandle::BottomRight));
    /// assert!(!sides.allows(ResizeHandle::Bottom));
    /// ```
    #[inline]
    pub const fn allows(self, handle: ResizeHandle) -> bool {
        self.contains(handle.direction())
    }
}

impl Default for Direction {
    /// Every side, matching what a [`Resizable`](super::Resizable) starts with.
    #[inline]
    fn default() -> Self {
        Self::ALL
    }
}

impl From<ResizeHandle> for Direction {
    /// The single side `handle` occupies.
    #[inline]
    fn from(handle: ResizeHandle) -> Self {
        handle.direction()
    }
}

impl BitOr for Direction {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for Direction {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Direction {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

impl BitAndAssign for Direction {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl BitXor for Direction {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self {
        Self(self.0 ^ rhs.0)
    }
}

impl BitXorAssign for Direction {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}

impl Sub for Direction {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        self.difference(rhs)
    }
}

impl SubAssign for Direction {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 &= !rhs.0;
    }
}

impl Not for Direction {
    type Output = Self;

    /// Every side this set leaves out.
    #[inline]
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_side_is_its_own_bit() {
        let sides = [
            Direction::LEFT,
            Direction::RIGHT,
            Direction::TOP,
            Direction::BOTTOM,
            Direction::TOP_LEFT,
            Direction::TOP_RIGHT,
            Direction::BOTTOM_LEFT,
            Direction::BOTTOM_RIGHT,
        ];

        let mut seen = Direction::NONE;
        for side in sides {
            assert_eq!(side.bits().count_ones(), 1, "{side:?} must be a single bit");
            assert!(!seen.intersects(side), "{side:?} shares a bit with another");
            seen |= side;
        }

        assert_eq!(seen, Direction::ALL, "the eight sides must cover the set");
    }

    #[test]
    fn the_groups_are_the_union_of_their_sides() {
        assert_eq!(
            Direction::EDGES,
            Direction::LEFT | Direction::RIGHT | Direction::TOP | Direction::BOTTOM
        );
        assert_eq!(
            Direction::CORNERS,
            Direction::TOP_LEFT
                | Direction::TOP_RIGHT
                | Direction::BOTTOM_LEFT
                | Direction::BOTTOM_RIGHT
        );
        assert_eq!(Direction::ALL, Direction::EDGES | Direction::CORNERS);
        assert_eq!(
            Direction::HORIZONTAL | Direction::VERTICAL,
            Direction::ALL,
            "the two axes together reach every side"
        );
    }

    #[test]
    fn contains_asks_for_every_side_and_intersects_for_any() {
        let sides = Direction::TOP_EDGES;

        assert!(sides.contains(Direction::TOP | Direction::TOP_LEFT));
        assert!(!sides.contains(Direction::TOP | Direction::BOTTOM));
        assert!(sides.intersects(Direction::TOP | Direction::BOTTOM));
        assert!(!sides.intersects(Direction::BOTTOM_EDGES));
    }

    #[test]
    fn removing_a_group_leaves_the_rest_untouched() {
        let sides = Direction::ALL - Direction::TOP_EDGES;

        assert!(sides.contains(Direction::BOTTOM_EDGES));
        assert!(sides.contains(Direction::LEFT | Direction::RIGHT));
        assert!(!sides.intersects(Direction::TOP_EDGES));
        assert_eq!(
            !sides,
            Direction::TOP_EDGES,
            "the complement is what was cut"
        );
    }

    #[test]
    fn the_empty_set_allows_nothing_and_the_full_set_allows_everything() {
        for handle in [
            ResizeHandle::Left,
            ResizeHandle::Right,
            ResizeHandle::Top,
            ResizeHandle::Bottom,
            ResizeHandle::TopLeft,
            ResizeHandle::TopRight,
            ResizeHandle::BottomLeft,
            ResizeHandle::BottomRight,
        ] {
            assert!(Direction::ALL.allows(handle), "{handle:?} must be allowed");
            assert!(!Direction::NONE.allows(handle), "{handle:?} must be denied");
            assert_eq!(Direction::from(handle), handle.direction());
        }

        assert!(Direction::NONE.is_empty());
        assert!(!Direction::ALL.is_empty());
    }

    #[test]
    fn assignment_operators_agree_with_their_expressions() {
        let mut sides = Direction::LEFT;
        sides |= Direction::RIGHT;
        assert_eq!(sides, Direction::LEFT | Direction::RIGHT);

        sides &= Direction::RIGHT_EDGES;
        assert_eq!(sides, Direction::RIGHT);

        sides -= Direction::RIGHT;
        assert_eq!(sides, Direction::NONE);

        sides ^= Direction::TOP;
        assert_eq!(sides, Direction::TOP);
    }

    #[test]
    fn a_set_survives_a_round_trip_through_its_bits() {
        let sides = Direction::TOP | Direction::BOTTOM_RIGHT;

        assert_eq!(Direction::from_bits(sides.bits()), sides);
    }
}
