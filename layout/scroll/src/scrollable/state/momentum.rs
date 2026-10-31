//! The per-frame momentum, fling and spring-back step.

use aimer_animation::Curve;
use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::*;
use super::fling::cubic_bezier_ease;
use crate::scrollable::constants::*;
use crate::scrollable::spring::advance_overscroll_spring;

/// Friction the glide starts its cap fade-out with, and how much of it the fade
/// takes away by the time the cap is reached.
const FADE_START_FRICTION: f32 = 0.999;
const FADE_FRICTION_DROP: f32 = 0.099;

#[inline]
fn is_moving(velocity: Vec2d) -> bool {
    velocity.x.abs() > VELOCITY_EPSILON || velocity.y.abs() > VELOCITY_EPSILON
}

/// Advances one axis that sits past its edge and keeps the stretch from
/// reversing through it: returns the new `(offset, spring velocity)`.
///
/// An axis inside its range is returned untouched, spring velocity included.
#[inline]
fn spring_stretched_axis(offset: f32, edge: f32, spring: f32, dt: f32) -> (f32, f32) {
    if offset == edge {
        return (offset, spring);
    }
    let (stretch, spring) = advance_overscroll_spring(offset - edge, spring, dt);
    (edge + stretch, spring)
}

/// Whether the spring carried `offset` from `before` (the edge it was measured
/// against) through to `after` (the edge of the position it landed on).
#[inline]
fn crossed_edge(before: f32, offset: f32, after: f32) -> bool {
    offset != before
        && ((before >= offset && offset >= after) || (before <= offset && offset <= after))
}

/// One frame of spring-back for an axis with no momentum left.
///
/// Runs for in-range axes too: the spring is integrated from a zero stretch so
/// a leftover spring velocity still decays instead of lingering.
#[inline]
fn recover_axis(offset: f32, edge: f32, spring: f32, dt: f32) -> (f32, f32) {
    let stretch = offset - edge;
    let (next_stretch, mut spring) = advance_overscroll_spring(stretch, spring, dt);
    let mut offset = edge + next_stretch;
    // Crossed the edge in one step: land on it rather than overshoot.
    if stretch != 0.0 && (stretch > 0.0) != (next_stretch > 0.0) {
        offset = edge;
        spring = 0.0;
    }
    // Snap to rest when distance from the edge and velocity are both negligible.
    if (offset - edge).abs() < SNAP_EPSILON && spring.abs() < VELOCITY_EPSILON {
        offset = edge;
        spring = 0.0;
    }
    (offset, spring)
}

/// What every step of one momentum frame needs to know about the frame.
#[derive(Clone, Copy)]
struct Frame {
    now: AnimInstant,
    /// Seconds since the previous frame, capped at [`MAX_FRAME_DT`].
    dt: f32,
    /// `dt` in units of the 120 Hz reference frame the tuning constants use.
    ratio: f32,
    /// Resolved once per frame: nothing below changes which device is scrolling.
    bouncy: bool,
}

impl ScrollState {
    /// Update momentum, spring-back, and friction during the draw phase (when
    /// not dragging). Returns the updated offset and whether a redraw is
    /// needed.
    pub(crate) fn update_momentum(&self, offset: Vec2d) -> (Vec2d, bool) {
        self.update_momentum_at(offset, AnimInstant::now())
    }

