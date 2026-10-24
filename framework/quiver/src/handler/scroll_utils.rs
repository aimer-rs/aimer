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
    last_tick: Instant,
}

impl MomentumScroller {
    pub fn new() -> Self {
        Self {
            remaining: Vec2d::ZERO,
            pixels_per_line: 40.0,
            friction: 0.65,
            min_velocity: 0.01,
            max_velocity: 60.0,
            last_tick: Instant::now(),
        }
    }

    /// Adds an input delta to the exact distance still to be delivered.
    ///
    /// New input extends the remaining distance. It does not estimate a
    /// velocity, so the smoother cannot manufacture inertial travel after the
    /// device stops reporting deltas.
    pub fn on_line_delta(&mut self, delta: Vec2d) {
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
}
