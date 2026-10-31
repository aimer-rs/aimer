//! Drag velocity sampling that feeds the release fling.

use std::cell::{Cell, RefCell};

use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::*;
use crate::scrollable::velocity_history::VelocityHistory;
use crate::scrollable::constants::*;
use crate::scrollable::ScrollAxis;

/// Drag velocity sampling state.
#[derive(Default)]
pub struct DragVelocityState {
    /// Finger delta accumulated since the last emitted drag-velocity sample.
    /// Coalesced same-frame pointer moves (web) fold in here instead of each
    /// producing its own inflated sample; flushed once a real time slice
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) has elapsed.
    pub(crate) accum: Cell<Vec2d>,
    /// Wall-clock instant of the last emitted drag-velocity sample, or `None`
    /// before the first sample of a gesture.
    pub(crate) sample_time: Cell<Option<AnimInstant>>,
    /// Allocated on the first drag sample. Keeping the fixed-size ring inline
    /// here made every idle scrollable carry its full sample buffer.
    pub(crate) history: RefCell<Option<Box<VelocityHistory>>>,
}

impl DragVelocityState {
    /// Carries the sampling window across a rebuild.
    pub(crate) fn adopt_from(&self, prev: &Self, axis: ScrollAxis) {
        self.accum
            .set(ScrollState::axis_only(axis, prev.accum.get()));
        self.sample_time.set(prev.sample_time.get());
        *self.history.borrow_mut() = prev
            .history
            .borrow()
            .as_ref()
            .map(|history| Box::new((**history).clone()));
    }
}

impl ScrollState {
    /// Record `velocity` as the speed the finger held for the `dt` seconds
    /// that ended at `at`.
    pub(crate) fn push_velocity(&self, velocity: Vec2d, dt: f32, at: AnimInstant) {
        self.drag_velocity.history
            .borrow_mut()
            .get_or_insert_with(|| Box::new(VelocityHistory::new()))
            .push(velocity, dt, at);
    }

    /// Return the speed the finger is moving at as of `now`, averaged over the
    /// samples that still lie inside [`VELOCITY_HORIZON_S`].
    ///
    /// This is the speed a release fling is launched with, so it deliberately
    /// describes the *end* of the gesture: a drag whose last recorded motion
    /// has fallen out of the horizon reports zero and flings nothing.
    pub(crate) fn smoothed_velocity(&self, now: AnimInstant) -> Vec2d {
        self.drag_velocity.history
            .borrow()
            .as_ref()
            .map_or(Vec2d::ZERO, |history| history.velocity_at(now))
    }

    /// Clear the velocity history (e.g. on pointer-down).
    pub(crate) fn clear_velocity_history(&self) {
        if let Some(history) = self.drag_velocity.history.borrow_mut().as_mut() {
            history.clear();
        }
    }

    /// Fold a raw drag delta (already scaled by `speed_multiplier`) into the
    /// velocity accumulator and, once a real time slice
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) has elapsed since the last sample, emit
    /// the averaged drag velocity (px per 120 Hz-frame, both axes) together
    /// with that slice's `dt`. Returns `None` while the delta is still
    /// being accumulated within the current slice.
    ///
    /// This merges the burst of *coalesced* same-`Instant` pointer moves that
    /// web delivers per native `pointermove` into one realistic velocity
    /// sample, instead of letting each tiny sub-delta / ~0 dt inflate the
    /// release fling (~3x too fast on touch). Native, delivering one move
    /// per frame, emits on every call. The offset is still updated
    /// per-event by the caller, so dragging stays 1:1.
    pub(crate) fn accumulate_drag_velocity(
        &self,
        dx: f32,
        dy: f32,
        now: AnimInstant,
    ) -> Option<(Vec2d, f32)> {
        let mut accum = self.drag_velocity.accum.get();
        accum.x += dx;
        accum.y += dy;

        let sample_dt = self
            .drag_velocity.sample_time
            .get()
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(FRAME_REF_120);

        if sample_dt >= VELOCITY_SAMPLE_MIN_DT {
            self.drag_velocity.accum.set(Vec2d { x: 0.0, y: 0.0 });
            self.drag_velocity.sample_time.set(Some(now));
            let velocity = Vec2d {
                x: (accum.x / sample_dt) * FRAME_REF_120,
                y: (accum.y / sample_dt) * FRAME_REF_120,
            };
            Some((velocity, sample_dt))
        } else {
            self.drag_velocity.accum.set(accum);
            None
        }
    }

    /// Close the open sampling slice at `now` and return the velocity it
    /// measured, or `None` while the slice is too short
    /// ([`VELOCITY_SAMPLE_MIN_DT`]) to mean anything.
    ///
    /// A gesture ends between samples, and everything that happened since the
    /// last one is still sitting in the accumulator: the tail of a coalesced
    /// burst, or — the case that matters most — no motion at all, because the
    /// finger came to rest on the glass and the platform stopped reporting
    /// moves. Both describe the release far better than the last emitted
    /// sample does, and a resting finger yields exactly the zero velocity that
    /// keeps a deliberate stop from turning into a fling.
    ///
    /// Unlike [`Self::accumulate_drag_velocity`] this always closes the slice,
    /// since no further move will arrive to close it.
    pub(crate) fn flush_drag_velocity(&self, now: AnimInstant) -> Option<(Vec2d, f32)> {
        let sample_time = self.drag_velocity.sample_time.get()?;
        let sample_dt = now.duration_since(sample_time).as_secs_f32();
        if sample_dt < VELOCITY_SAMPLE_MIN_DT {
            return None;
        }

        let accum = self.drag_velocity.accum.get();
        self.drag_velocity.accum.set(Vec2d { x: 0.0, y: 0.0 });
        self.drag_velocity.sample_time.set(Some(now));
        let velocity = Vec2d {
            x: (accum.x / sample_dt) * FRAME_REF_120,
            y: (accum.y / sample_dt) * FRAME_REF_120,
        };
        Some((velocity, sample_dt))
    }

}
