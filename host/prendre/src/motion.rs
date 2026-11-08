//! How the injected mouse moves: continuously, and never by teleporting.
//!
//! A real mouse crosses the pixels between two points, and whatever lies on the
//! way sees it pass: a hover starts and ends, a tooltip timer is cut short. It
//! also cannot change direction on the spot — told to go somewhere else while it
//! is moving, a hand swings round in an arc. [`Motion`] is that: a pointer with
//! a position and a velocity that is steered toward a target. Its heading turns
//! at a limited rate, it slows for a sharp turn, and it brakes to stop exactly on
//! the target, so a new target sent mid-flight bends the path instead of
//! snapping it.
//!
//! [`Motion`] is a pure model, stepped by whoever owns the clock. [`Pointer`]
//! is that owner: it shares one motion between every connection and walks it on a
//! thread of its own, which is what lets a command arrive while the pointer is
//! still on its way.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::command::MAX_GLIDE_MILLIS as MAX_MILLIS;
use crate::model::Modifiers;

/// How fast the heading may turn, in radians a second. At a thousand pixels a
/// second that is an arc about 140 pixels across.
pub(crate) const MAX_TURN: f32 = 7.0;
/// How fast it speeds up and how fast it may slow down, in pixels a second a second.
pub(crate) const ACCEL: f32 = 9000.0;
pub(crate) const DECEL: f32 = 13000.0;
/// Near the target the speed is proportional to the distance left, so the pointer
/// settles into it, instead of arriving at the speed a braking curve gives at the
/// last step. Per second: 20 is a time constant of 50 ms.
const APPROACH_GAIN: f32 = 20.0;
/// Where the braking curve gives way to that approach. A linear approach asks for
/// a deceleration of `gain² × distance`, so only inside this distance is it within
/// what the pointer can do; the curve outside it is chosen to meet it smoothly.
const APPROACH_FROM: f32 = DECEL / (APPROACH_GAIN * APPROACH_GAIN);
/// Closer than this is there: the snap onto the target is less than a pixel.
const ARRIVE_WITHIN: f32 = 0.4;
/// The fraction of its speed it keeps while pointing the wrong way entirely.
const CORNER_FLOOR: f32 = 0.35;
/// The shortest and longest a glide takes when the caller does not say, and how
/// much each pixel of distance adds to it.
const MIN_AUTO_MILLIS: f32 = 120.0;
const MAX_AUTO_MILLIS: f32 = 800.0;
const MILLIS_PER_PIXEL: f32 = 0.45;
/// An eased glide peaks this many times faster than its average speed.
const PEAK: f32 = 1.5;
const MIN_CRUISE: f32 = 150.0;
const MAX_CRUISE: f32 = 8000.0;

/// How often the pointer thread takes a step: about once a frame at 60 Hz.
const TICK: Duration = Duration::from_millis(16);
/// How often a waiting thread looks up to see whether it should give up.
const POLL: Duration = Duration::from_millis(50);

/// The fastest the pointer may be going `distance` from its target and still stop
/// there without ever being asked to slow down faster than [`DECEL`].
fn brake_speed(distance: f32) -> f32 {
    if distance <= APPROACH_FROM {
        APPROACH_GAIN * distance
    } else {
        // `v² = 2·a·(d − d₀/2)`: constant deceleration, meeting the linear approach
        // at `d₀` with the same slope.
        (2.0 * DECEL * distance - DECEL * APPROACH_FROM).sqrt()
    }
}

/// The cruising speed for a glide of `distance` pixels that should take about
/// `millis`, or the time the distance deserves when it is not said.
fn cruise_for(distance: f32, millis: Option<u32>) -> f32 {
    let total = match millis {
        Some(millis) => millis.clamp(1, MAX_MILLIS) as f32,
        None => (MIN_AUTO_MILLIS + distance * MILLIS_PER_PIXEL).clamp(MIN_AUTO_MILLIS, MAX_AUTO_MILLIS),
    };
    (distance / (total / 1000.0) * PEAK).clamp(MIN_CRUISE, MAX_CRUISE)
}

