//! Recognizes the two-finger trackpad swipe that pops a page.
//!
//! A trackpad delivers a two-finger drag as scroll events: a stream of pixel
//! deltas, with a phase and a flag that says the fingers are still on the
//! surface. After the fingers lift the platform keeps sending deltas for the
//! glide that follows, with that flag off. This is a plain state machine fed that
//! stream, answering with what the host should do next; like
//! [`EdgeSwipe`](crate::swipe_back) it owns no tree, clock or widget.
//!
//! Letting go does not end the swipe. The platform goes on sending deltas for the
//! glide that follows the fingers, and they count: a swipe that lifts at a fifth
//! of the way and glides past the threshold completes, as it does in Chrome, which
//! decides at the end of the inertia and not at the lift. Only a flick, which is
//! known to complete or to be thrown back, and a lift with no speed, which has no
//! glide to wait for, are decided at once.
//!
//! A gesture is judged once, when it has travelled past a slop: rightward and
//! clearly more sideways than down, it is a swipe; anything else is the page's
//! to scroll, and stays the page's until the fingers lift. The page sees every
//! event first and keeps the ones it uses, so a horizontal scroller that can still
//! move keeps its gesture, and one at its edge hands it up, which is the whole of
//! the arbitration. A cursor wheel, which never has fingers on it, is not a swipe.
//!
//! Rightward is a positive `x` delta: fingers moving right with natural
//! scrolling, the way the platform reports it. A platform that does not report
//! when the fingers are down has no direct manipulation to recognize.

use aimer_attribute::Vec2d;
use aimer_events::element::TouchPhase;

/// Thresholds of the trackpad swipe, in logical pixels and seconds.
///
/// These are first guesses, not tuned on hardware.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TrackpadConfig {
    /// Travel before a gesture is judged a swipe or not.
    pub slop: f32,
    /// How much more sideways than down a gesture must travel to be a swipe.
    pub ratio: f32,
    /// Fraction of the travel a full pop takes past which letting go completes it.
    pub commit_fraction: f32,
    /// Rightward speed, in pixels a second, at which letting go completes the pop
    /// at any distance, and leftward speed at which it cancels at any distance.
    pub fling_velocity: f32,
    /// How long after the fingers lift the glide that follows is swallowed.
    pub momentum_grace: f32,
    /// The slowest lift, in pixels a second, that a glide is expected to follow.
    /// Slower than this the fingers rested, and nothing is waited for.
    pub min_glide_velocity: f32,
    /// How long, at the most, a glide moves the swipe after the fingers lift.
    /// Chromium cuts inertia off at 0.3 s on Windows for the same reason: a glide
    /// that goes on and on is not a reason to wait on the user.
    pub glide_cap: f32,
    /// How long without a delta ends a glide whose end was never reported.
    pub glide_silence: f32,
}

impl TrackpadConfig {
    /// How much of the host's width the fingers' travel takes to pop the page all
    /// the way. A trackpad's pixels run faster than a finger's on glass, so a
    /// swipe does not have to cross the whole host.
    pub(crate) const TRAVEL_FRACTION: f32 = 0.6;

    /// How far back a flick's speed is measured.
    const VELOCITY_WINDOW: f32 = 0.12;
}

impl Default for TrackpadConfig {
    fn default() -> Self {
        Self {
            slop: 14.0,
            ratio: 1.75,
            commit_fraction: 0.5,
            fling_velocity: 600.0,
            momentum_grace: 0.35,
            min_glide_velocity: 50.0,
            glide_cap: 0.3,
            glide_silence: 0.12,
        }
    }
}

/// What the host does with the event that just arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TrackpadOutput {
    /// Nothing: the page has the event, and keeps it.
    None,
    /// The gesture is a swipe: start the pop, then drag it to
    /// [`TrackpadSwipe::progress`]. If the host cannot, it calls
    /// [`TrackpadSwipe::decline`].
    Begin,
    /// Drag the pop to this fraction of the way, `0.0..=1.0`.
    Update(f32),
    /// The fingers lifted: finish the pop, completing it if `commit`.
    Release { commit: bool },
}

