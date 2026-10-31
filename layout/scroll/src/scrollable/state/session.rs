//! The scroll session latch and the app-facing scroll callbacks.

use std::cell::{Cell, RefCell};

use aimer_attribute::position::Vec2d;
use aimer_utils::callback::Callback;

use super::controller::*;
use crate::scrollable::ScrollAxis;

/// The scroll session latch, its debug counters and the app callbacks.
#[derive(Default)]
pub struct SessionState {
    /// Whether a scroll session is currently in progress — a user drag, a
    /// wheel/ keyboard scroll, release momentum, a spring-back, or a
    /// programmatic animation. Latches the `on_scroll_start` /
    /// `on_scroll_end` callbacks so each fires exactly once per session (on
    /// the idle↔scrolling edge), never once per frame.
    pub(crate) active: Cell<bool>,
    /// Debug-only counters for one scroll session. They live on the shared
    /// state because input and draw paths both contribute to the same gesture.
    #[cfg(debug_assertions)]
    pub(crate) input_events: Cell<u64>,
    #[cfg(debug_assertions)]
    pub(crate) draw_frames: Cell<u64>,
    #[cfg(debug_assertions)]
    pub(crate) moved_frames: Cell<u64>,
    /// Fired once when a scroll session begins (idle → scrolling); receives the
    /// current logical offset. Shared in from the [`ScrollController`] on
    /// attach.
    pub(crate) on_start: RefCell<Callback<Vec2d>>,
    /// Fired once when a scroll session ends (scrolling → idle, after momentum
    /// and any spring-back have fully settled); receives the resting offset.
    pub(crate) on_end: RefCell<Callback<Vec2d>>,
    /// Fired on **every** frame where the logical offset actually moved (user
    /// drag, wheel/keyboard, momentum/fling, spring-back, or a programmatic
    /// `jump_to`/`animate_to`) — the level-triggered counterpart to the
    /// edge-triggered start/end pair. Receives the live logical offset. Shared
    /// in from the [`ScrollController`] on attach.
    pub(crate) on_scroll: RefCell<Callback<Vec2d>>,
    /// Last logical offset reported through [`on_scroll`](SessionState::on_scroll),
    /// used to fire only on genuine movement. `None` until the first frame
    /// establishes the baseline (so the initial render never fires
    /// `on_scroll`).
    pub(crate) last_reported: Cell<Option<Vec2d>>,
}

impl SessionState {
    /// Carries the session latch across a rebuild so a scroll in flight
    /// neither re-fires `on_scroll_start` nor loses its `on_scroll_end`.
    pub(crate) fn adopt_from(&self, prev: &Self, axis: ScrollAxis) {
        self.active.set(prev.active.get());
        #[cfg(debug_assertions)]
        {
            self.input_events.set(prev.input_events.get());
            self.draw_frames.set(prev.draw_frames.get());
            self.moved_frames.set(prev.moved_frames.get());
        }
        self.last_reported.set(
            prev.last_reported
                .get()
                .map(|offset| ScrollState::axis_only(axis, offset)),
        );
    }
}

impl ScrollState {
    /// Enter the "scrolling" state, firing
    /// [`on_scroll_start`](SessionState::on_start) once on the idle →
    /// scrolling edge. A no-op if a session is already active,
    /// so it is safe to call on every drag move / wheel tick.
    pub(crate) fn begin_scroll(&self) {
        if !self.session.active.replace(true) {
            #[cfg(debug_assertions)]
            {
                self.session.input_events.set(0);
                self.session.draw_frames.set(0);
                self.session.moved_frames.set(0);
            }
            // Clone the handle out (cheap `Rc` bump) before invoking so a callback
            // that touches the controller can't re-enter a live `RefCell` borrow.
            let cb = self.session.on_start.borrow().clone();
            cb.call(self.logical_offset());
        }
    }

    /// Leave the "scrolling" state, firing
    /// [`on_scroll_end`](SessionState::on_end) once on the scrolling → idle
    /// edge. A no-op when already idle, so the draw loop can call it every
    /// settled frame without emitting duplicates.
    pub(crate) fn end_scroll(&self) {
        if self.session.active.replace(false) {
            let cb = self.session.on_end.borrow().clone();
            cb.call(self.logical_offset());
        }
    }

    /// Records one accepted input event in the active debug scroll session.
    #[inline]
    pub(crate) fn record_input_event(&self) {
        #[cfg(debug_assertions)]
        self.session.input_events
            .set(self.session.input_events.get().saturating_add(1));
    }

    /// Records one frame requested by the active debug scroll session.
    #[inline]
    pub(crate) fn record_draw_frame(&self, active: bool, moved: bool) {
        #[cfg(debug_assertions)]
        if active {
            self.session.draw_frames
                .set(self.session.draw_frames.get().saturating_add(1));
            if moved {
                self.session.moved_frames
                    .set(self.session.moved_frames.get().saturating_add(1));
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = (active, moved);
    }

    /// Fire [`on_scroll`](SessionState::on_scroll) with the current logical offset, but
    /// only when it has actually moved (beyond [`SCROLL_NOTIFY_EPSILON`]) since
    /// the last report. Level-triggered: call it once per drawn frame after the
    /// offset is finalized; the epsilon guard collapses idle frames and
    /// sub-pixel jitter to no-ops. The very first call only establishes the
    /// baseline, so the initial render never emits a spurious update.
    pub(crate) fn notify_scroll(&self) -> bool {
        let current = self.logical_offset();
        let moved = match self.session.last_reported.get() {
            Some(prev) => {
                (current.x - prev.x).abs() > SCROLL_NOTIFY_EPSILON
                    || (current.y - prev.y).abs() > SCROLL_NOTIFY_EPSILON
            }
            None => false,
        };
        self.session.last_reported.set(Some(current));
        if moved {
            // Clone out (cheap `Rc` bump) so a callback that touches the
            // controller can't re-enter a live `RefCell` borrow.
            let cb = self.session.on_scroll.borrow().clone();
            cb.call(current);
        }
        moved
    }
}
