//! Release fling, programmatic animation and the bezier easing behind them.

use std::cell::{Cell, RefCell};

use aimer_animation::{AnyCurve, Curve};
use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::*;
use crate::scrollable::constants::*;
use crate::scrollable::ScrollAxis;

#[inline]
fn bezier_axis(s: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - s;
    3.0 * mt * mt * s * p1 + 3.0 * mt * s * s * p2 + s * s * s
}

/// Derivative (w.r.t. the parameter `s`) of [`bezier_axis`].
#[inline]
fn bezier_axis_deriv(s: f32, p1: f32, p2: f32) -> f32 {
    let mt = 1.0 - s;
    3.0 * mt * mt * p1 + 6.0 * mt * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
}

/// Evaluate a CSS-style `cubic-bezier(x1, y1, x2, y2)` easing at linear time
/// fraction `t ∈ [0, 1]`, returning the eased progress `∈ [0, 1]`.
///
/// The control points encode time on the x-axis and progress on the y-axis, so
/// we first solve `bezier_x(s) = t` for the curve parameter `s` (Newton-Raphson
/// with a bisection fallback), then read `bezier_y(s)`.
pub(crate) fn cubic_bezier_ease(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }

    // Newton-Raphson from a sensible initial guess.
    let mut s = t;
    for _ in 0..8 {
        let x = bezier_axis(s, x1, x2) - t;
        if x.abs() < 1e-5 {
            return bezier_axis(s, y1, y2);
        }
        let dx = bezier_axis_deriv(s, x1, x2);
        if dx.abs() < 1e-6 {
            break;
        }
        s = (s - x / dx).clamp(0.0, 1.0);
    }

    // Bisection fallback if Newton stalled.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    s = t;
    for _ in 0..16 {
        let x = bezier_axis(s, x1, x2);
        if (x - t).abs() < 1e-5 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = 0.5 * (lo + hi);
    }
    bezier_axis(s, y1, y2)
}

/// Release-fling and spring-back state.
#[derive(Default)]
pub struct FlingState {
    /// Wall-clock instant the current release fling started, or `None` when no
    /// cubic-bézier fling is active. While `Some`, momentum is driven by the
    /// curve rather than by per-frame velocity decay.
    pub(crate) start_time: Cell<Option<AnimInstant>>,
    /// Scroll offset captured at the moment the fling started.
    pub(crate) start_offset: Cell<Vec2d>,
    /// Scroll offset the fling eases toward (`start + projected distance`).
    pub(crate) target_offset: Cell<Vec2d>,
    /// Total duration (seconds) of the active bézier fling.
    pub(crate) duration: Cell<f32>,
    /// Optional easing curve for a programmatic `animate_to`. When `Some`, the
    /// active fling interpolates through this curve instead of the default
    /// release-fling bézier; `None` = a normal touch/mouse release fling.
    pub(crate) curve: RefCell<Option<AnyCurve>>,
    /// Velocity of the spring-back oscillation (px/s). Separate from
    /// `pointer_velocity` (which carries momentum / drag velocity) so the
    /// damped-spring simulation can overshoot and oscillate independently.
    pub(crate) spring_velocity: Cell<Vec2d>,
    /// Wall-clock instant the velocity-based momentum started (set on the
    /// first `update_momentum` frame where velocity exceeds epsilon after a
    /// touch release).  Used to hard-cap the glide at
    /// [`MAX_MOMENTUM_DURATION_S`] so it doesn't creep for 15–20 s.
    pub(crate) momentum_start: Cell<Option<AnimInstant>>,
    /// Velocity retention that replaces the behavior's friction while a key or
    /// track-click glide is coasting, and `None` for ordinary momentum.
    ///
    /// Cleared when the glide settles and whenever any other input takes over
    /// the motion (see [`ScrollState::cancel_fling`]).
    pub(crate) friction: Cell<Option<f32>>,
}

impl FlingState {
    /// Carries an in-flight fling across a rebuild.
    pub(crate) fn adopt_from(&self, prev: &Self, axis: ScrollAxis) {
        self.start_time.set(prev.start_time.get());
        self.start_offset
            .set(ScrollState::axis_only(axis, prev.start_offset.get()));
        self.target_offset
            .set(ScrollState::axis_only(axis, prev.target_offset.get()));
        self.duration.set(prev.duration.get());
        self.curve.replace(prev.curve.borrow().clone());
        self.spring_velocity
            .set(ScrollState::axis_only(axis, prev.spring_velocity.get()));
        self.momentum_start.set(prev.momentum_start.get());
        self.friction.set(prev.friction.get());
    }
}

impl ScrollState {
    /// Cancel any active cubic-bézier release fling.
    ///
    /// Called whenever a new input (touch-down, wheel, keyboard, scrollbar
    /// paging) should take over momentum, so the curve-driven glide does not
    /// keep fighting the fresh interaction.
    pub(crate) fn cancel_fling(&self) {
        self.fling.start_time.set(None);
        self.fling.curve.replace(None);
        // Whoever cancels is taking over the motion, so a glide's own friction
        // must not outlive it and shape the momentum that follows.
        self.fling.friction.set(None);
    }

