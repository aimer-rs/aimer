//! Where an indicator sits.

use std::fmt;
use std::sync::Arc;

use aimer_attribute::ResolvedSize;

/// Maps a fractional slot position to a place on screen.
///
/// An indicator is animated as one scalar, its *position*: `0.0` is the first
/// slot, `1.0` the second, and `0.5` halfway between them. A track turns that
/// scalar into the top-left corner of the indicator, so the same motion and
/// style work whether the slots run along a row, down a column, or around
/// something else.
///
/// Slots are a fixed size so that the indicator's layers can be built once and
/// then only moved: sizing them from the laid-out content would mean building
/// them again whenever the content changed size.
///
/// A track must be `Send + Sync + 'static` so that holders can share it freely.
pub trait IndicatorTrack: Send + Sync + 'static {
    /// How many slots the track has.
    fn slot_count(&self) -> usize;

    /// The size of one slot, in logical pixels.
    fn slot_size(&self) -> ResolvedSize;

    /// The top-left corner of the slot at `position`, in logical pixels from
    /// the track's own origin.
    ///
    /// `position` is not limited to `0.0..=slot_count - 1`: a spring overshoots
    /// its target, and the indicator should follow it past the last slot rather
    /// than stop dead. Positions outside the track should extrapolate.
    fn origin(&self, position: f32) -> (f32, f32);

    /// The position whose slot is nearest to `point`, given in logical pixels
    /// from the track's own origin: the inverse of [`origin`](Self::origin) for
    /// the center of a slot, so that whole numbers are slot centers.
    ///
    /// Like `origin`, the result is not limited to the track: a point past the
    /// last slot gives a position past it, and the caller clamps.
    fn nearest(&self, point: (f32, f32)) -> f32;

    /// Where an indicator at `position` ends up when it is dragged by `delta`
    /// logical pixels, as `(x, y)`.
    ///
    /// This is what lets a drag follow the pointer on any track: a row reads
    /// the horizontal movement, a column the vertical one, a curved track could
    /// project onto its path. The result is not limited to the track; the
    /// caller clamps it to the slots it wants reachable.
    fn shifted(&self, position: f32, delta: (f32, f32)) -> f32;

    /// Converts this track into the handle holders store.
    ///
    /// The default boxes the track. [`Linear`] overrides it to avoid the
    /// allocation.
    fn into_any(self) -> AnyIndicatorTrack
    where
        Self: Sized,
    {
        AnyIndicatorTrack::custom(self)
    }
}

/// The direction a [`Linear`] track runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// Evenly spaced slots along a row or a column.
///
/// Build one with [`tracks::row`] or [`tracks::column`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Linear {
    axis: Axis,
    count: usize,
    slot: ResolvedSize,
    gap: f32,
    inset: (f32, f32),
}

impl Linear {
    /// Sets the space between neighbouring slots. Negative or non-finite
    /// values are treated as no gap.
    #[inline]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = if gap >= 0.0 && gap.is_finite() { gap } else { 0.0 };
        self
    }

    /// Offsets the whole track from the origin of the widget it sits in, for a
    /// bar whose content is padded. Non-finite values are treated as zero.
    #[inline]
    pub const fn inset(mut self, x: f32, y: f32) -> Self {
        self.inset = (
            if x.is_finite() { x } else { 0.0 },
            if y.is_finite() { y } else { 0.0 },
        );
        self
    }

    /// The distance between the starts of two neighbouring slots.
    #[inline]
    fn pitch(&self) -> f32 {
        match self.axis {
            Axis::Horizontal => self.slot.width + self.gap,
            Axis::Vertical => self.slot.height + self.gap,
        }
    }
}

impl IndicatorTrack for Linear {
    #[inline]
    fn slot_count(&self) -> usize {
        self.count
    }

    #[inline]
    fn slot_size(&self) -> ResolvedSize {
        self.slot
    }

    #[inline]
    fn origin(&self, position: f32) -> (f32, f32) {
        let along = if position.is_finite() { position * self.pitch() } else { 0.0 };
        match self.axis {
            Axis::Horizontal => (self.inset.0 + along, self.inset.1),
            Axis::Vertical => (self.inset.0, self.inset.1 + along),
        }
    }