/// A pointer with a place, a velocity and somewhere it is heading.
#[derive(Debug, Default)]
pub(crate) struct Motion {
    pos: Option<(f32, f32)>,
    vel: (f32, f32),
    target: Option<(f32, f32)>,
    cruise: f32,
    modifiers: Modifiers,
}

impl Motion {
    /// Puts the pointer at `pos`, or nowhere, at rest: a teleport, which ends any
    /// glide in progress.
    pub(crate) fn place(&mut self, pos: Option<(f32, f32)>) {
        self.pos = pos;
        self.vel = (0.0, 0.0);
        self.target = None;
    }

    /// Sets off toward `to`, taking about `millis` (or the time the distance
    /// deserves). Called mid-flight it retargets: the velocity it has is kept
    /// and steered round, not discarded.
    ///
    /// With nowhere to start from there is nothing to cross, so the pointer is
    /// put at `to` and that position is returned for the caller to announce.
    pub(crate) fn glide(&mut self, to: (f32, f32), millis: Option<u32>) -> Option<(f32, f32)> {
        let Some(from) = self.pos else {
            self.place(Some(to));
            return Some(to);
        };
        let distance = (to.0 - from.0).hypot(to.1 - from.1);
        if distance < 0.5 {
            // Already there: nothing to cross, whatever it was heading for.
            self.vel = (0.0, 0.0);
            self.target = None;
            return None;
        }
        self.target = Some(to);
        self.cruise = cruise_for(distance, millis);
        None
    }

    /// Where the pointer is, if it is anywhere.
    pub(crate) fn position(&self) -> Option<(f32, f32)> {
        self.pos
    }

    #[cfg(test)]
    pub(crate) fn velocity(&self) -> (f32, f32) {
        self.vel
    }

    /// Whether it is still on its way.
    pub(crate) fn is_moving(&self) -> bool {
        self.target.is_some()
    }

    /// Stops where it is.
    fn cancel(&mut self) {
        self.vel = (0.0, 0.0);
        self.target = None;
    }

    /// Advances by `dt` seconds and returns where the pointer now is, or `None`
    /// when it is not going anywhere. The step that arrives lands exactly on the
    /// target and stops.
    pub(crate) fn step(&mut self, dt: f32) -> Option<(f32, f32)> {
        let (pos, target) = (self.pos?, self.target?);
        let to = (target.0 - pos.0, target.1 - pos.1);
        let distance = to.0.hypot(to.1);
        if distance < ARRIVE_WITHIN {
            return Some(self.arrive(target));
        }
        let aim = (to.0 / distance, to.1 / distance);

        // Turn toward the target, no faster than a hand can.
        let speed = self.vel.0.hypot(self.vel.1);
        let heading = if speed > 1.0 { (self.vel.0 / speed, self.vel.1 / speed) } else { aim };
        let error = (heading.0 * aim.1 - heading.1 * aim.0).atan2(heading.0 * aim.0 + heading.1 * aim.1);
        let turn = error.clamp(-MAX_TURN * dt, MAX_TURN * dt);
        let (sin, cos) = turn.sin_cos();
        let heading = (heading.0 * cos - heading.1 * sin, heading.0 * sin + heading.1 * cos);

        // Speed: up to cruising, down to what lets it stop at the target, and
        // lower while it is still pointing the wrong way.
        let align = (error - turn).cos().max(0.0);
        let limit = self.cruise.min(brake_speed(distance));
        let want = limit * (CORNER_FLOOR + (1.0 - CORNER_FLOOR) * align);
        let speed = if want > speed { (speed + ACCEL * dt).min(want) } else { (speed - DECEL * dt).max(want) };

        if speed * dt >= distance {
            return Some(self.arrive(target));
        }
        self.vel = (heading.0 * speed, heading.1 * speed);
        let now = (pos.0 + self.vel.0 * dt, pos.1 + self.vel.1 * dt);
        self.pos = Some(now);
        Some(now)
    }

