//! The end anchor of a reverse [`Scrollable`](crate::Scrollable).
//!
//! A reverse engine keeps the same internal offset convention as a forward one
//! (`-(distance from the start)`, scaled), so momentum, overscroll, the scroll
//! bar and keyboard handling are untouched. What changes is what it *preserves*
//! when the range changes, and how positions are presented to the app: as a
//! distance from the content end.
//!
//! Before the first layout the range is unknown (`cached_max_scroll` is zero),
//! so a requested distance `d` is held as the offset `+d` — exactly
//! `-(max - d)` for `max == 0`. The first range change then subtracts the real
//! range and lands on `d - max` without any separate pending state.

use aimer_attribute::position::Vec2d;

use super::controller::*;
use crate::ScrollAxis;

impl ScrollState {
    /// Moves a reverse engine's offset by however much the scroll range changed,
    /// so the distance from the content end — the thing it keeps — stays put.
    ///
    /// The offset is `-(max - distance_from_end)`, so a range that grew by `d`
    /// shifts it by `-d`. Taking the *difference* rather than recomputing from
    /// a stored distance means an unchanged range returns the offset bit for
    /// bit (no per-frame float drift, no spurious "offset moved" frame), and an
    /// offset that is out of range mid-overscroll keeps its overshoot.
    ///
    /// Does nothing for a forward engine. Returns whether the offset moved.
    pub(crate) fn follow_range_change(&self, previous_max: Vec2d, max: Vec2d) -> bool {
        if !self.reverse {
            return false;
        }
        let mut offset = self.scroll_offset.get();
        match self.axis {
            ScrollAxis::Vertical => offset.y -= max.y - previous_max.y,
            ScrollAxis::Horizontal => offset.x -= max.x - previous_max.x,
        }
        if offset == self.scroll_offset.get() {
            return false;
        }
        self.set_scroll_offset(offset);
        true
    }

    /// How far a reverse engine shifts content that is shorter than the
    /// viewport toward the end, so it hugs the end instead of the start.
    #[inline]
    pub(super) fn end_inset(&self) -> Vec2d {
        if !self.reverse {
            return Vec2d::ZERO;
        }
        let (viewport_w, viewport_h) = self.cached_viewport.get();
        let content = self.cached_content_size.get();
        match self.axis {
            ScrollAxis::Vertical => Vec2d {
                x: 0.0,
                y: (viewport_h - content.height).max(0.0),
            },
            ScrollAxis::Horizontal => Vec2d {
                x: (viewport_w - content.width).max(0.0),
                y: 0.0,
            },
        }
    }


    /// Converts an app position in logical pixels into an internal offset.
    ///
    /// A forward engine counts from the start (`-position * scale`); a reverse
    /// one counts from the end, so `position` is the distance from it and the
    /// offset is `distance - max`. Before the first layout `max` is zero, which
    /// yields the seed form described in the module docs.
    pub(crate) fn internal_for_position(&self, position: Vec2d) -> Vec2d {
        let scale = self.last_scale.get().max(f32::EPSILON);
        if !self.reverse {
            return Self::axis_only(
                self.axis,
                Vec2d {
                    x: -position.x * scale,
                    y: -position.y * scale,
                },
            );
        }
        let max = self.cached_max_scroll.get();
        Self::axis_only(
            self.axis,
            Vec2d {
                x: position.x * scale - max.x,
                y: position.y * scale - max.y,
            },
        )
    }

    /// Whether a programmatic position must wait for the first layout because
    /// clamping it against the still-empty range would discard it.
    #[inline]
    pub(crate) fn awaiting_first_range(&self) -> bool {
        self.reverse && !self.cached_content_size_valid.get()
    }

    /// An app position in logical pixels from an internal `offset`: measured
    /// from the content start for a forward engine, from the end for a reverse
    /// one. See [`ScrollController`](crate::ScrollController).
    pub(crate) fn logical_position(&self, offset: Vec2d) -> Vec2d {
        let scale = self.last_scale.get().max(f32::EPSILON);
        let o = Self::axis_only(self.axis, offset);
        if !self.reverse {
            return Vec2d {
                x: -o.x / scale,
                y: -o.y / scale,
            };
        }
        let max = self.cached_max_scroll.get();
        Self::axis_only(
            self.axis,
            Vec2d {
                x: (o.x + max.x) / scale,
                y: (o.y + max.y) / scale,
            },
        )
    }

    /// The offset to write to teardown storage, in logical pixels.
    ///
    /// A forward engine stores its internal offset as it always has. A reverse
    /// one stores its distance from the end, which stays meaningful when
    /// content is added before the viewport while the scrollable is gone.
    pub(crate) fn persisted_offset(&self, offset: Vec2d) -> Vec2d {
        if self.reverse {
            return self.logical_position(offset);
        }
        let scale = self.last_scale.get().max(f32::EPSILON);
        Vec2d {
            x: offset.x / scale,
            y: offset.y / scale,
        }
    }
}