    #[inline]
    fn nearest(&self, point: (f32, f32)) -> f32 {
        let (along, inset, slot) = match self.axis {
            Axis::Horizontal => (point.0, self.inset.0, self.slot.width),
            Axis::Vertical => (point.1, self.inset.1, self.slot.height),
        };
        let pitch = self.pitch();
        if along.is_finite() && pitch > 0.0 { (along - inset - slot * 0.5) / pitch } else { 0.0 }
    }

    #[inline]
    fn shifted(&self, position: f32, delta: (f32, f32)) -> f32 {
        if !position.is_finite() {
            return 0.0;
        }
        let along = match self.axis {
            Axis::Horizontal => delta.0,
            Axis::Vertical => delta.1,
        };
        let pitch = self.pitch();
        if along.is_finite() && pitch > 0.0 { position + along / pitch } else { position }
    }

    #[inline]
    fn into_any(self) -> AnyIndicatorTrack {
        AnyIndicatorTrack(Repr::Linear(self))
    }
}

/// The standard tracks.
pub mod tracks {
    use aimer_attribute::ResolvedSize;

    use super::{Axis, Linear};

    /// `count` slots of `slot_width` by `slot_height`, left to right.
    ///
    /// ```
    /// use aimer_attribute::ResolvedSize;
    /// use aimer_indicator::{IndicatorTrack, tracks};
    ///
    /// let track = tracks::row(3, 100.0, 56.0).gap(4.0);
    /// assert_eq!(track.origin(1.0), (104.0, 0.0));
    /// assert_eq!(track.slot_size(), ResolvedSize { width: 100.0, height: 56.0 });
    /// ```
    pub const fn row(count: usize, slot_width: f32, slot_height: f32) -> Linear {
        linear(Axis::Horizontal, count, slot_width, slot_height)
    }

    /// `count` slots of `slot_width` by `slot_height`, top to bottom.
    pub const fn column(count: usize, slot_width: f32, slot_height: f32) -> Linear {
        linear(Axis::Vertical, count, slot_width, slot_height)
    }

    const fn linear(axis: Axis, count: usize, width: f32, height: f32) -> Linear {
        Linear {
            axis,
            count,
            slot: ResolvedSize {
                width: if width >= 0.0 && width.is_finite() { width } else { 0.0 },
                height: if height >= 0.0 && height.is_finite() { height } else { 0.0 },
            },
            gap: 0.0,
            inset: (0.0, 0.0),
        }
    }
}

/// A track of any kind, cheap to clone.
///
/// This is what the framework stores wherever it holds a track. [`Linear`] is
/// held inline, so no allocation or virtual call is needed to find the
/// indicator each frame; only a custom track is boxed. `AnyIndicatorTrack` is
/// itself an [`IndicatorTrack`].
#[derive(Clone)]
pub struct AnyIndicatorTrack(Repr);

#[derive(Clone)]
enum Repr {
    Linear(Linear),
    Custom(Arc<dyn IndicatorTrack>),
}

impl AnyIndicatorTrack {
    /// Wraps `track`. Equivalent to [`IndicatorTrack::into_any`].
    pub fn new(track: impl IndicatorTrack) -> Self {
        track.into_any()
    }

    fn custom(track: impl IndicatorTrack) -> Self {
        Self(Repr::Custom(Arc::new(track)))
    }

    /// Whether the track is held without an allocation.
    #[cfg(test)]
    fn is_inline(&self) -> bool {
        matches!(self.0, Repr::Linear(_))
    }
}

impl Default for AnyIndicatorTrack {
    /// A single 48 by 32 slot.
    fn default() -> Self {
        Self(Repr::Linear(tracks::row(1, 48.0, 32.0)))
    }
}

impl IndicatorTrack for AnyIndicatorTrack {
    #[inline]
    fn slot_count(&self) -> usize {
        match &self.0 {
            Repr::Linear(track) => track.slot_count(),
            Repr::Custom(track) => track.slot_count(),
        }
    }

