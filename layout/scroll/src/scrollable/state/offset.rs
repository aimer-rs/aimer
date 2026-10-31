//! Applying input deltas to the offset: clamping, wheel and precise-device
//! steps, and the visual (rubber-banded) offset.

use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::*;
use crate::scrollable::constants::*;

impl ScrollState {
    /// Clamp the scroll offset within the allowed range.
    /// scroll_offset is negative (content moves up), so min_scroll <= offset <=
    /// 0 typically.
    pub(crate) fn clamp_offset(&self, mut offset: Vec2d) -> Vec2d {
        let min = self.cached_min_scroll.get();
        let max = self.cached_max_scroll.get();
        offset.x = offset.x.max(-max.x).min(-min.x);
        offset.y = offset.y.max(-max.y).min(-min.y);
        offset
    }

    /// Apply a wheel/trackpad scroll delta to `offset` and return the new
    /// offset.
    ///
    /// Non-bouncy scrollables clamp to the valid range immediately (matching
    /// the pointer-drag and keyboard paths). Bouncy ones keep the raw
    /// (possibly out-of-range) offset and let [`visual_offset`] +
    /// [`update_momentum`] apply the rubber-band and spring back.
    ///
    /// This replaces an earlier attempt that zeroed the delta up-front when
    /// `offset` was already at `clamp_offset(offset)`. Because an in-range
    /// offset always equals its own clamp, that predicate matched on *every*
    /// frame and silently discarded all wheel/trackpad deltas — the scrollable
    /// could not be scrolled by mouse wheel or trackpad at all (drag/keyboard
    /// still worked because they clamp *after* applying the delta).
    pub(crate) fn apply_wheel_delta(&self, offset: Vec2d, scroll_delta: Vec2d) -> Vec2d {
        let mut next = Vec2d {
            x: offset.x + scroll_delta.x,
            y: offset.y + scroll_delta.y,
        };
        if !self.bouncy() {
            next = self.clamp_offset(next);
        }
        next
    }

    /// Apply one discrete mouse-wheel detent without starting inertial motion.
    ///
    /// A line wheel has no native gesture velocity to preserve. Each frame step
    /// therefore updates the offset once and clears any previous glide; bouncy
    /// edge recovery remains draw-driven.
    pub(crate) fn apply_line_wheel_delta(&self, scroll_delta: Vec2d) -> bool {
        self.commit_device_scroll_delta(scroll_delta, AnimInstant::now())
    }

    /// Apply one pixel-precise device delta without deriving a second glide.
    ///
    /// Quiver already frame-synchronizes trackpad input, while operating
    /// systems and browsers provide the gesture's own deceleration samples.
    /// Keeping this layer distance-only prevents double integration and leaves
    /// touch/mouse drag release flings unchanged.
    pub(crate) fn apply_precise_scroll_delta(&self, scroll_delta: Vec2d) -> bool {
        self.apply_precise_scroll_delta_at(scroll_delta, AnimInstant::now())
    }

    /// [`apply_precise_scroll_delta`](Self::apply_precise_scroll_delta) at an
    /// explicit instant, so episode expiry is testable without sleeping.
    pub(crate) fn apply_precise_scroll_delta_at(
        &self,
        scroll_delta: Vec2d,
        now: AnimInstant,
    ) -> bool {
        self.commit_device_scroll_delta(scroll_delta, now)
    }

    /// Store one device delta unless it is a weaker outward re-stretch of an
    /// overscroll episode that is already recovering.
    ///
    /// A rejected delta still proves the gesture's tail is running, so it keeps
    /// the high-water mark alive; only a genuine silence lets the mark expire.
    pub(super) fn commit_device_scroll_delta(&self, scroll_delta: Vec2d, now: AnimInstant) -> bool {
        let current = self.scroll_offset.get();
        let next = self.apply_wheel_delta(current, scroll_delta);
        if !self.accepts_overscroll_step_at(current, next, now) {
            self.keep_overscroll_peak_alive(now);
            return false;
        }
        self.set_scroll_offset(next);
        self.record_overscroll_peak_at(next, now);
        self.stop_device_scroll_momentum();
        true
    }

    pub(super) fn stop_device_scroll_momentum(&self) {
        self.pointer_velocity.set(Vec2d::ZERO);
        self.fling.spring_velocity.set(Vec2d::ZERO);
        self.fling.momentum_start.set(None);
        self.clear_velocity_history();
        self.cancel_fling();
    }

    pub(crate) fn visual_offset(&self, offset: Vec2d) -> Vec2d {
        let offset = Self::axis_only(self.axis, offset);
        let min = self.cached_min_scroll.get();
        let max = self.cached_max_scroll.get();

        let min_x = -min.x;
        let max_x = -max.x;
        let min_y = -min.y;
        let max_y = -max.y;

        if self.bouncy() {
            let (vp_w, vp_h) = self.cached_viewport.get();
            let resistance = self.scroll_behavior.bouncy_resistance;
            let vx = Self::apply_bouncy(offset.x, max_x, min_x, vp_w.max(MIN_VIEWPORT), resistance);
            let vy = Self::apply_bouncy(offset.y, max_y, min_y, vp_h.max(MIN_VIEWPORT), resistance);

            (vx, vy).into()
        } else {
            (offset.x.clamp(max_x, min_x), offset.y.clamp(max_y, min_y)).into()
        }
    }

}
