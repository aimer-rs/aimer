use aimer_attribute::Vec2d;
use aimer_utils::AnimInstant as Instant;
use winit::dpi::PhysicalPosition;
use winit::event::MouseScrollDelta;

/// Pixels-per-line heuristic for native platforms where LineDelta
/// actually occurs (macOS/Linux/Windows with certain mice). Not used
/// on wasm since winit always reports PixelDelta there.
const LINE_HEIGHT_PX: f64 = 20.0;

pub fn to_pixel_delta(delta: MouseScrollDelta, scale_factor: f64) -> PhysicalPosition<f64> {
    match delta {
        MouseScrollDelta::PixelDelta(pos) => pos,
        MouseScrollDelta::LineDelta(x, y) => PhysicalPosition::new(
            x as f64 * LINE_HEIGHT_PX * scale_factor,
            y as f64 * LINE_HEIGHT_PX * scale_factor,
        ),
    }
}

/// The length of the shortest step the smoother takes: the clamp's lower bound.
///
/// A step taken right after another is this long, however little time passed.
pub(crate) const FOLLOW_UP_DT: f64 = 1.0 / 120.0;

/// A frame-rate-independent input-distance smoother.
///
/// Despite its historical name, this type does not derive or retain momentum.
/// It stores only undelivered input distance, subtracting every emitted frame
/// step until the exact total has been consumed.
pub struct MomentumScroller {
    remaining: Vec2d,
    pub(crate) pixels_per_line: f32,
    pub(crate) friction: f32,
    min_velocity: f32,
    pub(crate) max_velocity: f32,
    /// Seconds over which pending distance eases out, or `None` for the
    /// friction model above.
    ///
    /// The friction model is defined per rendered frame: each frame takes a
    /// fixed fraction plus a fixed-length follow-up step, so how long a notch
    /// takes to land depends on the display's refresh rate, and on a fast
    /// display a clamp on the step length makes it land sooner still. Easing by
    /// a time constant depends only on the wall-clock time between frames, so a
    /// 60 Hz and a 240 Hz window glide for the same duration.
    pub(crate) time_constant: Option<f64>,
    last_tick: Instant,
}

/// Distance, in pixels, below which an easing smoother delivers what is left.
///
/// An exponential tail never reaches zero; the last half pixel is invisible but
/// would keep frames being requested for no visible change.
const EASING_SETTLE_PX: f32 = 0.5;

/// Shortest and longest frame time the easing smoother believes.
///
/// A stalled frame must not deliver a whole gesture at once, and a burst of
/// frames closer together than a refresh cannot make it glide any faster. The
/// lower bound is the friction model's own (`FOLLOW_UP_DT`), so a frame driven
/// without waiting for a display — a headless pump — advances a glide by the
/// same amount in both. Only a display faster than 120 Hz is affected: its
/// easing runs in proportion faster than the wall clock.
const EASING_DT_RANGE: (f64, f64) = (FOLLOW_UP_DT, 1.0 / 20.0);

impl MomentumScroller {
    pub fn new() -> Self {
        Self {
            remaining: Vec2d::ZERO,
            pixels_per_line: 40.0,
            friction: 0.65,
            min_velocity: 0.01,
            max_velocity: 60.0,
            time_constant: None,
            last_tick: Instant::now(),
        }
    }

    /// Adds an input delta to the exact distance still to be delivered.
    ///
    /// New input extends the remaining distance. It does not estimate a
    /// velocity, so the smoother cannot manufacture inertial travel after the
    /// device stops reporting deltas.
    pub fn on_line_delta(&mut self, delta: Vec2d) {
        if self.time_constant.is_some() && !self.is_active() {
            // Easing is measured in wall-clock time, so the time since the last
            // frame of the previous gesture must not count towards this one:
            // a notch after a pause would otherwise land in its first frame.
            self.last_tick = Instant::now();
        }
        self.remaining = self.remaining + delta.scale(self.pixels_per_line);
    }

    pub fn clear(&mut self) {
        self.remaining = Vec2d::ZERO;
        self.last_tick = Instant::now();
    }

    #[inline]
    pub fn is_active(&self) -> bool {
        self.remaining.magnitude() > 0.0
    }

