//! Which input devices are allowed to rubber-band a bouncy edge.
//!
//! A bouncy viewport is not equally trustworthy on every input device. A
//! touch screen reports the exact frame a finger lands and lifts, so the
//! stretch it produces always ends with the gesture. A browser `wheel` stream
//! reports neither: it keeps delivering its own momentum after the user has
//! let go, and a rubber-band edge fed by it stretches on deltas nobody is
//! producing.
//!
//! [`OverscrollSources`] is the bitmap that resolves this per device instead of
//! per viewport, so a browser can keep its bouncy touch scrolling while its
//! wheel stream clamps.

use std::ops::{BitOr, BitOrAssign};

/// One kind of input that can stretch a bouncy edge.
///
/// Each variant is a single bit of an [`OverscrollSources`] bitmap; the
/// discriminants are the bit values themselves so the conversion is a move,
/// not a table lookup.
///
/// # Examples
///
/// ```rust
/// use aimer_scroll::{OverscrollSource, OverscrollSources};
///
/// let sources = OverscrollSource::Touch | OverscrollSource::Mouse;
/// assert!(sources.contains(OverscrollSource::Touch));
/// assert!(!sources.contains(OverscrollSource::Wheel));
/// assert_eq!(OverscrollSources::from(OverscrollSource::Touch).bits(), 1 << 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OverscrollSource {
    /// Mouse-wheel and trackpad scroll frames, including the momentum a
    /// platform keeps delivering after the gesture ends.
    Wheel = 1 << 0,
    /// Finger drags on a touch screen.
    Touch = 1 << 1,
    /// Mouse-button drags of the content or of a scrollbar thumb.
    Mouse = 1 << 2,
    /// Arrow / page / home / end keys.
    Keyboard = 1 << 3,
}

impl OverscrollSource {
    /// The single bit this source occupies in an [`OverscrollSources`] bitmap.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use aimer_scroll::OverscrollSource;
    ///
    /// assert_eq!(OverscrollSource::Wheel.bit(), 1);
    /// ```
    #[inline]
    pub const fn bit(self) -> u8 {
        self as u8
    }
}

impl BitOr for OverscrollSource {
    type Output = OverscrollSources;

    #[inline]
    fn bitor(self, rhs: Self) -> OverscrollSources {
        OverscrollSources(self.bit() | rhs.bit())
    }
}

/// The set of input devices allowed to rubber-band a bouncy edge.
///
/// A viewport still has to declare bouncy edges through
/// [`ScrollBehavior::bouncy`](crate::ScrollBehavior); this bitmap can only take
/// the bounce away from a device, never grant it to a rigid viewport.
///
/// # Examples
///
/// ```rust
/// use aimer_scroll::{OverscrollSource, OverscrollSources};
///
/// // Everything but the wheel — what the web target uses by default.
/// let sources = OverscrollSources::ALL.without(OverscrollSource::Wheel);
/// assert!(sources.contains(OverscrollSource::Touch));
/// assert!(!sources.contains(OverscrollSource::Wheel));
///
/// // Build one up from nothing instead.
/// let touch_only = OverscrollSources::NONE.with(OverscrollSource::Touch);
/// assert_eq!(touch_only, OverscrollSources::from(OverscrollSource::Touch));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverscrollSources(u8);

impl OverscrollSources {
    /// No device rubber-bands; every edge clamps like
    /// [`ScrollBehavior::no_bounce`](crate::ScrollBehavior::no_bounce).
    pub const NONE: Self = Self(0);

    /// Every device rubber-bands — what a native target uses.
    pub const ALL: Self = Self(
        OverscrollSource::Wheel.bit()
            | OverscrollSource::Touch.bit()
            | OverscrollSource::Mouse.bit()
            | OverscrollSource::Keyboard.bit(),
    );

    /// What the web target allows unless the app says otherwise:
    /// everything except [`OverscrollSource::Wheel`].
    ///
    /// A browser never reports the end of a wheel gesture and appends a
    /// momentum tail of its own, so a rubber-band edge fed by that stream
    /// fights input the user is no longer producing. Every other device is a
    /// normal, fully reported gesture even in a browser.
    pub const WEB_DEFAULT: Self = Self::ALL.without(OverscrollSource::Wheel);

    /// Wraps a raw bit pattern, ignoring bits that name no source.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use aimer_scroll::{OverscrollSource, OverscrollSources};
    ///
    /// let sources = OverscrollSources::from_bits_truncate(0b1111_0010);
    /// assert_eq!(sources, OverscrollSources::from(OverscrollSource::Touch));
    /// ```
    #[inline]
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// The raw bit pattern behind this set.
    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether `source` may stretch a bouncy edge.
    #[inline]
    pub const fn contains(self, source: OverscrollSource) -> bool {
        self.0 & source.bit() != 0
    }