    fn arrive(&mut self, target: (f32, f32)) -> (f32, f32) {
        self.pos = Some(target);
        self.cancel();
        target
    }
}

/// The injected mouse, shared by every connection and walked on its own thread.
pub(crate) struct Pointer {
    motion: Mutex<Motion>,
    /// Signalled when the motion starts, changes or ends.
    changed: Condvar,
}

impl Pointer {
    pub(crate) fn new() -> Self {
        Self { motion: Mutex::new(Motion::default()), changed: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, Motion> {
        self.motion.lock().expect("pointer mutex poisoned")
    }

    /// Wakes whoever is waiting on, or for, the motion — the thread, and anything
    /// in [`Pointer::wait_settled`].
    pub(crate) fn wake(&self) {
        self.changed.notify_all();
    }

    /// Puts the pointer at `pos` (or nowhere) at once, ending any glide.
    pub(crate) fn place(&self, pos: Option<(f32, f32)>) {
        self.lock().place(pos);
        self.wake();
    }

    /// Stops where it is.
    pub(crate) fn cancel(&self) {
        self.lock().cancel();
        self.wake();
    }

    /// Sets the pointer gliding toward `to`; see [`Motion::glide`]. Returns at
    /// once, with the position to announce if there was nowhere to glide from.
    pub(crate) fn glide(&self, to: (f32, f32), millis: Option<u32>, modifiers: Modifiers) -> Option<(f32, f32)> {
        let mut motion = self.lock();
        motion.modifiers = modifiers;
        let jump = motion.glide(to, millis);
        drop(motion);
        self.wake();
        jump
    }

    /// Waits until the pointer has stopped, and says whether it did: `false` when
    /// `timeout` ran out or `stop` was raised first.
    pub(crate) fn wait_settled(&self, stop: &AtomicBool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut motion = self.lock();
        while motion.is_moving() {
            let now = Instant::now();
            if stop.load(Ordering::Acquire) || now >= deadline {
                return false;
            }
            motion = self.changed.wait_timeout(motion, (deadline - now).min(POLL)).expect("pointer mutex poisoned").0;
        }
        true
    }

    /// The body of the pointer's thread: while the pointer is on its way, steps it
    /// once a tick and hands each position to `emit`, until `stop` is raised.
    ///
    /// `emit` runs while the motion is held, so whoever waits for the pointer to
    /// arrive wakes only after the move that arrived has been handed over. It
    /// returns whether the app took the move; one that did not ends the glide.
    pub(crate) fn drive(&self, stop: &AtomicBool, emit: impl Fn((f32, f32), Modifiers) -> bool) {
        let mut last: Option<Instant> = None;
        let mut next = Instant::now();
        loop {
            let mut motion = self.lock();
            while !motion.is_moving() {
                if stop.load(Ordering::Acquire) {
                    return;
                }
                last = None;
                motion = self.changed.wait_timeout(motion, POLL).expect("pointer mutex poisoned").0;
            }
            if stop.load(Ordering::Acquire) {
                return;
            }
            let now = Instant::now();
            // The time actually elapsed, not the tick that was hoped for, so a late
            // wake-up costs smoothness and not speed.
            let dt = last.map_or(TICK, |last| now - last).clamp(Duration::from_millis(1), Duration::from_millis(50));
            last = Some(now);
            if let Some(at) = motion.step(dt.as_secs_f32())
                && !emit(at, motion.modifiers)
            {
                motion.cancel();
            }
            drop(motion);
            self.wake();

            // The next tick is due a tick after the last was, not a tick after this
            // one finished: sleeps overshoot, and the overshoot must not accumulate.
            next += TICK;
            let now = Instant::now();
            match next.checked_duration_since(now) {
                Some(wait) => std::thread::sleep(wait),
                None => next = now,
            }
        }
    }
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
