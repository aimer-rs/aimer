//! Frame-rate independent integration of the bouncy overscroll recovery.
//!
//! The recovery is a mass-spring-damper pulling the content back to its edge:
//!
//! ```text
//! x'' = -k·x - c·x'      k = stiffness, c = 2·ζ·√k, x = distance past the edge
//! ```
//!
//! Advancing that system with one explicit step per rendered frame is only
//! *conditionally* stable: the damping term becomes `x'·(1 − c·dt)`, which
//! flips sign as soon as `dt > 1/c`. With the scrollable's stiffness that
//! threshold sits at ≈11 ms — **between** a 120 Hz and a 60 Hz frame. So the
//! very same "critically damped, no overshoot" spring settles cleanly on a
//! native ProMotion display and visibly ping-pongs outward/inward with
//! shrinking amplitude in a browser tab, which renders at the compositor's
//! ~16.7 ms cadence and cannot be pushed faster.
//!
//! This module therefore decouples the integration step from the frame: a
//! frame is split into [`SPRING_SUBSTEP_S`]-long sub-steps, so the browser's
//! long frame walks the exact same trajectory native already walks — one
//! sub-step per 120 Hz frame, two or three per browser frame — instead of
//! taking one oversized, unstable leap.

use crate::scrollable::constants::{SPRING_DAMPING_RATIO, SPRING_STIFFNESS, SPRING_SUBSTEP_S};

/// Advance the scrollable's overscroll spring by `dt` seconds.
///
/// `displacement` is the signed distance the content is past its edge and
/// `velocity` the spring's own velocity (px/s). Returns the pair after `dt`,
/// using [`SPRING_STIFFNESS`] and [`SPRING_DAMPING_RATIO`].
///
/// # Examples
///
/// ```ignore
/// let (x, v) = advance_overscroll_spring(40.0, 0.0, 1.0 / 60.0);
/// assert!(x < 40.0 && x > 0.0);
/// ```
#[inline]
pub(crate) fn advance_overscroll_spring(displacement: f32, velocity: f32, dt: f32) -> (f32, f32) {
    advance_damped_spring(
        displacement,
        velocity,
        SPRING_STIFFNESS,
        SPRING_DAMPING_RATIO,
        dt,
    )
}

