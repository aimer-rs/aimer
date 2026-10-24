//! Time-aware record of how fast a pointer drag was moving.
//!
//! A release fling is only as good as the speed it is launched with, and that
//! speed is a property of a *moment*: the instant the finger left the glass.
//! A drag that races across the viewport and then eases to a standstill before
//! the lift must launch nothing at all, even though the very same gesture was
//! moving fast a fraction of a second earlier.
//!
//! [`VelocityHistory`] therefore records every drag sample together with the
//! wall-clock slice it describes and answers with the average speed over the
//! last [`VELOCITY_HORIZON_S`] seconds only — total travel divided by total
//! time, the physical definition of a velocity. Samples older than the horizon
//! belong to an earlier part of the gesture and are simply not asked.
//!
//! The duration weighting matters as much as the horizon: pointer moves do not
//! arrive on a fixed cadence (a browser coalesces a burst of them into one
//! dispatch, a native host delivers one per frame), so a plain mean over the
//! last *n* samples would let a handful of dense, fast samples outvote the
//! long, slow slice that actually ended the gesture.

use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use crate::scrollable::constants::{MIN_MOVE_DT, VELOCITY_HISTORY_SIZE, VELOCITY_HORIZON_S};

/// One measured slice of finger travel.
#[derive(Clone, Copy)]
struct VelocitySample {
    /// Average velocity across the slice, in px per 120 Hz frame — the unit
    /// the whole scroll engine works in.
    velocity: Vec2d,
    /// Wall-clock length of the slice in seconds, always `>= MIN_MOVE_DT` so
    /// it can be used as a weight without guarding against zero.
    dt: f32,
    /// Instant the slice ended, which is when the sample was recorded.
    at: AnimInstant,
}

/// Fixed-capacity ring buffer of recent drag-velocity samples.
///
/// The buffer is inline (no heap allocation) and reading it walks newest-first
/// and stops at the first sample outside the horizon, so a query costs a
/// handful of adds even on a long gesture.
#[derive(Clone)]
pub(crate) struct VelocityHistory {
    samples: [VelocitySample; VELOCITY_HISTORY_SIZE],
    count: usize,
    write_pos: usize,
}

