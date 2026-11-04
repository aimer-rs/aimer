use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

/// Composable optical effects, axes, and anchors for a magnifying decoration.
///
/// Combine flags with `|`. At least one optical effect must be selected:
/// [`UNIFORM`](Self::UNIFORM), [`BULGE`](Self::BULGE),
/// [`REFRACTION`](Self::REFRACTION), or [`RADIAL`](Self::RADIAL). Axis and
/// anchor flags alone paint nothing.
/// No axis flags means both axes; no anchors means center. Opposing anchors
/// cancel to center on their axis, independently of flag ordering.
///
/// Anchors move the origin of uniform zoom and bulge. Refraction follows the
/// box outline, so it can move samples even at the enlargement anchor. Axis
/// flags constrain displacement from every effect, including refraction.
///
/// ```ignore
/// use aimer_cupid::material::MagnificationStyle as Style;
/// let toolbar = Style::UNIFORM | Style::HORIZONTAL | Style::LEFT;
/// assert!(toolbar.contains(Style::UNIFORM | Style::LEFT));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MagnificationStyle(u32);

impl MagnificationStyle {
    /// No effects. Axes and anchors alone are also inert.
    pub const NONE: Self = Self(0);
    /// Enlarges uniformly around the selected anchor using the zoom factor.
    pub const UNIFORM: Self = Self(1 << 0);
    /// Adds a smooth elliptical swell around the selected anchor.
    pub const BULGE: Self = Self(1 << 1);
    /// Bends backdrop samples near the outline using the glass thickness.
    pub const REFRACTION: Self = Self(1 << 2);
    /// Allows horizontal displacement; combine with `VERTICAL` for both axes.
    pub const HORIZONTAL: Self = Self(1 << 3);
    /// Allows vertical displacement; combine with `HORIZONTAL` for both axes.
    pub const VERTICAL: Self = Self(1 << 4);
    /// Anchors enlargement to the left edge, unless `RIGHT` is also selected.
    pub const LEFT: Self = Self(1 << 5);
    /// Anchors enlargement to the right edge, unless `LEFT` is also selected.
    pub const RIGHT: Self = Self(1 << 6);
    /// Anchors enlargement to the top edge, unless `BOTTOM` is also selected.
    pub const TOP: Self = Self(1 << 7);
    /// Anchors enlargement to the bottom edge, unless `TOP` is also selected.
    pub const BOTTOM: Self = Self(1 << 8);
    /// Pulls backdrop samples toward the middle of the box, smoothly across the
    /// whole of it: nothing at the center, growing toward the rim, where the pull
    /// is the glass thickness. Where [`REFRACTION`](Self::REFRACTION) bends only
    /// a band at the rim and leaves the middle flat, this is a continuous field,
    /// so content near the rim is compressed and then folded over. Select one or
    /// the other; selecting both lets this one decide.
    pub const RADIAL: Self = Self(1 << 9);
    /// Centered uniform zoom and edge refraction on both axes.
    pub const DEFAULT: Self = Self(Self::UNIFORM.0 | Self::REFRACTION.0);
    /// All known flags. Opposing anchors resolve to center.
    pub const ALL: Self = Self((1 << 10) - 1);

    /// Returns the stable flag bits.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Reconstructs flags, returning `None` if any bit is unknown.
    #[inline]
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    /// Returns whether every flag in `other` is present.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Returns whether any flag in `other` is present.
    #[inline]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Returns whether no flags are present.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn axes(self) -> [f32; 2] {
        match self.0 & (Self::HORIZONTAL.0 | Self::VERTICAL.0) {
            bits if bits == Self::HORIZONTAL.0 => [1.0, 0.0],
            bits if bits == Self::VERTICAL.0 => [0.0, 1.0],
            _ => [1.0, 1.0],
        }
    }

    pub(crate) fn anchor(self) -> [f32; 2] {
        let coordinate = |low, high| match (self.contains(low), self.contains(high)) {
            (true, false) => 0.0,
            (false, true) => 1.0,
            _ => 0.5,
        };
        [
            coordinate(Self::LEFT, Self::RIGHT),
            coordinate(Self::TOP, Self::BOTTOM),
        ]
    }
}

impl Default for MagnificationStyle {
    #[inline]
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl BitOr for MagnificationStyle {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for MagnificationStyle {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for MagnificationStyle {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl BitAndAssign for MagnificationStyle {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
impl Not for MagnificationStyle {
    type Output = Self;
    #[inline]
    fn not(self) -> Self {
        Self(!self.0 & Self::ALL.0)
    }
}

#[cfg(test)]
mod tests {
    use super::MagnificationStyle as S;

    #[test]
    fn flags_combine_validate_and_remove_without_unknown_bits() {
        let mut style = S::UNIFORM | S::LEFT;
        style |= S::HORIZONTAL | S::BULGE;
        assert!(style.contains(S::LEFT | S::BULGE));
        assert!(!style.contains(S::REFRACTION));
        style &= !S::BULGE;
        assert_eq!(style & S::BULGE, S::NONE);
        assert_eq!(style | !style, S::ALL);
        assert_eq!(S::from_bits(style.bits()), Some(style));
        assert_eq!(S::from_bits(1 << 10), None);
        assert_eq!(S::from_bits(S::RADIAL.bits()), Some(S::RADIAL));
        assert!(S::ALL.contains(S::RADIAL));
        assert_eq!(S::from_bits(u32::MAX), None);
        assert!(S::NONE.is_empty());
        assert_eq!(S::default(), S::UNIFORM | S::REFRACTION);
    }

    #[test]
    fn axes_and_anchors_resolve_independently() {
        assert_eq!(S::NONE.axes(), [1.0, 1.0]);
        assert_eq!(S::HORIZONTAL.axes(), [1.0, 0.0]);
        assert_eq!(S::VERTICAL.axes(), [0.0, 1.0]);
        assert_eq!((S::HORIZONTAL | S::VERTICAL).axes(), [1.0, 1.0]);
        for (style, anchor) in [
            (S::NONE, [0.5, 0.5]),
            (S::LEFT | S::TOP, [0.0, 0.0]),
            (S::RIGHT | S::BOTTOM, [1.0, 1.0]),
            (S::LEFT | S::RIGHT | S::TOP, [0.5, 0.0]),
            (S::BOTTOM | S::TOP | S::LEFT, [0.0, 0.5]),
            (S::ALL, [0.5, 0.5]),
        ] {
            assert_eq!(style.anchor(), anchor);
        }
    }
}
