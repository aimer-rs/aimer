//! A scroll shaped like a trackpad swipe.
//!
//! A mouse wheel sends a few big pulses, and the app smooths those into a quick
//! burst. A trackpad sends a steady stream of small pixel deltas, faster than
//! one every twenty milliseconds, that swells and fades with the fingers — and
//! the app treats that stream differently: it scrolls by exactly what the
//! fingers travelled, with the gesture's own start and end. This plans such a
//! stream. The server walks it.
//!
//! The deltas must not repeat: the receiving side calls a run of equal deltas a
//! wheel. An eased swipe never repeats one, and its cadence is held under the
//! twenty milliseconds at which a stream stops looking like a trackpad.

use std::time::Duration;

use crate::command::MAX_GLIDE_MILLIS as MAX_MILLIS;

/// How often a step is sent. Under twenty milliseconds, with room to spare for a
/// late wake-up, which is what the receiving side needs to call it a trackpad.
const STEP: Duration = Duration::from_millis(12);
/// The shortest and longest a swipe takes when the caller does not say, and how
/// much each pixel adds to it.
///
/// A deliberate scroll, not a flick: about 1250 pixels a second on average, with
/// the eased peak half as fast again, so 120 pixels take 0.4 s and 700 take 0.9 s.
const MIN_AUTO_MILLIS: f32 = 300.0;
const MAX_AUTO_MILLIS: f32 = 2400.0;
const MILLIS_PER_PIXEL: f32 = 0.8;
/// Less than this is no scroll at all.
const MIN_DISTANCE: f32 = 0.5;

/// The pixel deltas of a swipe, in order, and how long to wait between them.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    /// The `(dx, dy)` of each step. They add up to exactly the distance asked for.
    pub(crate) steps: Vec<(f32, f32)>,
    /// How long to wait after each step.
    pub(crate) interval: Duration,
}

/// Plans a swipe of `(dx, dy)` pixels that takes about `millis`, or as long as the
/// distance deserves. A distance under a pixel plans nothing.
pub(crate) fn plan(dx: f32, dy: f32, millis: Option<u32>) -> Plan {
    let distance = dx.hypot(dy);
    if distance < MIN_DISTANCE {
        return Plan { steps: Vec::new(), interval: Duration::ZERO };
    }
    let total = match millis {
        Some(millis) => millis.clamp(1, MAX_MILLIS) as f32,
        None => (MIN_AUTO_MILLIS + distance * MILLIS_PER_PIXEL).clamp(MIN_AUTO_MILLIS, MAX_AUTO_MILLIS),
    };
    let count = ((total / STEP.as_millis() as f32).round() as usize).max(3);
    // Where the swipe has got to after each step, and the steps are the gaps. The
    // last position is the distance itself, so the steps sum to it exactly.
    let at = |i: usize| {
        if i == count {
            return (dx, dy);
        }
        let t = ease(i as f32 / count as f32);
        (dx * t, dy * t)
    };
    let steps = (1..=count)
        .map(|i| {
            let (now, before) = (at(i), at(i - 1));
            (now.0 - before.0, now.1 - before.1)
        })
        .collect();
    Plan { steps, interval: Duration::from_secs_f32(total / count as f32 / 1000.0) }
}

/// Slow out of the start, fastest in the middle, slow into the end: a finger
/// that lands, sweeps and lifts.
fn ease(t: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::PI * t).cos()
}

#[cfg(test)]
#[path = "trackpad_tests.rs"]
mod tests;