#[derive(Debug)]
enum State {
    /// No gesture, or one that has ended.
    Idle,
    /// A gesture being watched for a verdict, by the travel so far.
    Judging { dx: f32, dy: f32 },
    /// A swipe: how far across the travel it is, and the recent travel for its speed.
    Swiping { dx: f32, samples: Vec<(f32, f32)> },
    /// The fingers have lifted and the swipe goes on with the glide that follows: how
    /// far across the travel it is, when it lifted and when it last moved.
    Gliding { dx: f32, since: f32, last: f32 },
    /// A gesture that is the page's, until the fingers lift.
    Ignoring,
    /// A swipe that was released at `at`, whose glide is still arriving.
    Released { at: f32 },
}

/// The trackpad swipe's recognizer.
#[derive(Debug)]
pub(crate) struct TrackpadSwipe {
    config: TrackpadConfig,
    state: State,
}

impl TrackpadSwipe {
    pub(crate) fn new(config: TrackpadConfig) -> Self {
        Self { config, state: State::Idle }
    }

    /// Whether a swipe is in progress, with the fingers on the surface or gliding
    /// on after they lift.
    pub(crate) fn is_swiping(&self) -> bool {
        matches!(self.state, State::Swiping { .. } | State::Gliding { .. })
    }

    /// Whether the host should take scroll events from the page at `now`: while
    /// swiping, and for a moment after the release, for the glide that follows
    /// and is the swipe's, not the page's.
    pub(crate) fn holds(&self, now: f32) -> bool {
        match self.state {
            State::Swiping { .. } | State::Gliding { .. } => true,
            State::Released { at } => now - at < self.config.momentum_grace,
            _ => false,
        }
    }

    /// How far across `width` the swipe is, from `0.0` to `1.0`.
    pub(crate) fn progress(&self, width: f32) -> f32 {
        let (State::Swiping { dx, .. } | State::Gliding { dx, .. }) = &self.state else { return 0.0 };
        let travel = Self::travel(width);
        if travel > 0.0 { (dx / travel).clamp(0.0, 1.0) } else { 0.0 }
    }

    fn travel(width: f32) -> f32 {
        if width.is_finite() && width > 0.0 { width * TrackpadConfig::TRAVEL_FRACTION } else { 0.0 }
    }

    /// The page used the gesture, so the rest of it is the page's: a swipe is only
    /// ever made of what the page left. Does nothing to a swipe already under way.
    pub(crate) fn yield_to_page(&mut self) {
        if matches!(self.state, State::Idle | State::Judging { .. } | State::Released { .. }) {
            self.state = State::Ignoring;
        }
    }

    /// The host could not start the pop, so the rest of this gesture is the page's.
    pub(crate) fn decline(&mut self) {
        if self.is_swiping() {
            self.state = State::Ignoring;
        }
    }

    /// Something other than the fingers ended the gesture: let a swipe go,
    /// without completing it.
    pub(crate) fn cancel(&mut self, now: f32) -> TrackpadOutput {
        match self.state {
            State::Swiping { .. } | State::Gliding { .. } => {
                self.state = State::Released { at: now };
                TrackpadOutput::Release { commit: false }
            }
            State::Judging { .. } | State::Ignoring => {
                self.state = State::Idle;
                TrackpadOutput::None
            }
            _ => TrackpadOutput::None,
        }
    }

    /// Takes one scroll event: `delta` pixels, in `phase`, with the fingers on the
    /// surface if `direct`, at `now` seconds, over a host `width` wide.
    pub(crate) fn scroll(&mut self, delta: Vec2d, phase: TouchPhase, direct: bool, now: f32, width: f32) -> TrackpadOutput {
        let delta = Vec2d {
            x: if delta.x.is_finite() { delta.x } else { 0.0 },
            y: if delta.y.is_finite() { delta.y } else { 0.0 },
        };
        let ends = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);