    /// Pushes the content so it glides `delta` (an internal, scaled offset
    /// change) and keeps decelerating to a stop, as a fling does.
    ///
    /// The push goes through the same momentum model as a drag release, with
    /// [`KEY_GLIDE_FRICTION`] in place of the behavior's, sized so the glide
    /// travels `delta` in total. `carry` keeps the speed already built up, so a
    /// held key adds to the glide in flight instead of starting over; without
    /// it, whatever was moving is stopped first.
    pub(crate) fn glide_by(&self, delta: Vec2d, carry: bool) {
        let carried = if carry && self.fling.friction.get().is_some() {
            Self::axis_only(self.axis, self.pointer_velocity.get())
        } else {
            Vec2d::ZERO
        };
        self.cancel_fling();
        self.fling.spring_velocity.set(Vec2d::ZERO);
        self.release_overscroll_recovery();
        self.reset_overscroll_peak();
        self.clear_velocity_history();

        let push = 1.0 - KEY_GLIDE_FRICTION;
        let delta = Self::axis_only(self.axis, delta);
        self.pointer_velocity.set(Vec2d {
            x: carried.x + delta.x * push,
            y: carried.y + delta.y * push,
        });
        self.fling.friction.set(Some(KEY_GLIDE_FRICTION));
        self.fling.momentum_start.set(None);
        self.begin_scroll();
        self.record_input_event();
        self.request_animation_frame();
    }

    /// Arm a programmatic animation that eases the scroll offset from its
    /// current value to `target` over `duration_s` seconds following `curve`.
    ///
    /// This reuses the existing curve-driven fling machinery in
    /// [`Self::update_momentum`]: it seeds the fling start/target/duration and
    /// records `curve` in [`FlingState::curve`] so the draw loop interpolates
    /// `start + (target - start) · curve(t / duration)` each frame. A
    /// non-positive duration jumps straight to the (clamped) target.
    ///
    /// `target` is an internal (scaled) scroll offset in the same convention as
    /// [`Self::scroll_offset`] (negative = content moved up).
    pub(crate) fn start_animation(&self, target: Vec2d, duration_s: f32, curve: impl Curve) {
        // Clear any live drag/momentum so the animation fully owns the motion.
        self.pointer_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        self.fling.spring_velocity.set(Vec2d { x: 0.0, y: 0.0 });
        self.fling.momentum_start.set(None);
        // A programmatic scroll supersedes any device gesture, so it must not be
        // stalled by an edge-recovery hold left behind by that gesture.
        self.release_overscroll_recovery();
        self.reset_overscroll_peak();

        let start = Self::axis_only(self.axis, self.scroll_offset.get());
        self.scroll_offset.set(start);
        // Non-bouncy scrollables never overshoot; pin the target to the edge.
        let target = if self.bouncy() {
            Self::axis_only(self.axis, target)
        } else {
            self.clamp_offset(Self::axis_only(self.axis, target))
        };

        if duration_s <= 0.0 || (start.x == target.x && start.y == target.y) {
            self.set_scroll_offset(target);
            self.cancel_fling();
            return;
        }

        self.fling.start_offset.set(start);
        self.fling.target_offset.set(target);
        self.fling.duration.set(duration_s);
        self.fling.curve.replace(Some(curve.into_any()));
        self.fling.start_time.set(Some(AnimInstant::now()));
    }

    /// Eases to `target`, an internal (scaled) offset, as a scroll session of its
    /// own. The draw loop reports the session over once the animation settles.
    pub(crate) fn animate_scroll_to(&self, target: Vec2d, duration_s: f32, curve: impl Curve) {
        self.begin_scroll();
        self.start_animation(target, duration_s, curve);
        self.record_input_event();
        self.request_animation_frame();
    }

    /// Arm a cubic-bézier release fling.
    ///
    /// `release_velocity` is the projected launch velocity (px per 120 Hz
    /// frame). The fling runs for a fixed [`FLING_DURATION_S`] window and its
    /// position follows `start + distance · cubic-bezier(t / duration)`.
    ///
    /// The coast distance is derived per axis so the curve's initial slope
    /// (`slope0 = y1 / x1`, the curve's `dy/dx` at `t = 0`) matches the release
    /// speed: the animation leaves the finger at exactly the velocity it was
    /// moving (no visible jump), then decelerates along the curve to a gentle
    /// stop. Because the curve is the only deceleration model here, the
    /// friction field no longer participates in the fling —
    /// `FLING_DURATION_S` is the single knob (longer = farther + slower
    /// settle).
    ///
    ///   v0_px_s = release_velocity / FRAME_REF_120         (px per second)
    ///   v(0)    = distance · slope0 / duration  =!  v0_px_s
    ///   ⇒ distance = v0_px_s · duration / slope0
    ///
    /// Currently unused: touch/mouse release now carries momentum through the
    /// shared velocity + friction model (so it matches trackpad feel) rather
    /// than this bézier fling. Kept available as an alternative fling model.
    #[allow(dead_code)]
    pub(crate) fn start_fling(&self, release_velocity: Vec2d, now: AnimInstant) {
        let release_velocity = Self::axis_only(self.axis, release_velocity);
        if release_velocity.x == 0.0 && release_velocity.y == 0.0 {
            self.cancel_fling();
            return;
        }

        let duration = FLING_DURATION_S;
        let slope0 = FLING_BEZIER_Y1 / FLING_BEZIER_X1;
        // distance = (v_px_frame / FRAME_REF_120) · duration / slope0.
        let k = duration / (FRAME_REF_120 * slope0);
        let dist = Vec2d {
            x: release_velocity.x * k,
            y: release_velocity.y * k,
        };

        let start = Self::axis_only(self.axis, self.scroll_offset.get());
        // debug!("Start: {:?}", start);
        let mut target = Vec2d {
            x: start.x + dist.x,
            y: start.y + dist.y,
        };
        // Non-bouncy scrolling never overshoots, so pin the target to the edge
        // and let the curve ease straight into it.
        if !self.bouncy() {
            target = self.clamp_offset(target);
        }

        if duration <= 0.0 || (dist.x == 0.0 && dist.y == 0.0) {
            self.cancel_fling();
            return;
        }

        self.fling.start_offset.set(start);
        self.fling.target_offset.set(target);
        self.fling.duration.set(duration);
        self.fling.start_time.set(Some(now));
    }

}