    /// Whether no device at all may stretch a bouncy edge.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// This set plus `source`.
    #[inline]
    pub const fn with(self, source: OverscrollSource) -> Self {
        Self(self.0 | source.bit())
    }

    /// This set minus `source`.
    #[inline]
    pub const fn without(self, source: OverscrollSource) -> Self {
        Self(self.0 & !source.bit())
    }

    /// The union of two sets.
    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// The intersection of two sets.
    #[inline]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl Default for OverscrollSources {
    /// [`OverscrollSources::ALL`] — a bouncy viewport bounces on every device
    /// until a target or an app narrows the set.
    #[inline]
    fn default() -> Self {
        Self::ALL
    }
}

impl From<OverscrollSource> for OverscrollSources {
    #[inline]
    fn from(source: OverscrollSource) -> Self {
        Self(source.bit())
    }
}

impl BitOr<OverscrollSource> for OverscrollSources {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: OverscrollSource) -> Self {
        self.with(rhs)
    }
}

impl BitOr for OverscrollSources {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign<OverscrollSource> for OverscrollSources {
    #[inline]
    fn bitor_assign(&mut self, rhs: OverscrollSource) {
        self.0 |= rhs.bit();
    }
}

#[cfg(test)]
mod tests {
    use super::{OverscrollSource, OverscrollSources};

    #[test]
    fn every_source_owns_a_distinct_bit() {
        let all = [
            OverscrollSource::Wheel,
            OverscrollSource::Touch,
            OverscrollSource::Mouse,
            OverscrollSource::Keyboard,
        ];
        let mut seen = 0u8;
        for source in all {
            assert_eq!(source.bit().count_ones(), 1, "{source:?} is not one bit");
            assert_eq!(seen & source.bit(), 0, "{source:?} reuses another bit");
            seen |= source.bit();
        }
        assert_eq!(seen, OverscrollSources::ALL.bits());
    }

    #[test]
    fn the_full_set_holds_every_source_and_the_empty_one_holds_none() {
        for source in [
            OverscrollSource::Wheel,
            OverscrollSource::Touch,
            OverscrollSource::Mouse,
            OverscrollSource::Keyboard,
        ] {
            assert!(OverscrollSources::ALL.contains(source));
            assert!(!OverscrollSources::NONE.contains(source));
        }
        assert!(OverscrollSources::NONE.is_empty());
        assert!(!OverscrollSources::ALL.is_empty());
        assert_eq!(OverscrollSources::default(), OverscrollSources::ALL);
    }

    #[test]
    fn the_web_default_clamps_the_wheel_and_nothing_else() {
        let web = OverscrollSources::WEB_DEFAULT;

        assert!(
            !web.contains(OverscrollSource::Wheel),
            "a browser wheel stream never reports the end of its gesture"
        );
        assert!(
            web.contains(OverscrollSource::Touch),
            "a touch screen reports its lift even in a browser"
        );
        assert!(web.contains(OverscrollSource::Mouse));
        assert!(web.contains(OverscrollSource::Keyboard));
    }

    #[test]
    fn adding_and_removing_a_source_are_idempotent() {
        let touch = OverscrollSources::NONE.with(OverscrollSource::Touch);

        assert_eq!(touch.with(OverscrollSource::Touch), touch);
        assert_eq!(
            touch.without(OverscrollSource::Wheel),
            touch,
            "removing an absent source changes nothing"
        );
        assert_eq!(
            touch.without(OverscrollSource::Touch),
            OverscrollSources::NONE
        );
    }

    #[test]
    fn sets_combine_through_the_bitwise_operators() {
        let pair = OverscrollSource::Touch | OverscrollSource::Mouse;

        assert_eq!(
            pair,
            OverscrollSources::NONE
                .with(OverscrollSource::Touch)
                .with(OverscrollSource::Mouse)
        );
        assert_eq!(
            pair | OverscrollSource::Wheel,
            pair.with(OverscrollSource::Wheel)
        );
        assert_eq!(
            pair | OverscrollSources::ALL,
            OverscrollSources::ALL,
            "a union with everything is everything"
        );
        assert_eq!(
            pair.intersection(OverscrollSources::WEB_DEFAULT),
            pair,
            "neither touch nor mouse is clamped on the web"
        );

        let mut growing = OverscrollSources::NONE;
        growing |= OverscrollSource::Keyboard;
        assert_eq!(growing, OverscrollSources::from(OverscrollSource::Keyboard));
    }

    #[test]
    fn unknown_bits_are_dropped_when_wrapping_a_raw_pattern() {
        assert_eq!(
            OverscrollSources::from_bits_truncate(u8::MAX),
            OverscrollSources::ALL
        );
        assert_eq!(
            OverscrollSources::from_bits_truncate(0b1111_0000),
            OverscrollSources::NONE
        );
    }
}