    #[inline]
    fn slot_size(&self) -> ResolvedSize {
        match &self.0 {
            Repr::Linear(track) => track.slot_size(),
            Repr::Custom(track) => track.slot_size(),
        }
    }

    #[inline]
    fn origin(&self, position: f32) -> (f32, f32) {
        match &self.0 {
            Repr::Linear(track) => track.origin(position),
            Repr::Custom(track) => track.origin(position),
        }
    }

    #[inline]
    fn nearest(&self, point: (f32, f32)) -> f32 {
        match &self.0 {
            Repr::Linear(track) => track.nearest(point),
            Repr::Custom(track) => track.nearest(point),
        }
    }

    #[inline]
    fn shifted(&self, position: f32, delta: (f32, f32)) -> f32 {
        match &self.0 {
            Repr::Linear(track) => track.shifted(position, delta),
            Repr::Custom(track) => track.shifted(position, delta),
        }
    }

    fn into_any(self) -> AnyIndicatorTrack {
        self
    }
}

impl PartialEq for AnyIndicatorTrack {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Repr::Linear(a), Repr::Linear(b)) => a == b,
            (Repr::Custom(a), Repr::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for AnyIndicatorTrack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Repr::Linear(track) => write!(f, "{track:?}"),
            Repr::Custom(_) => f.write_str("Custom"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_advances_by_the_slot_width_and_the_gap() {
        let track = tracks::row(3, 100.0, 56.0).gap(4.0);
        assert_eq!(track.origin(0.0), (0.0, 0.0));
        assert_eq!(track.origin(1.0), (104.0, 0.0));
        assert_eq!(track.origin(2.0), (208.0, 0.0));
        assert_eq!(track.origin(0.5), (52.0, 0.0));
    }

    #[test]
    fn a_column_advances_by_the_slot_height_and_the_gap() {
        let track = tracks::column(3, 100.0, 56.0).gap(4.0);
        assert_eq!(track.origin(2.0), (0.0, 120.0));
    }

    #[test]
    fn the_inset_shifts_every_slot() {
        let track = tracks::row(2, 80.0, 40.0).inset(6.0, 3.0);
        assert_eq!(track.origin(0.0), (6.0, 3.0));
        assert_eq!(track.origin(1.0), (86.0, 3.0));
    }

    #[test]
    fn positions_outside_the_track_extrapolate_so_an_overshoot_is_followed() {
        let track = tracks::row(3, 100.0, 56.0);
        let (before_first, _) = track.origin(-0.1);
        let (past_last, _) = track.origin(2.1);
        assert!((before_first + 10.0).abs() < 1.0e-3, "{before_first}");
        assert!((past_last - 210.0).abs() < 1.0e-3, "{past_last}");
    }

    #[test]
    fn unusable_numbers_fall_back_instead_of_poisoning_layout() {
        let track = tracks::row(2, f32::NAN, -5.0).gap(f32::INFINITY).inset(f32::NAN, 2.0);
        assert_eq!(track.slot_size(), ResolvedSize { width: 0.0, height: 0.0 });
        assert_eq!(track.origin(1.0), (0.0, 2.0));
        assert_eq!(tracks::row(2, 10.0, 10.0).origin(f32::NAN), (0.0, 0.0));
        assert_eq!(tracks::row(2, 10.0, 10.0).gap(-3.0).origin(1.0), (10.0, 0.0));
    }

    #[test]
    fn handles_are_inline_for_linear_tracks_and_equal_by_value() {
        let any = tracks::row(3, 10.0, 10.0).into_any();
        assert!(any.is_inline());
        assert_eq!(any, tracks::row(3, 10.0, 10.0).into_any());
        assert_ne!(any, tracks::column(3, 10.0, 10.0).into_any());
        assert_eq!(any.slot_count(), 3);
        assert_eq!(any.origin(1.0), (10.0, 0.0));
    }

    #[test]
    fn dragging_a_row_follows_the_horizontal_movement_by_the_slot_pitch() {
        let track = tracks::row(3, 100.0, 56.0).gap(4.0);
        assert_close(track.shifted(0.0, (104.0, 0.0)), 1.0);
        assert_close(track.shifted(1.0, (52.0, 0.0)), 1.5);
        assert_close(track.shifted(2.0, (-208.0, 0.0)), 0.0);
        // The other axis is ignored.
        assert_close(track.shifted(1.0, (0.0, 500.0)), 1.0);
    }

    #[test]
    fn dragging_a_column_follows_the_vertical_movement() {
        let track = tracks::column(3, 100.0, 56.0).gap(4.0);
        assert_close(track.shifted(0.0, (500.0, 60.0)), 1.0);
        assert_close(track.shifted(2.0, (0.0, -30.0)), 2.0 - 0.5);
    }

    #[test]
    fn dragging_survives_a_track_with_no_extent_and_bad_numbers() {
        assert_eq!(tracks::row(3, 0.0, 10.0).shifted(1.0, (50.0, 0.0)), 1.0);
        let track = tracks::row(3, 100.0, 56.0);
        assert_eq!(track.shifted(1.0, (f32::NAN, 0.0)), 1.0);
        assert_eq!(track.shifted(1.0, (f32::INFINITY, 0.0)), 1.0);
        // A position that is not a number is the first slot.
        assert_eq!(track.shifted(f32::NAN, (10.0, 0.0)), 0.0);
    }

    #[test]
    fn the_nearest_position_of_a_point_inverts_where_a_slot_sits() {
        let track = tracks::row(3, 100.0, 56.0).gap(4.0).inset(6.0, 3.0);
        for index in 0..3 {
            let (x, y) = track.origin(index as f32);
            // The center of the slot is exactly that slot.
            assert_close(track.nearest((x + 50.0, y + 28.0)), index as f32);
            // And a point a little off the center is still nearest to it.
            assert_close(track.nearest((x + 50.0 + 20.0, y)).round(), index as f32);
        }
        // Past either end the position leaves the track, for the caller to clamp.
        assert!(track.nearest((-500.0, 0.0)) < 0.0);
        assert!(track.nearest((900.0, 0.0)) > 2.0);
        // A column reads the vertical coordinate.
        let column = tracks::column(3, 100.0, 56.0);
        assert_close(column.nearest((9999.0, 56.0 + 28.0)), 1.0);
    }

    #[test]
    fn the_nearest_position_survives_bad_numbers() {
        assert_eq!(tracks::row(3, 100.0, 56.0).nearest((f32::NAN, 0.0)), 0.0);
        assert_eq!(tracks::row(3, 0.0, 56.0).nearest((50.0, 0.0)), 0.0);
    }

    #[test]
    fn a_handle_forwards_dragging_to_its_track() {
        let any = tracks::row(3, 10.0, 10.0).into_any();
        assert_close(any.shifted(0.0, (20.0, 0.0)), 2.0);
        assert_close(Diagonal.into_any().shifted(1.0, (10.0, 10.0)), 2.0);
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-4, "{actual} is not {expected}");
    }

    /// Slots along a diagonal, as a custom track.
    struct Diagonal;

    impl IndicatorTrack for Diagonal {
        fn slot_count(&self) -> usize {
            4
        }

        fn slot_size(&self) -> ResolvedSize {
            ResolvedSize { width: 8.0, height: 8.0 }
        }

        fn origin(&self, position: f32) -> (f32, f32) {
            (position * 10.0, position * 10.0)
        }

        fn nearest(&self, point: (f32, f32)) -> f32 {
            (point.0 + point.1) / 20.0
        }

        fn shifted(&self, position: f32, delta: (f32, f32)) -> f32 {
            position + (delta.0 + delta.1) / 20.0
        }
    }

    #[test]
    fn a_custom_track_is_boxed_and_compared_by_identity() {
        let custom = Diagonal.into_any();
        assert!(!custom.is_inline());
        assert_eq!(custom.origin(2.0), (20.0, 20.0));
        assert_eq!(custom, custom.clone());
        assert_ne!(custom, Diagonal.into_any());
    }
}