    /// [`update_momentum`](Self::update_momentum) at an explicit instant, so
    /// the frame-to-frame integration is reproducible without sleeping.
    pub(crate) fn update_momentum_at(&self, offset: Vec2d, now: AnimInstant) -> (Vec2d, bool) {
        let mut offset = Self::axis_only(self.axis, offset);
        let clamped = self.clamp_offset(offset);
        let bouncy = self.bouncy();
        let velocity = Self::axis_only(self.axis, self.pointer_velocity.get());
        self.pointer_velocity.set(velocity);
        self.fling
            .spring_velocity
            .set(Self::axis_only(self.axis, self.fling.spring_velocity.get()));

        let dt = self
            .last_frame_time
            .get()
            .map(|t| now.duration_since(t).as_secs_f32())
            .unwrap_or(FRAME_REF_120)
            .min(MAX_FRAME_DT);
        self.last_frame_time.set(Some(now));
        let frame = Frame {
            now,
            dt,
            ratio: dt / FRAME_REF_120,
            bouncy,
        };

        let past_edge = offset.x != clamped.x || offset.y != clamped.y;
        if bouncy && past_edge && self.overscroll_recovery_held_at(now) {
            let held = self.clamp_overscroll(offset);
            let out_of_bounds = held.x != clamped.x || held.y != clamped.y;
            return (held, out_of_bounds);
        }

        let mut needs_redraw = false;
        if let Some(fling_start) = self.fling.start_time.get() {
            offset = self.step_fling(offset, clamped, fling_start, frame);
            needs_redraw = true;
        } else if is_moving(velocity) {
            match self.step_coast(offset, velocity, frame) {
                Some(next) => {
                    offset = next;
                    needs_redraw = true;
                }
                // The glide hit its duration cap and was stopped.
                None => return (offset, false),
            }
        } else if velocity.x != 0.0 || velocity.y != 0.0 {
            self.pointer_velocity.set(Vec2d::ZERO);
            self.fling.momentum_start.set(None);
            self.fling.friction.set(None);
        } else {
            // Nothing is coasting any more, whichever way it came to rest.
            self.fling.friction.set(None);
        }

        if bouncy && !is_moving(self.pointer_velocity.get()) && past_edge_of(offset, clamped) {
            let spring = self.fling.spring_velocity.get();
            let (x, sx) = recover_axis(offset.x, clamped.x, spring.x, dt);
            let (y, sy) = recover_axis(offset.y, clamped.y, spring.y, dt);
            offset = Vec2d { x, y };
            self.fling.spring_velocity.set(Vec2d { x: sx, y: sy });
            needs_redraw = true;
        } else if !bouncy {
            // A rigid viewport never rests outside its range. Clamp where the
            // step landed, not where the frame started, or the step is lost.
            offset = self.clamp_offset(offset);
        }

        // Hard-cap overscroll to a fraction of viewport (prevents unlimited
        // rubber-band from trackpad or high-velocity flings).
        offset = self.clamp_overscroll(offset);

        // Keep the peak after reaching the boundary: the current device
        // gesture may still have native momentum-tail events in flight.
        self.record_overscroll_peak(offset);

        (Self::axis_only(self.axis, offset), needs_redraw)
    }

    /// One frame of a curve-driven fling (a release fling or `animate_to`).
    /// Returns the new offset.
    fn step_fling(
        &self,
        offset: Vec2d,
        clamped: Vec2d,
        fling_start: AnimInstant,
        Frame { now, dt, ratio, bouncy }: Frame,
    ) -> Vec2d {
        let duration = self.fling.duration.get();
        let elapsed = now.duration_since(fling_start).as_secs_f32();
        let u = if duration > 0.0 {
            (elapsed / duration).clamp(0.0, 1.0)
        } else {
            1.0
        };

        let eased = match self.fling.curve.borrow().as_ref() {
            Some(curve) => curve.transform(u),
            None => cubic_bezier_ease(
                u,
                FLING_BEZIER_X1,
                FLING_BEZIER_Y1,
                FLING_BEZIER_X2,
                FLING_BEZIER_Y2,
            ),
        };
        let start = self.fling.start_offset.get();
        let target = self.fling.target_offset.get();
        let mut next = Vec2d {
            x: start.x + (target.x - start.x) * eased,
            y: start.y + (target.y - start.y) * eased,
        };

        let step = Vec2d {
            x: next.x - offset.x,
            y: next.y - offset.y,
        };
        let velocity = if dt > 0.0 {
            Vec2d {
                x: step.x / ratio,
                y: step.y / ratio,
            }
        } else {
            Vec2d::default()
        };
        self.pointer_velocity.set(velocity);

        if past_edge_of(next, clamped) && bouncy {
            // Hand the remaining momentum to the velocity-based spring so the
            // content bounces and recovers from the edge like native iOS.
            self.cancel_fling();
        } else {
            // Snap to rest once finished, or once the tail step becomes
            // sub-pixel (the curve crawls toward the target for a long time
            // after covering ~95% of the distance early).
            let tail_done = u >= FLING_TAIL_START
                && step.x.abs() < FLING_END_STEP_PX
                && step.y.abs() < FLING_END_STEP_PX;
            if u >= 1.0 || tail_done {
                next = target;
                self.pointer_velocity.set(Vec2d::ZERO);
                self.cancel_fling();
            }
        }
        next
    }