        if !direct {
            // The fingers are not there: a glide, or a wheel with no fingers at all.
            return match self.state {
                // The fingers lifted without saying so; this is the first of the glide.
                State::Swiping { .. } => {
                    self.advance(delta.x, width);
                    self.lift(now, width)
                }
                State::Gliding { .. } => {
                    self.advance(delta.x, width);
                    let State::Gliding { since, last, .. } = &mut self.state else { unreachable!() };
                    *last = now;
                    if ends || now - *since >= self.config.glide_cap {
                        self.finish_glide(now, width)
                    } else {
                        TrackpadOutput::Update(self.progress(width))
                    }
                }
                State::Judging { .. } | State::Ignoring => {
                    self.state = State::Idle;
                    TrackpadOutput::None
                }
                _ => TrackpadOutput::None,
            };
        }
        // A touch down while a glide is still going is another gesture: the glide is
        // over, where it had got to.
        if matches!(self.state, State::Gliding { .. }) {
            return self.finish_glide(now, width);
        }
        if matches!(phase, TouchPhase::Started) && !self.is_swiping() {
            self.state = State::Idle;
        }
        if matches!(self.state, State::Idle | State::Released { .. }) {
            self.state = State::Judging { dx: 0.0, dy: 0.0 };
        }

        match &mut self.state {
            State::Judging { dx, dy } => {
                *dx += delta.x;
                *dy += delta.y;
                let (dx, dy) = (*dx, *dy);
                if ends {
                    self.state = State::Idle;
                    return TrackpadOutput::None;
                }
                if dx.hypot(dy) <= self.config.slop {
                    return TrackpadOutput::None;
                }
                if dx > 0.0 && dx > self.config.ratio * dy.abs() {
                    let dx = dx.min(Self::travel(width));
                    self.state = State::Swiping { dx, samples: vec![(now, dx)] };
                    TrackpadOutput::Begin
                } else {
                    self.state = State::Ignoring;
                    TrackpadOutput::None
                }
            }
            State::Swiping { dx, samples } => {
                *dx = (*dx + delta.x).clamp(0.0, Self::travel(width));
                samples.push((now, delta.x));
                if matches!(phase, TouchPhase::Cancelled) {
                    return self.release(now, width, true);
                }
                if ends {
                    return self.lift(now, width);
                }
                TrackpadOutput::Update(self.progress(width))
            }
            State::Gliding { .. } => TrackpadOutput::None,
            State::Ignoring => {
                if ends {
                    self.state = State::Idle;
                }
                TrackpadOutput::None
            }
            State::Idle | State::Released { .. } => TrackpadOutput::None,
        }
    }

    /// Moves a swipe on by `dx`, kept within the travel.
    fn advance(&mut self, dx: f32, width: f32) {
        let travel = Self::travel(width);
        match &mut self.state {
            // A glide's travel is not part of the fingers' own speed, which is what a
            // flick is measured by, so it is not recorded as a sample.
            State::Swiping { dx: at, .. } | State::Gliding { dx: at, .. } => *at = (*at + dx).clamp(0.0, travel),
            _ => {}
        }
    }

    /// The fingers have lifted. A flick is decided now, and so is a lift that has
    /// no speed and so no glide to wait for; anything between goes on gliding, and
    /// is decided where the glide ends.
    fn lift(&mut self, now: f32, width: f32) -> TrackpadOutput {
        let velocity = self.velocity(now);
        let flick = velocity.abs() >= self.config.fling_velocity;
        if flick || velocity.abs() < self.config.min_glide_velocity {
            return self.release(now, width, false);
        }
        let State::Swiping { dx, .. } = &self.state else { return TrackpadOutput::None };
        self.state = State::Gliding { dx: *dx, since: now, last: now };
        TrackpadOutput::Update(self.progress(width))
    }

    /// Where the host asks, between events, whether a glide has run out: it is over
    /// once nothing has moved it for a moment, or it has gone on for too long, even
    /// though no end was reported.
    pub(crate) fn poll(&mut self, now: f32, width: f32) -> TrackpadOutput {
        let State::Gliding { since, last, .. } = self.state else { return TrackpadOutput::None };
        if now - last >= self.config.glide_silence || now - since >= self.config.glide_cap {
            return self.finish_glide(now, width);
        }
        TrackpadOutput::None
    }

    /// Ends a glide: complete the pop if it has carried the swipe far enough.
    fn finish_glide(&mut self, now: f32, width: f32) -> TrackpadOutput {
        let commit = self.progress(width) >= self.config.commit_fraction;
        self.state = State::Released { at: now };
        TrackpadOutput::Release { commit }
    }

    /// Ends the swipe: complete the pop if it went far enough, or was thrown.
    fn release(&mut self, now: f32, width: f32, cancelled: bool) -> TrackpadOutput {
        let velocity = self.velocity(now);
        let commit = if cancelled {
            false
        } else if velocity >= self.config.fling_velocity {
            true
        } else if velocity <= -self.config.fling_velocity {
            false
        } else {
            self.progress(width) >= self.config.commit_fraction
        };
        self.state = State::Released { at: now };
        TrackpadOutput::Release { commit }
    }

    /// The swipe's rightward speed over the last moments, in pixels a second.
    fn velocity(&self, now: f32) -> f32 {
        let State::Swiping { samples, .. } = &self.state else { return 0.0 };
        let recent: Vec<&(f32, f32)> =
            samples.iter().filter(|(at, _)| now - at <= TrackpadConfig::VELOCITY_WINDOW).collect();
        let Some((first, _)) = recent.first() else { return 0.0 };
        let travelled: f32 = recent.iter().map(|(_, dx)| dx).sum();
        travelled / (now - first).max(1.0 / 60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: f32 = 400.0;
    /// Where a swipe has gone half way: half of the travel a full pop takes.
    const HALF: f32 = WIDTH * TrackpadConfig::TRAVEL_FRACTION / 2.0;

    fn recognizer() -> TrackpadSwipe {
        TrackpadSwipe::new(TrackpadConfig::default())
    }

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    /// Fingers down and moving: the stream a trackpad gives while they are.
    fn feed(swipe: &mut TrackpadSwipe, phase: TouchPhase, delta: Vec2d, now: f32) -> TrackpadOutput {
        swipe.scroll(delta, phase, true, now, WIDTH)
    }

    /// The progress an update reports.
    fn progress_of(output: TrackpadOutput) -> f32 {
        match output {
            TrackpadOutput::Update(progress) => progress,
            other => panic!("expected an update, got {other:?}"),
        }
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
    }

    /// A gesture that has travelled far enough right to have begun.
    fn begun() -> TrackpadSwipe {
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(30.0, 1.0), 0.0), TrackpadOutput::Begin);
        swipe
    }

    #[test]
    fn a_rightward_gesture_begins_once_it_has_travelled_past_the_slop() {
        let mut swipe = recognizer();

        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(4.0, 0.0), 0.00), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(4.0, 0.0), 0.01), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(12.0, 0.5), 0.02), TrackpadOutput::Begin);
        assert!(swipe.is_swiping());
    }

    #[test]
    fn a_gesture_that_is_not_clearly_sideways_is_the_pages() {
        // Mostly down: a scroll.
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(3.0, 40.0), 0.0), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(60.0, 20.0), 0.1), TrackpadOutput::None);
        assert!(!swipe.is_swiping(), "it stays the page's for the rest of the gesture");

        // Sideways but a little too diagonal: 20 across and 14 down.
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(20.0, 14.0), 0.0), TrackpadOutput::None);
        // Sideways enough: 20 across and 8 down.
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(20.0, 8.0), 0.0), TrackpadOutput::Begin);
    }

    #[test]
    fn a_leftward_gesture_never_begins() {
        let mut swipe = recognizer();

        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(-40.0, 0.0), 0.0), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(-40.0, 0.0), 0.1), TrackpadOutput::None);
        assert!(!swipe.is_swiping());
    }

    #[test]
    fn a_gesture_the_page_keeps_is_judged_again_only_when_a_new_one_starts() {
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(0.0, 50.0), 0.0), TrackpadOutput::None);
        // Drifting sideways later in the same gesture does not turn it into a swipe.
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(80.0, 0.0), 0.2), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 0.3), TrackpadOutput::None);

        // The next one is its own.
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(30.0, 0.0), 1.0), TrackpadOutput::Begin);
    }

    #[test]
    fn the_swipe_follows_the_travel_and_can_come_back() {
        let mut swipe = begun();
        // 30 already; HALF in all brings it to half a pop.
        let output = feed(&mut swipe, TouchPhase::Moved, at(HALF - 30.0, 0.0), 0.1);
        assert_near(progress_of(output), 0.5);
        assert_near(swipe.progress(WIDTH), 0.5);

        // Back the way it came.
        assert_near(progress_of(feed(&mut swipe, TouchPhase::Moved, at(-HALF / 2.0, 0.0), 0.2)), 0.25);
        // Past where it began does not go negative, and past the end does not go over.
        assert_eq!(progress_of(feed(&mut swipe, TouchPhase::Moved, at(-WIDTH, 0.0), 0.3)), 0.0);
        assert_eq!(progress_of(feed(&mut swipe, TouchPhase::Moved, at(WIDTH * 3.0, 0.0), 0.4)), 1.0);
        // The travel past either end is not owed: coming back from far over the end
        // starts at the end.
        assert_near(progress_of(feed(&mut swipe, TouchPhase::Moved, at(-HALF, 0.0), 0.5)), 0.5);
    }

    #[test]
    fn lifting_the_fingers_past_half_way_commits_and_short_of_it_cancels() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(HALF + 1.0 - 30.0, 0.0), 1.0);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 2.0), TrackpadOutput::Release { commit: true });
        assert!(!swipe.is_swiping());

        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(HALF - 31.0, 0.0), 1.0);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 2.0), TrackpadOutput::Release { commit: false });
    }

    #[test]
    fn a_quick_flick_commits_from_a_short_distance_and_a_quick_one_back_cancels() {
        // A short distance covered fast, then lifted at once.
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(20.0, 0.0), 0.04);
        let _ = feed(&mut swipe, TouchPhase::Moved, at(20.0, 0.0), 0.08);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 0.09), TrackpadOutput::Release { commit: true });

        // Far past half way, but heading back quickly when the fingers lift.
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(WIDTH, 0.0), 0.5);
        let _ = feed(&mut swipe, TouchPhase::Moved, at(-50.0, 0.0), 1.00);
        let _ = feed(&mut swipe, TouchPhase::Moved, at(-50.0, 0.0), 1.04);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 1.06), TrackpadOutput::Release { commit: false });
    }

    #[test]
    fn a_pause_before_lifting_takes_the_speed_away() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(20.0, 0.0), 0.04);
        // The fingers rest for a good while; the earlier speed is old news.
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 1.5), TrackpadOutput::Release { commit: false });
    }

    /// A swipe that has gone 0.4 of the way and lifted at a moderate speed, so a glide
    /// is to follow: between the speed under which none is expected and the flick.
    fn lifted_with_a_glide_to_come() -> TrackpadSwipe {
        let mut swipe = begun();
        // 30 at the start and 66 more makes 96 of the 240 a full pop takes: 0.4.
        let _ = feed(&mut swipe, TouchPhase::Moved, at(66.0, 0.0), 0.10);
        let _ = feed(&mut swipe, TouchPhase::Moved, at(10.0, 0.0), 0.50);
        let _ = feed(&mut swipe, TouchPhase::Moved, at(10.0, 0.0), 0.55);
        // 20 pixels in the last 80 ms: 250 a second.
        let lift = feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 0.58);
        assert!(matches!(lift, TrackpadOutput::Update(_)), "the swipe goes on, awaiting its glide: {lift:?}");
        assert!(swipe.is_swiping(), "and is not over");
        swipe
    }

    fn glide(swipe: &mut TrackpadSwipe, dx: f32, now: f32) -> TrackpadOutput {
        swipe.scroll(at(dx, 0.0), TouchPhase::Moved, false, now, WIDTH)
    }

    #[test]
    fn the_glide_after_the_fingers_lift_goes_on_moving_the_swipe() {
        let mut swipe = lifted_with_a_glide_to_come();

        let before = swipe.progress(WIDTH);
        let output = glide(&mut swipe, 12.0, 0.60);

        assert_near(progress_of(output), before + 12.0 / (WIDTH * TrackpadConfig::TRAVEL_FRACTION));
        assert!(swipe.holds(0.61), "it is the swipe's, not the page's");
    }

    #[test]
    fn a_swipe_the_glide_carries_past_half_way_completes_where_the_lift_alone_would_not() {
        let mut swipe = lifted_with_a_glide_to_come();
        assert!(swipe.progress(WIDTH) < 0.5, "at the lift it was short of half way");

        let _ = glide(&mut swipe, 8.0, 0.60);
        let _ = glide(&mut swipe, 6.0, 0.62);
        let end = swipe.scroll(at(0.0, 0.0), TouchPhase::Ended, false, 0.64, WIDTH);

        assert_eq!(end, TrackpadOutput::Release { commit: true });
    }

    #[test]
    fn a_glide_that_stops_short_of_half_way_cancels() {
        let mut swipe = lifted_with_a_glide_to_come();

        let _ = glide(&mut swipe, 2.0, 0.60);
        let end = swipe.scroll(at(0.0, 0.0), TouchPhase::Ended, false, 0.62, WIDTH);

        assert_eq!(end, TrackpadOutput::Release { commit: false });
    }

    #[test]
    fn a_glide_that_goes_quiet_is_ended_by_the_host_asking() {
        let mut swipe = lifted_with_a_glide_to_come();
        let _ = glide(&mut swipe, 30.0, 0.60);

        // Events still arriving: not yet.
        assert_eq!(swipe.poll(0.64, WIDTH), TrackpadOutput::None);
        // Then none for a moment, and no end ever reported.
        assert_eq!(swipe.poll(0.80, WIDTH), TrackpadOutput::Release { commit: true });
        assert!(!swipe.is_swiping());
        assert_eq!(swipe.poll(1.0, WIDTH), TrackpadOutput::None, "once");
    }

    #[test]
    fn a_glide_does_not_go_on_for_ever() {
        let mut swipe = lifted_with_a_glide_to_come();

        // Events keep coming, a little at a time, but past a third of a second the
        // glide is over whatever it is still doing.
        let mut ended = None;
        for step in 0..40 {
            let now = 0.60 + step as f32 * 0.02;
            if let TrackpadOutput::Release { commit } = glide(&mut swipe, 0.5, now) {
                ended = Some((now, commit));
                break;
            }
        }
        let (at_time, _) = ended.expect("the glide ended by itself");
        assert!(at_time <= 0.58 + 0.35, "it ended at {at_time}");
    }

    #[test]
    fn a_new_gesture_during_the_glide_ends_it_where_it_has_got_to() {
        let mut swipe = lifted_with_a_glide_to_come();
        let _ = glide(&mut swipe, 20.0, 0.60);

        let output = swipe.scroll(at(5.0, 0.0), TouchPhase::Started, true, 0.65, WIDTH);

        assert_eq!(output, TrackpadOutput::Release { commit: true });
        assert!(!swipe.is_swiping());
    }

    #[test]
    fn cancelling_from_outside_ends_a_glide_without_completing_it() {
        let mut swipe = lifted_with_a_glide_to_come();
        let _ = glide(&mut swipe, 40.0, 0.60);

        assert_eq!(swipe.cancel(0.62), TrackpadOutput::Release { commit: false });
    }

    #[test]
    fn a_swipe_lifted_with_no_speed_ends_at_once_for_there_is_no_glide_to_wait_for() {
        // The fingers rested, so nothing follows them.
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(HALF, 0.0), 0.1);

        let lift = feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 1.0);

        assert_eq!(lift, TrackpadOutput::Release { commit: true });
    }

    #[test]
    fn a_cancelled_gesture_cancels_the_swipe() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(WIDTH, 0.0), 0.5);

        assert_eq!(feed(&mut swipe, TouchPhase::Cancelled, at(0.0, 0.0), 0.6), TrackpadOutput::Release { commit: false });
    }

    #[test]
    fn fingers_that_lift_without_an_end_still_release_the_swipe() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(HALF, 0.0), 1.0);

        // The first momentum event: no longer direct manipulation.
        let output = swipe.scroll(at(10.0, 0.0), TouchPhase::Moved, false, 1.02, WIDTH);

        assert_eq!(output, TrackpadOutput::Release { commit: true });
    }

    #[test]
    fn momentum_after_the_release_is_swallowed_for_a_moment_and_then_it_is_not() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(HALF, 0.0), 1.0);
        let _ = feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 1.1);

        // The glide that follows belongs to the swipe that has just ended.
        assert!(swipe.holds(1.2));
        assert_eq!(swipe.scroll(at(8.0, 0.0), TouchPhase::Moved, false, 1.2, WIDTH), TrackpadOutput::None);
        // Long after, a wheel of any kind is nothing to do with it.
        assert!(!swipe.holds(3.0));
        assert_eq!(swipe.scroll(at(8.0, 0.0), TouchPhase::Moved, false, 3.0, WIDTH), TrackpadOutput::None);
    }

    #[test]
    fn a_wheel_that_is_not_a_finger_never_begins_anything() {
        let mut swipe = recognizer();

        for step in 0..20 {
            let output = swipe.scroll(at(40.0, 0.0), TouchPhase::Moved, false, step as f32 * 0.01, WIDTH);
            assert_eq!(output, TrackpadOutput::None);
        }
        assert!(!swipe.is_swiping() && !swipe.holds(0.2));
    }

    #[test]
    fn a_swipe_the_host_could_not_start_leaves_the_rest_of_the_gesture_to_the_page() {
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(30.0, 0.0), 0.0), TrackpadOutput::Begin);

        swipe.decline();

        assert!(!swipe.is_swiping());
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(60.0, 0.0), 0.1), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 0.2), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(30.0, 0.0), 1.0), TrackpadOutput::Begin, "the next is its own");
    }

    #[test]
    fn a_gesture_the_page_used_is_the_pages_until_the_fingers_lift() {
        // The page took the very first frame.
        let mut swipe = recognizer();
        swipe.yield_to_page();
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(80.0, 0.0), 0.1), TrackpadOutput::None);
        assert!(!swipe.is_swiping());
        assert_eq!(feed(&mut swipe, TouchPhase::Ended, at(0.0, 0.0), 0.2), TrackpadOutput::None);
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(30.0, 0.0), 1.0), TrackpadOutput::Begin, "the next is its own");

        // The page took a frame after the gesture was already being watched.
        let mut swipe = recognizer();
        assert_eq!(feed(&mut swipe, TouchPhase::Started, at(4.0, 0.0), 0.0), TrackpadOutput::None);
        swipe.yield_to_page();
        assert_eq!(feed(&mut swipe, TouchPhase::Moved, at(60.0, 0.0), 0.1), TrackpadOutput::None);
        assert!(!swipe.is_swiping());
    }

    #[test]
    fn yielding_never_lets_go_of_a_swipe_that_is_under_way() {
        let mut swipe = begun();

        swipe.yield_to_page();

        assert!(swipe.is_swiping(), "a swipe the host owns is not the page's to take back");
    }

    #[test]
    fn cancelling_from_outside_releases_a_swipe_without_committing() {
        let mut swipe = begun();
        let _ = feed(&mut swipe, TouchPhase::Moved, at(WIDTH, 0.0), 0.5);

        assert_eq!(swipe.cancel(0.6), TrackpadOutput::Release { commit: false });
        assert_eq!(swipe.cancel(0.7), TrackpadOutput::None, "and there is nothing left to cancel");
    }

    #[test]
    fn nothing_that_is_not_a_number_moves_a_swipe() {
        let mut swipe = begun();

        for delta in [at(f32::NAN, 0.0), at(f32::INFINITY, 0.0), at(0.0, f32::NAN)] {
            let output = feed(&mut swipe, TouchPhase::Moved, delta, 0.1);
            if let TrackpadOutput::Update(progress) = output {
                assert!((0.0..=1.0).contains(&progress) && progress.is_finite(), "{progress}");
            }
        }
        assert!(swipe.progress(WIDTH).is_finite());
        assert!(swipe.progress(0.0).is_finite() && swipe.progress(f32::NAN).is_finite());
    }
}