    /// Returns the next frame-synchronized portion of the pending input.
    ///
    /// Call it once per rendered frame. The response is bounded by
    /// `max_velocity` and always removed from `remaining`. The final frame
    /// emits the residue exactly, preserving total input distance without
    /// overshoot.
    pub fn tick(&mut self) -> Option<PhysicalPosition<f64>> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;
        self.frame_step(dt)
    }

    /// The portion of pending input one rendered frame delivers: a step of
    /// `dt`, followed by the shortest step the smoother takes.
    ///
    /// The follow-up is what the frame loop's second tick used to deliver as
    /// a separate scroll event a few microseconds after the first, and its
    /// clamped step length is part of how the friction values below feel at
    /// every refresh rate. It stays part of the response; it no longer costs a
    /// second trip through the widget tree.
    pub(crate) fn frame_step(&mut self, dt: f64) -> Option<PhysicalPosition<f64>> {
        if let Some(time_constant) = self.time_constant {
            return self.eased_step(dt, time_constant);
        }
        let first = self.tick_with_dt(dt);
        let follow_up = self.tick_with_dt(FOLLOW_UP_DT);
        match (first, follow_up) {
            (Some(first), Some(follow_up)) => Some(PhysicalPosition::new(
                first.x + follow_up.x,
                first.y + follow_up.y,
            )),
            (step, None) | (None, step) => step,
        }
    }

    /// The portion of pending input that eases out in `dt` seconds.
    ///
    /// `1 - e^(-dt/τ)` of what is owed, which composes: two steps of `dt / 2`
    /// deliver what one step of `dt` does. `max_velocity` keeps its meaning of
    /// pixels per 1/60 s tick, and a frame of the friction model was two of
    /// them, so the cap is `max_velocity` × 120 pixels per second.
    fn eased_step(&mut self, dt: f64, time_constant: f64) -> Option<PhysicalPosition<f64>> {
        let magnitude = self.remaining.magnitude();
        if magnitude == 0.0 {
            return None;
        }
        if magnitude <= EASING_SETTLE_PX {
            let rest = self.remaining;
            self.remaining = Vec2d::ZERO;
            return Some(PhysicalPosition::new(rest.x as f64, rest.y as f64));
        }

        let dt = dt.clamp(EASING_DT_RANGE.0, EASING_DT_RANGE.1);
        let fraction = 1.0 - (-dt / time_constant).exp();
        let mut step = self.remaining.scale(fraction as f32);
        let limit = self.max_velocity as f64 * 120.0 * dt;
        let step_magnitude = step.magnitude() as f64;
        if step_magnitude > limit {
            step = step.scale((limit / step_magnitude) as f32);
        }
        self.remaining = Vec2d {
            x: self.remaining.x - step.x,
            y: self.remaining.y - step.y,
        };
        Some(PhysicalPosition::new(step.x as f64, step.y as f64))
    }

    fn tick_with_dt(&mut self, dt: f64) -> Option<PhysicalPosition<f64>> {
        let magnitude = self.remaining.magnitude();
        if magnitude == 0.0 {
            return None;
        }

        if magnitude <= self.min_velocity {
            let final_step = self.remaining;
            self.remaining = Vec2d::ZERO;
            return Some(PhysicalPosition::new(
                final_step.x as f64,
                final_step.y as f64,
            ));
        }

        let frame_ratio = dt.clamp(1.0 / 120.0, 1.0 / 30.0) * 60.0;
        let response = 1.0 - (self.friction as f64).powf(frame_ratio);
        let mut step = self.remaining.scale(response as f32);
        let step_magnitude = step.magnitude();
        if step_magnitude > self.max_velocity {
            step = step.scale(self.max_velocity / step_magnitude);
        }
        self.remaining = Vec2d {
            x: self.remaining.x - step.x,
            y: self.remaining.y - step.y,
        };

        Some(PhysicalPosition::new(step.x as f64, step.y as f64))
    }
}

impl Default for MomentumScroller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_preserves_distance_without_adding_momentum() {
        let mut scroller = MomentumScroller::new();
        scroller.pixels_per_line = 1.0;
        scroller.on_line_delta(Vec2d { x: 0.0, y: -8.0 });