/// Advance `x'' = -k·x - 2·ζ·√k·x'` by `dt` seconds and return the new
/// `(displacement, velocity)`.
///
/// The step is integrated semi-implicitly in fixed [`SPRING_SUBSTEP_S`]
/// sub-steps, so the result depends on the elapsed *time* and not on how many
/// frames the host managed to render in it. A non-positive `dt` or `stiffness`
/// leaves the state untouched.
///
/// # Examples
///
/// ```ignore
/// // A critically damped spring returns to rest without ever crossing zero.
/// let (mut x, mut v) = (100.0_f32, 0.0_f32);
/// for _ in 0..60 {
///     (x, v) = advance_damped_spring(x, v, 2000.0, 1.0, 1.0 / 60.0);
///     assert!(x >= 0.0);
/// }
/// assert!(x < 0.5);
/// ```
pub(crate) fn advance_damped_spring(
    displacement: f32,
    velocity: f32,
    stiffness: f32,
    damping_ratio: f32,
    dt: f32,
) -> (f32, f32) {
    if dt <= 0.0 || stiffness <= 0.0 {
        return (displacement, velocity);
    }

    let damping_coeff = 2.0 * damping_ratio.max(0.0) * stiffness.sqrt();
    // Ceil, so a sub-step is never *longer* than the stable bound; the frame is
    // then split evenly to land exactly on `dt`.
    let steps = (dt / SPRING_SUBSTEP_S).ceil().max(1.0);
    let h = dt / steps;
    let mut x = displacement;
    let mut v = velocity;

    for _ in 0..steps as u32 {
        v += (-stiffness * x - damping_coeff * v) * h;
        x += v * h;
    }

    (x, v)
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;
    use std::time::Instant;

    use super::*;
    use crate::scrollable::constants::SNAP_EPSILON;

    /// Tolerance for `f32` round-off in an otherwise monotone recovery.
    const NOISE: f32 = 1.0e-3;

    /// Run the overscroll spring for `seconds` in steps of `dt`, reporting the
    /// final displacement and the largest *outward* move any single step made.
    fn simulate(start: f32, dt: f32, seconds: f32) -> (f32, f32) {
        let steps = (seconds / dt).round() as usize;
        let mut x = start;
        let mut v = 0.0;
        let mut worst_outward = 0.0_f32;
        for _ in 0..steps {
            let (nx, nv) = advance_overscroll_spring(x, v, dt);
            worst_outward = worst_outward.max(nx.abs() - x.abs());
            x = nx;
            v = nv;
        }
        (x, worst_outward)
    }

    // The reported web bug: on a 60 Hz frame a single explicit step makes the
    // damping term `(1 - c·dt)` negative, so the spring velocity flips sign
    // every frame and the content ping-pongs outward/inward with shrinking
    // amplitude instead of sliding home once.
    #[test]
    fn recovery_at_sixty_hertz_never_moves_back_outward() {
        let (x, worst_outward) = simulate(100.0, 1.0 / 60.0, 1.0);

        assert!(
            worst_outward <= NOISE,
            "a step pulled the content {worst_outward} px further from the edge"
        );
        assert!(x >= -NOISE, "the spring must not cross the edge, got {x}");
        assert!(x < SNAP_EPSILON, "the spring must settle, got {x}");
    }

    #[test]
    fn recovery_at_thirty_hertz_never_moves_back_outward() {
        let (x, worst_outward) = simulate(100.0, 1.0 / 30.0, 1.0);

        assert!(
            worst_outward <= NOISE,
            "a step pulled the content {worst_outward} px further from the edge"
        );
        assert!(x >= -NOISE, "the spring must not cross the edge, got {x}");
    }

    // Same wall-clock time must put the content in the same place, whatever
    // the host's frame cadence is.
    #[test]
    fn the_recovery_path_is_frame_rate_independent() {
        // 0.1 s is a whole number of frames at each of these rates, so the
        // three runs cover exactly the same wall-clock span.
        let at_120 = simulate(100.0, 1.0 / 120.0, 0.1).0;
        let at_60 = simulate(100.0, 1.0 / 60.0, 0.1).0;
        let at_30 = simulate(100.0, 1.0 / 30.0, 0.1).0;

        assert!(
            (at_120 - at_60).abs() < 0.5,
            "120 Hz gave {at_120}, 60 Hz gave {at_60}"
        );
        assert!(
            (at_120 - at_30).abs() < 0.5,
            "120 Hz gave {at_120}, 30 Hz gave {at_30}"
        );
    }

    // A frame as long as `MAX_FRAME_DT` (a stutter, or a tab regaining focus)
    // must still be a plain move toward the edge.
    #[test]
    fn a_stuttering_frame_still_recovers_toward_the_edge() {
        let (x, v) = advance_overscroll_spring(100.0, 0.0, 0.05);

        assert!(x.abs() < 100.0, "got {x}");
        assert!(x >= -SNAP_EPSILON, "got {x}");
        // A spring released from rest can never move faster than ω·x0.
        assert!(
            v.abs() <= SPRING_STIFFNESS.sqrt() * 100.0,
            "the step may not inject energy, got {v}"
        );
    }

    #[test]
    fn a_negative_stretch_recovers_upward_symmetrically() {
        let (x, _) = advance_overscroll_spring(-100.0, 0.0, 1.0 / 60.0);
        let (mirrored, _) = advance_overscroll_spring(100.0, 0.0, 1.0 / 60.0);

        assert!((x + mirrored).abs() < 1.0e-3, "{x} vs {mirrored}");
    }

    #[test]
    fn a_zero_step_leaves_the_state_untouched() {
        assert_eq!(advance_overscroll_spring(40.0, -7.0, 0.0), (40.0, -7.0));
    }

    #[test]
    fn an_underdamped_spring_overshoots_the_edge() {
        let mut x = 100.0_f32;
        let mut v = 0.0_f32;
        let mut crossed = false;
        for _ in 0..60 {
            (x, v) = advance_damped_spring(x, v, 2_000.0, 0.2, 1.0 / 60.0);
            crossed |= x < 0.0;
        }

        assert!(crossed, "ζ = 0.2 must overshoot the edge at least once");
    }

    #[test]
    fn an_overdamped_spring_never_overshoots_the_edge() {
        let mut x = 100.0_f32;
        let mut v = 0.0_f32;
        for _ in 0..120 {
            (x, v) = advance_damped_spring(x, v, 2_000.0, 2.5, 1.0 / 60.0);
            assert!(x >= 0.0, "ζ = 2.5 crossed the edge: {x}");
        }
        assert!(x < 100.0);
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_spring_integration() {
        const MEASURED: usize = 1_024;
        const WARMUP: usize = 128;
        const ROUNDS: usize = 7;

        let cases = [
            ("120hz-frame", 1.0 / 120.0),
            ("60hz-frame", 1.0 / 60.0),
            ("30hz-frame", 1.0 / 30.0),
            ("stutter-frame", 0.05),
        ];

        for (name, dt) in cases {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                let mut x = 100.0;
                let mut v = 0.0;
                for _ in 0..WARMUP {
                    (x, v) = black_box(advance_overscroll_spring(x, v, dt));
                }

                let start = Instant::now();
                for _ in 0..MEASURED {
                    (x, v) = black_box(advance_overscroll_spring(x, v, dt));
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
                checksum = black_box(checksum + x + v);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }
}