impl VelocityHistory {
    /// Creates an empty history.
    ///
    /// The backing slots are pre-filled with a placeholder that is never read:
    /// `count` alone decides which slots hold real samples.
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            samples: [VelocitySample {
                velocity: Vec2d::ZERO,
                dt: 0.0,
                at: AnimInstant::now(),
            }; VELOCITY_HISTORY_SIZE],
            count: 0,
            write_pos: 0,
        }
    }

    /// Records `velocity` as the average speed the finger held for the `dt`
    /// seconds that ended at `at`.
    ///
    /// The oldest sample is overwritten once the buffer is full.
    #[inline]
    pub(crate) fn push(&mut self, velocity: Vec2d, dt: f32, at: AnimInstant) {
        self.samples[self.write_pos] = VelocitySample {
            velocity,
            dt: dt.max(MIN_MOVE_DT),
            at,
        };
        self.write_pos = (self.write_pos + 1) % VELOCITY_HISTORY_SIZE;
        if self.count < VELOCITY_HISTORY_SIZE {
            self.count += 1;
        }
    }

    /// Returns the speed the finger was moving at as of `now`: the travel
    /// recorded within the last [`VELOCITY_HORIZON_S`] seconds divided by the
    /// time that travel took.
    ///
    /// Returns zero once nothing inside the horizon is left, which is exactly
    /// the case of a finger that came to rest before it was lifted.
    pub(crate) fn velocity_at(&self, now: AnimInstant) -> Vec2d {
        let mut travel = Vec2d::ZERO;
        let mut elapsed = 0.0f32;

        for age_rank in 0..self.count {
            let idx =
                (self.write_pos + VELOCITY_HISTORY_SIZE - 1 - age_rank) % VELOCITY_HISTORY_SIZE;
            let sample = self.samples[idx];
            if now.duration_since(sample.at).as_secs_f32() > VELOCITY_HORIZON_S {
                break;
            }
            travel.x += sample.velocity.x * sample.dt;
            travel.y += sample.velocity.y * sample.dt;
            elapsed += sample.dt;
        }

        if elapsed <= 0.0 {
            return Vec2d::ZERO;
        }
        Vec2d {
            x: travel.x / elapsed,
            y: travel.y / elapsed,
        }
    }

    /// Forgets every sample, so the next query starts from a clean gesture.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.count = 0;
        self.write_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Duration;
    use std::time::Instant;

    use aimer_attribute::position::Vec2d;
    use aimer_utils::AnimInstant;

    use super::VelocityHistory;
    use crate::scrollable::constants::VELOCITY_SAMPLE_MIN_DT;
    use crate::scrollable::controller::ScrollState;

    /// One 60 Hz host frame, the cadence a native touch stream is delivered on.
    const FRAME: Duration = Duration::from_micros(16_667);
    /// The same frame expressed in seconds, for building samples by hand.
    const FRAME_S: f32 = 1.0 / 60.0;

    /// The finger from the issue report: it crosses the viewport fast, then
    /// eases down over the last third of the gesture until it is standing
    /// still. Values are the per-frame `y` deltas of the logged positions.
    const COASTS_TO_A_STOP: [f32; 19] = [
        -10.33, -11.33, -11.0, -10.0, -9.33, -9.0, -7.67, -5.0, -3.67, -2.0, -1.67, -1.33, -0.67,
        -0.67, 0.0, -0.33, 0.0, 0.0, 1.0,
    ];

    /// Replays `deltas` as one pointer move per host frame and returns the
    /// engine together with the instant of the last move.
    fn drag(deltas: &[f32]) -> (ScrollState, AnimInstant) {
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        let mut at = AnimInstant::now();
        for dy in deltas {
            at += FRAME;
            if let Some((velocity, dt)) = ctrl.accumulate_drag_velocity(0.0, *dy, at) {
                ctrl.push_velocity(
                    Vec2d {
                        x: 0.0,
                        y: velocity.y,
                    },
                    dt,
                    at,
                );
            }
        }
        (ctrl, at)
    }

    /// The speed the engine would launch its fling with if the finger were
    /// lifted at `release`.
    fn release_velocity(ctrl: &ScrollState, release: AnimInstant) -> Vec2d {
        if let Some((velocity, dt)) = ctrl.flush_drag_velocity(release) {
            ctrl.push_velocity(
                Vec2d {
                    x: 0.0,
                    y: velocity.y,
                },
                dt,
                release,
            );
        }
        ctrl.smoothed_velocity(release)
    }

    #[test]
    fn a_history_with_no_samples_reports_no_motion() {
        let history = VelocityHistory::new();
        assert_eq!(history.velocity_at(AnimInstant::now()).y, 0.0);
    }

    #[test]
    fn a_steady_drag_is_released_at_the_speed_it_was_dragged() {
        let mut history = VelocityHistory::new();
        let mut at = AnimInstant::now();
        for _ in 0..6 {
            at += FRAME;
            history.push(Vec2d { x: 0.0, y: -5.0 }, FRAME_S, at);
        }

        assert!((history.velocity_at(at).y + 5.0).abs() < 1e-3);
    }

    // A velocity describes a moment, not a whole gesture: once every recorded
    // sample is older than the horizon there is no evidence the finger was
    // still moving, so the release must not inherit the speed of a burst that
    // has long since ended.
    #[test]
    fn samples_older_than_the_horizon_no_longer_describe_the_finger() {
        let mut history = VelocityHistory::new();
        let mut at = AnimInstant::now();
        for _ in 0..6 {
            at += FRAME;
            history.push(Vec2d { x: 0.0, y: -20.0 }, FRAME_S, at);
        }

        let much_later = at + Duration::from_millis(200);
        assert_eq!(history.velocity_at(much_later).y, 0.0);
    }

    // The slow tail of a gesture must outweigh its fast start, even though the
    // fast start contributed more samples: the average is taken over travel
    // per unit of time, not per sample.
    #[test]
    fn a_slow_tail_outweighs_the_fast_start_it_followed() {
        let mut history = VelocityHistory::new();
        let mut at = AnimInstant::now();
        // A dense burst of fast, very short slices…
        for _ in 0..8 {
            at += Duration::from_micros(2_000);
            history.push(Vec2d { x: 0.0, y: -30.0 }, 0.002, at);
        }
        // …followed by one long, slow slice that ends the gesture.
        at += Duration::from_millis(60);
        history.push(Vec2d { x: 0.0, y: -1.0 }, 0.06, at);

        // A plain mean over the samples would report ≈ -26.8 px/frame, because
        // the burst simply contributed more of them.
        let released = history.velocity_at(at).y;
        assert!(
            released > -12.0,
            "the long ending slice must weigh more than the dense burst, got {released}"
        );
    }

    // The reported bug: a drag that decelerates to a standstill and is only
    // then lifted still flung the content, because the release speed was read
    // from a fixed count of untimed samples that still held the fast start of
    // the same gesture.
    #[test]
    fn a_drag_that_coasts_to_a_stop_is_released_without_momentum() {
        let (ctrl, last_move) = drag(&COASTS_TO_A_STOP);

        // The finger rests on the glass for a moment before it is lifted, so
        // the platform stops reporting moves entirely.
        let release = last_move + Duration::from_millis(120);

        // Not merely small: the whole swing has fallen out of the horizon and
        // the closing slice measured no travel at all.
        let velocity = release_velocity(&ctrl, release);
        assert_eq!(
            velocity.y, 0.0,
            "a finger brought to a stop must release at rest"
        );
    }

    // The counterpart contract: a flick that was still moving when it was
    // lifted keeps every bit of its speed, so the fix cannot be "never fling".
    #[test]
    fn a_flick_that_never_slowed_keeps_its_speed_at_release() {
        let steady = [-10.0f32; 12];
        let (ctrl, last_move) = drag(&steady);

        let velocity = release_velocity(&ctrl, last_move + Duration::from_millis(4));
        // 10 px per 60 Hz frame is 5 px per 120 Hz frame.
        assert!(
            (velocity.y + 5.0).abs() < 0.5,
            "a live flick must keep its speed, got {}",
            velocity.y
        );
    }

    // Coalesced delivery: a burst of moves that share one instant is folded
    // into the sampling accumulator instead of emitting a sample, so the tail
    // of a gesture can end without ever reaching the history. Releasing must
    // still account for that travel rather than fall back to the last sample
    // the fast part of the gesture left behind.
    #[test]
    fn travel_still_held_by_the_accumulator_counts_at_release() {
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        let at = AnimInstant::now();

        // The first move of a gesture emits a sample; the two that share its
        // instant are only folded into the accumulator.
        ctrl.accumulate_drag_velocity(0.0, -1.0, at);
        assert!(ctrl.accumulate_drag_velocity(0.0, -4.0, at).is_none());
        assert!(ctrl.accumulate_drag_velocity(0.0, -4.0, at).is_none());

        let release = at + Duration::from_secs_f32(FRAME_S);
        let (velocity, dt) = ctrl
            .flush_drag_velocity(release)
            .expect("the residual spans a full sampling slice");

        // 8 px over one 60 Hz frame is 4 px per 120 Hz frame.
        assert!((velocity.y + 4.0).abs() < 1e-2, "got {}", velocity.y);
        assert!((dt - FRAME_S).abs() < 1e-3);
    }

    // A release that lands within the same sampling slice as the last move
    // carries a sub-pixel remainder over a near-zero `dt`. Turning that into a
    // sample would divide noise by ~0 and launch an enormous fling, so the
    // remainder is left where it is.
    #[test]
    fn a_sub_slice_remainder_is_not_flushed_as_a_spike() {
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        let at = AnimInstant::now();
        ctrl.accumulate_drag_velocity(0.0, -10.0, at);
        ctrl.accumulate_drag_velocity(0.0, -0.2, at);

        let release = at + Duration::from_secs_f32(VELOCITY_SAMPLE_MIN_DT * 0.5);
        assert!(ctrl.flush_drag_velocity(release).is_none());
    }

    // A finger that never moved again after its last reported move is at rest,
    // whatever it was doing before: the flushed slice reports zero travel over
    // the whole waiting time.
    #[test]
    fn a_finger_resting_before_the_lift_flushes_a_standstill() {
        let (ctrl, last_move) = drag(&[-20.0, -20.0, -20.0]);

        let rest = Duration::from_millis(80);
        let (velocity, dt) = ctrl
            .flush_drag_velocity(last_move + rest)
            .expect("a rest longer than a sampling slice is measurable");

        assert_eq!(velocity.y, 0.0);
        assert!((dt - rest.as_secs_f32()).abs() < 1e-3);
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_velocity_history() {
        const MEASURED: usize = 4_096;
        const WARMUP: usize = 512;
        const ROUNDS: usize = 7;

        for (name, count) in [("recent-1", 1), ("recent-8", 8), ("recent-24", 24)] {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                let now = AnimInstant::now();
                let mut history = VelocityHistory::new();
                for index in 0..count {
                    let age = Duration::from_micros((count - index) as u64 * 2_000);
                    history.push(
                        Vec2d {
                            x: index as f32,
                            y: -(index as f32),
                        },
                        0.002,
                        now - age,
                    );
                }

                for _ in 0..WARMUP {
                    checksum = black_box(checksum + history.velocity_at(now).y);
                }

                let start = Instant::now();
                for _ in 0..MEASURED {
                    checksum = black_box(checksum + history.velocity_at(now).y);
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }
}