    /// One frame of velocity-decay momentum (touch release without a fling
    /// curve, key and track-click glides), including the rubber band and
    /// spring-back when it carries the content past an edge.
    ///
    /// Returns `None` when the glide has run for [`MAX_MOMENTUM_DURATION_S`]
    /// and was stopped, in which case the offset must be left as it was.
    fn step_coast(
        &self,
        mut offset: Vec2d,
        mut velocity: Vec2d,
        Frame { now, dt, ratio, bouncy }: Frame,
    ) -> Option<Vec2d> {
        // Clear any in-flight spring oscillation when fresh momentum begins.
        self.fling.spring_velocity.set(Vec2d::ZERO);
        let momentum_elapsed = match self.fling.momentum_start.get() {
            Some(t) => now.duration_since(t).as_secs_f32(),
            None => {
                self.fling.momentum_start.set(Some(now));
                0.0
            }
        };
        if momentum_elapsed >= MAX_MOMENTUM_DURATION_S {
            self.pointer_velocity.set(Vec2d::ZERO);
            self.fling.momentum_start.set(None);
            return None;
        }
        // Fade-out zone: in the last MOMENTUM_FADEOUT_S seconds before the
        // cap, apply progressively increasing friction so the velocity bleeds
        // to near-zero instead of hitting a wall.
        let remaining = MAX_MOMENTUM_DURATION_S - momentum_elapsed;
        if remaining < MOMENTUM_FADEOUT_S {
            // progress in [0, 1]: 0 = fade starts, 1 = at the cap.
            let progress = 1.0 - (remaining / MOMENTUM_FADEOUT_S);
            let fade_friction = FADE_START_FRICTION - FADE_FRICTION_DROP * progress;
            velocity.x *= fade_friction.powf(ratio);
            velocity.y *= fade_friction.powf(ratio);
        }

        let friction = self
            .fling
            .friction
            .get()
            .unwrap_or(self.scroll_behavior.friction);
        let decay = friction.powf(ratio);
        offset.x += velocity.x * ratio;
        offset.y += velocity.y * ratio;
        if bouncy {
            // An axis stopped by the overscroll cap loses its velocity.
            let capped = self.clamp_overscroll(offset);
            if capped.x != offset.x {
                offset.x = capped.x;
                velocity.x = 0.0;
            }
            if capped.y != offset.y {
                offset.y = capped.y;
                velocity.y = 0.0;
            }
        }
        velocity.x *= decay;
        velocity.y *= decay;

        let range = self.clamp_offset(offset);
        if bouncy {
            let oob_decay = OVERSCROLL_FRICTION.powf(ratio);
            if offset.x != range.x {
                velocity.x *= oob_decay;
            }
            if offset.y != range.y {
                velocity.y *= oob_decay;
            }

            // A stretched axis hands its momentum to the spring.
            let spring = self.fling.spring_velocity.get();
            if offset.x != range.x {
                velocity.x = 0.0;
            }
            if offset.y != range.y {
                velocity.y = 0.0;
            }
            let (x, mut sx) = spring_stretched_axis(offset.x, range.x, spring.x, dt);
            let (y, mut sy) = spring_stretched_axis(offset.y, range.y, spring.y, dt);
            offset = Vec2d { x, y };

            // Snap to the boundary if the spring crossed through it.
            let landed = self.clamp_offset(offset);
            if crossed_edge(range.x, offset.x, landed.x) {
                offset.x = landed.x;
                sx = 0.0;
            }
            if crossed_edge(range.y, offset.y, landed.y) {
                offset.y = landed.y;
                sy = 0.0;
            }
            self.fling.spring_velocity.set(Vec2d { x: sx, y: sy });
        } else {
            if offset.x != range.x {
                offset.x = range.x;
                velocity.x = 0.0;
            }
            if offset.y != range.y {
                offset.y = range.y;
                velocity.y = 0.0;
            }
        }

        self.pointer_velocity.set(velocity);
        Some(offset)
    }
}