        let mut total: f64 = 0.0;
        let mut frames = 0;
        while let Some(delta) = scroller.tick_with_dt(1.0 / 60.0) {
            assert!(delta.y <= 0.0);
            total += delta.y;
            frames += 1;
            assert!(frames < 30);
        }

        assert!(frames > 1);
        assert!((total + 8.0).abs() < 0.0001);
    }

    fn scroller(friction: f32, max_velocity: f32, remaining: f32) -> MomentumScroller {
        let mut scroller = MomentumScroller::new();
        scroller.pixels_per_line = 1.0;
        scroller.friction = friction;
        scroller.max_velocity = max_velocity;
        scroller.remaining = Vec2d {
            x: remaining * 0.25,
            y: -remaining,
        };
        scroller
    }

    fn sum(
        first: Option<PhysicalPosition<f64>>,
        second: Option<PhysicalPosition<f64>>,
    ) -> Option<PhysicalPosition<f64>> {
        match (first, second) {
            (Some(a), Some(b)) => Some(PhysicalPosition::new(a.x + b.x, a.y + b.y)),
            (step, None) | (None, step) => step,
        }
    }

    /// A frame used to be two ticks, the second one a moment after the first
    /// and so always the shortest step the smoother takes. Delivering them as
    /// one step must move exactly as far, and leave exactly as much behind,
    /// at every frame rate, for every tuning, and where the caps bite.
    #[test]
    fn one_frame_step_is_exactly_the_two_ticks_it_replaces() {
        for (friction, max_velocity) in [(0.40, 60.0), (0.15, 80.0), (0.65, 60.0), (0.9, 5.0)] {
            for dt in [
                1.0 / 240.0,
                1.0 / 120.0,
                1.0 / 90.0,
                1.0 / 60.0,
                1.0 / 45.0,
                1.0 / 30.0,
                0.1,
            ] {
                for remaining in [0.004, 0.5, 8.0, 40.0, 120.0, 500.0, 5000.0] {
                    let mut former = scroller(friction, max_velocity, remaining);
                    let mut now = scroller(friction, max_velocity, remaining);

                    let first = former.tick_with_dt(dt);
                    let second = former.tick_with_dt(FOLLOW_UP_DT);
                    let step = now.frame_step(dt);

                    let case = format!(
                        "friction {friction}, cap {max_velocity}, dt {dt}, remaining {remaining}"
                    );
                    assert_eq!(step, sum(first, second), "step, {case}");
                    assert_eq!(now.remaining, former.remaining, "left behind, {case}");
                }
            }
        }
    }

    #[test]
    fn a_frame_step_never_beats_the_per_tick_cap_twice() {
        let mut scroller = scroller(0.4, 60.0, 100_000.0);
        let step = scroller.frame_step(1.0 / 60.0).expect("distance is owed");
        let moved = (step.x * step.x + step.y * step.y).sqrt();

        assert!(moved <= 2.0 * 60.0 + 1e-9, "moved {moved}");
        assert!(
            moved > 60.0,
            "the second sub-step still contributes: {moved}"
        );
    }

    #[test]
    fn nothing_owed_means_no_step() {
        let mut scroller = scroller(0.4, 60.0, 0.0);
        assert_eq!(scroller.frame_step(1.0 / 60.0), None);
        assert!(!scroller.is_active());
    }

    #[test]
    fn frames_deliver_the_exact_total_and_stop() {
        let mut scroller = scroller(0.4, 60.0, 300.0);
        let mut total = 0.0;
        let mut frames = 0;
        while let Some(step) = scroller.frame_step(1.0 / 60.0) {
            total += step.y;
            frames += 1;
            assert!(frames < 100, "the smoother never finished");
        }
        assert!((total + 300.0).abs() < 1e-3, "delivered {total}");
        assert!(!scroller.is_active());
    }

    #[test]
    fn repeated_deltas_extend_the_remaining_distance() {
        let mut scroller = MomentumScroller::new();
        scroller.pixels_per_line = 1.0;
        scroller.on_line_delta(Vec2d { x: 0.0, y: -8.0 });
        let first = scroller.tick_with_dt(1.0 / 60.0).unwrap();
        scroller.on_line_delta(Vec2d { x: 0.0, y: -8.0 });

        let mut total: f64 = first.y;
        while let Some(delta) = scroller.tick_with_dt(1.0 / 60.0) {
            total += delta.y;
        }

        assert!((total + 16.0).abs() < 0.0001);
    }
    /// A smoother easing over `time_constant` seconds with `remaining` pixels owed.
    fn eased(time_constant: f64, remaining: f32) -> MomentumScroller {
        let mut scroller = scroller(0.4, 60.0, remaining);
        scroller.remaining = Vec2d { x: 0.0, y: -remaining };
        scroller.time_constant = Some(time_constant);
        scroller
    }

    /// What is still owed after `frames` frames of `dt` seconds each.
    fn owed_after(scroller: &mut MomentumScroller, dt: f64, frames: usize) -> f32 {
        for _ in 0..frames {
            scroller.frame_step(dt);
        }
        -scroller.remaining.y
    }

    #[test]
    fn an_eased_notch_is_spread_over_frames_not_delivered_at_once() {
        let mut scroller = eased(0.05, 90.0);
        let first = scroller.frame_step(1.0 / 60.0).expect("distance is owed");

        // The 0.40 friction smoother hands over three quarters of a notch in
        // its first frame, which reads as a jump rather than a glide.
        assert!(-first.y < 90.0 * 0.35, "first frame moved {}", -first.y);
        assert!(-first.y > 0.0);
    }

    /// The same wall-clock time must leave the same distance owed whatever the
    /// refresh rate, or a faster display would scroll differently.
    #[test]
    fn easing_takes_the_same_time_at_every_refresh_rate_up_to_120_hz() {
        let at_60 = owed_after(&mut eased(0.05, 90.0), 1.0 / 60.0, 6);
        let at_120 = owed_after(&mut eased(0.05, 90.0), 1.0 / 120.0, 12);
        let at_90 = owed_after(&mut eased(0.05, 90.0), 1.0 / 90.0, 9);

        // 100 ms each, up to the length of one frame.
        for owed in [at_120, at_90] {
            assert!((owed - at_60).abs() < 6.0, "{owed} vs {at_60}");
        }
    }

    #[test]
    fn easing_delivers_the_exact_total_and_settles() {
        for dt in [1.0 / 120.0, 1.0 / 90.0, 1.0 / 60.0, 1.0 / 30.0] {
            let mut scroller = eased(0.05, 90.0);
            let mut total = 0.0;
            let mut frames = 0;
            while let Some(step) = scroller.frame_step(dt) {
                assert!(step.y <= 0.0, "scrolled backwards");
                total += step.y;
                frames += 1;
                assert!(frames < 400, "never settled at dt {dt}");
            }
            assert!((total + 90.0).abs() < 1e-3, "delivered {total} at dt {dt}");
            assert!(!scroller.is_active());
        }
    }

    #[test]
    fn easing_settles_within_a_third_of_a_second() {
        let mut scroller = eased(0.05, 90.0);
        let mut elapsed = 0.0;
        while scroller.frame_step(1.0 / 60.0).is_some() {
            elapsed += 1.0 / 60.0;
        }
        assert!(elapsed < 0.34, "took {elapsed}s");
    }

    #[test]
    fn easing_never_exceeds_the_speed_limit() {
        let mut scroller = eased(0.05, 100_000.0);
        scroller.max_velocity = 60.0;
        let step = scroller.frame_step(1.0 / 60.0).expect("distance is owed");

        // `max_velocity` is per 1/60 s tick, and a frame was two of them.
        assert!(-step.y <= 120.0 + 1e-6, "moved {}", -step.y);
    }

    #[test]
    fn a_fresh_notch_after_idle_does_not_jump() {
        let mut scroller = eased(0.05, 0.0);
        scroller.last_tick = Instant::now() - std::time::Duration::from_secs(5);
        scroller.on_line_delta(Vec2d { x: 0.0, y: -90.0 });

        // The wall clock stood still for five seconds; the first step must not
        // treat that as five seconds of easing.
        let first = scroller.tick().expect("distance is owed");
        assert!(-first.y < 90.0 * 0.5, "first step moved {}", -first.y);
    }

}