#[inline]
fn past_edge_of(offset: Vec2d, clamped: Vec2d) -> bool {
    offset.x != clamped.x || offset.y != clamped.y
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_animation::curves;

    use super::*;
    use crate::scrollable::scroll_behavior::{ScrollAxis, ScrollBehavior};

    /// Every frame of a run, reduced to one order-sensitive hash.
    ///
    /// The momentum step is a long chain of float updates, so instead of
    /// asserting a handful of sample points these tests pin the whole
    /// trajectory: offset, redraw flag, and both velocities on every frame.
    /// Any change to the integration order or the branch structure shows up as
    /// a different fingerprint.
    struct Trace {
        hash: u64,
        last: Vec2d,
        redraws: usize,
    }

    fn fold(hash: &mut u64, value: u32) {
        for byte in value.to_le_bytes() {
            *hash ^= byte as u64;
            *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn engine(axis: ScrollAxis, bouncy: bool) -> ScrollState {
        let mut state = ScrollState::for_test_at(Vec2d::ZERO);
        state.axis = axis;
        state.scroll_behavior = if bouncy {
            ScrollBehavior::default()
        } else {
            ScrollBehavior::no_bounce()
        };
        state.cached_max_scroll.set(Vec2d {
            x: 1000.0,
            y: 1000.0,
        });
        state.cached_viewport.set((400.0, 800.0));
        state
    }

    /// Runs `dts_us` frames, repeating the pattern, starting from `offset`.
    fn run(
        state: &ScrollState,
        start: AnimInstant,
        mut offset: Vec2d,
        frames: usize,
        dts_us: &[u64],
    ) -> Trace {
        let mut at = start;
        let mut trace = Trace {
            hash: 0xcbf2_9ce4_8422_2325,
            last: offset,
            redraws: 0,
        };
        state.scroll_offset.set(offset);
        for frame in 0..frames {
            at = at + Duration::from_micros(dts_us[frame % dts_us.len()]);
            let (next, redraw) = state.update_momentum_at(offset, at);
            offset = next;
            state.scroll_offset.set(offset);
            let velocity = state.pointer_velocity.get();
            let spring = state.fling.spring_velocity.get();
            for value in [offset.x, offset.y, velocity.x, velocity.y, spring.x, spring.y] {
                fold(&mut trace.hash, value.to_bits());
            }
            fold(&mut trace.hash, redraw as u32);
            trace.redraws += redraw as usize;
            trace.last = offset;
        }
        trace
    }

    const STEADY_120: &[u64] = &[8_333];
    const STEADY_60: &[u64] = &[16_667];
    // Includes a stall longer than MAX_FRAME_DT so the dt clamp is exercised.
    const UNEVEN: &[u64] = &[4_000, 16_000, 60_000, 8_333, 33_000];

    fn coast(state: &ScrollState, velocity: Vec2d) {
        state.pointer_velocity.set(velocity);
    }

    fn v(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    /// `(name, fingerprint)` for every scenario, in a fixed order.
    fn scenarios() -> Vec<(&'static str, Trace)> {
        // One epoch for the whole table: every instant below is derived from it,
        // so no scenario depends on how long the previous one took to run.
        let t0 = AnimInstant::now();
        let mut out = Vec::new();

        let s = engine(ScrollAxis::Vertical, true);
        coast(&s, v(0.0, -12.0));
        out.push(("coast in range", run(&s, t0, v(0.0, -300.0), 400, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        coast(&s, v(0.0, -30.0));
        out.push(("coast into the end edge", run(&s, t0, v(0.0, -950.0), 400, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        coast(&s, v(0.0, 30.0));
        out.push(("coast into the start edge", run(&s, t0, v(0.0, -50.0), 400, STEADY_120)));

        let s = engine(ScrollAxis::Vertical, false);
        coast(&s, v(0.0, -30.0));
        out.push(("rigid coast into the end edge", run(&s, t0, v(0.0, -950.0), 400, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        out.push(("spring back past the start", run(&s, t0, v(0.0, 60.0), 300, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        out.push(("spring back past the end", run(&s, t0, v(0.0, -1060.0), 300, STEADY_120)));

        let s = engine(ScrollAxis::Vertical, true);
        s.fling.spring_velocity.set(v(0.0, -500.0));
        out.push(("spring with initial velocity", run(&s, t0, v(0.0, 20.0), 300, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, false);
        s.start_fling(v(0.0, -20.0), t0);
        out.push(("rigid release fling", run(&s, t0, v(0.0, -100.0), 400, STEADY_120)));

        let s = engine(ScrollAxis::Vertical, true);
        s.start_fling(v(0.0, -40.0), t0);
        out.push(("bouncy fling into the end edge", run(&s, t0, v(0.0, -900.0), 600, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        s.start_fling(v(0.0, -20.0), t0);
        out.push(("fling with uneven frames", run(&s, t0, v(0.0, -100.0), 300, UNEVEN)));

        let s = engine(ScrollAxis::Vertical, true);
        s.fling.start_offset.set(v(0.0, -100.0));
        s.fling.target_offset.set(v(0.0, -700.0));
        s.fling.duration.set(0.6);
        s.fling.start_time.set(Some(t0));
        s.fling.curve.replace(Some(curves::LINEAR.into_any()));
        out.push(("curve-driven animation", run(&s, t0, v(0.0, -100.0), 120, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        coast(&s, v(0.0, -4.0));
        s.fling.friction.set(Some(0.999_99));
        out.push(("coast to the duration cap", run(&s, t0, v(0.0, -10.0), 700, STEADY_120)));

        let s = engine(ScrollAxis::Vertical, true);
        s.hold_overscroll_recovery_at(t0);
        out.push(("held overscroll then recovery", run(&s, t0, v(0.0, 40.0), 60, STEADY_60)));

        let s = engine(ScrollAxis::Horizontal, true);
        coast(&s, v(-30.0, 5.0));
        out.push(("horizontal coast into the end edge", run(&s, t0, v(-950.0, -20.0), 400, STEADY_60)));

        let s = engine(ScrollAxis::Vertical, true);
        coast(&s, v(0.0, -25.0));
        out.push(("coast with uneven frames", run(&s, t0, v(0.0, -700.0), 400, UNEVEN)));

        out
    }

    /// Fingerprints recorded from the implementation before `update_momentum`
    /// was restructured; the restructure must reproduce them bit for bit.
    ///
    /// Update an entry only for a deliberate behavior change, and say why.
    const EXPECTED: &[(&str, u64)] = &[
        ("coast in range", 0xe0c983a7d7e3adf5),
        ("coast into the end edge", 0xb70ac3e035aa2905),
        ("coast into the start edge", 0xd0f9175a62d927e7),
        ("rigid coast into the end edge", 0x11be1d796dcaa184),
        ("spring back past the start", 0xab36f51c6550747a),
        ("spring back past the end", 0xe5a61d85f812e791),
        ("spring with initial velocity", 0x54982fd3c1d8d9d1),
        ("rigid release fling", 0x462ac01703b0d8d1),
        ("bouncy fling into the end edge", 0xd2c7c9b5bbe8a9a6),
        ("fling with uneven frames", 0xc47cbf5220c48851),
        ("curve-driven animation", 0xf69ec709d9dad21a),
        ("coast to the duration cap", 0x35d1253ea90211eb),
        ("held overscroll then recovery", 0x65967080f6b0c56a),
        ("horizontal coast into the end edge", 0x33dd4ee77e3a8cd5),
        ("coast with uneven frames", 0xd3e276fdd36df440),
    ];

    #[test]
    fn momentum_trajectories_are_unchanged() {
        let mut mismatches = String::new();
        for (name, trace) in scenarios() {
            let expected = EXPECTED.iter().find(|(n, _)| *n == name).map(|(_, h)| *h);
            if expected != Some(trace.hash) {
                mismatches.push_str(&format!(
                    "    (\"{name}\", {:#018x}), // redraws {} last ({}, {})\n",
                    trace.hash, trace.redraws, trace.last.x, trace.last.y
                ));
            }
        }
        assert!(mismatches.is_empty(), "fingerprints:\n{mismatches}");
    }

    #[test]
    fn a_stretched_edge_springs_back_and_stops_asking_for_frames() {
        let s = engine(ScrollAxis::Vertical, true);
        let trace = run(&s, AnimInstant::now(), v(0.0, 60.0), 600, STEADY_60);
        assert_eq!(trace.last.y, 0.0, "recovery settles exactly on the edge");
        let (_, redraw) = s.update_momentum_at(trace.last, AnimInstant::now() + Duration::from_secs(60));
        assert!(!redraw, "a settled viewport schedules no more frames");
    }

    #[test]
    fn a_rigid_viewport_never_leaves_its_range() {
        let s = engine(ScrollAxis::Vertical, false);
        coast(&s, v(0.0, -30.0));
        let trace = run(&s, AnimInstant::now(), v(0.0, -950.0), 400, STEADY_60);
        assert_eq!(trace.last.y, -1000.0, "momentum is absorbed by the end edge");
    }

    #[test]
    fn a_rigid_coast_carries_the_content_and_then_stops_asking_for_frames() {
        let s = engine(ScrollAxis::Vertical, false);
        coast(&s, v(0.0, -12.0));
        let trace = run(&s, AnimInstant::now(), v(0.0, -300.0), 400, STEADY_60);
        assert_eq!(trace.last.y, -1000.0, "the coast runs on to the end edge");
        assert!(
            trace.redraws < 400,
            "frames stop once the coast has settled, got {} of 400",
            trace.redraws
        );
    }

    #[test]
    fn a_rigid_release_fling_moves_the_content_to_its_target() {
        let s = engine(ScrollAxis::Vertical, false);
        let t0 = AnimInstant::now();
        s.start_fling(v(0.0, -20.0), t0);
        let trace = run(&s, t0, v(0.0, -100.0), 400, STEADY_120);
        assert_eq!(
            trace.last.y, -1000.0,
            "a fling that outruns the range is pinned to the end edge"
        );
        assert!(trace.redraws < 400, "the fling settles instead of spinning");
    }

    #[test]
    fn a_rigid_animation_reaches_its_target() {
        let s = engine(ScrollAxis::Vertical, false);
        let t0 = AnimInstant::now();
        s.fling.start_offset.set(v(0.0, -100.0));
        s.fling.target_offset.set(v(0.0, -700.0));
        s.fling.duration.set(0.6);
        s.fling.start_time.set(Some(t0));
        s.fling.curve.replace(Some(curves::LINEAR.into_any()));
        let trace = run(&s, t0, v(0.0, -100.0), 120, STEADY_60);
        assert_eq!(trace.last.y, -700.0);
    }
}
