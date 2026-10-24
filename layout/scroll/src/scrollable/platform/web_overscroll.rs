use std::cell::Cell;

use aimer_attribute::position::Vec2d;
use aimer_events::element::TouchPhase;

/// Fraction of the previous delta a sample must fall below to count as
/// decaying.
///
/// A browser momentum tail loses roughly a sixth of its magnitude on every
/// frame, while a finger easing off the trackpad loses only a few percent at
/// a time. The threshold sits between the two: a user slowing down keeps the
/// stretched edge, a tail hands it to the recovery spring.
const DECAY_RATIO: f32 = 0.90;

/// Consecutive decaying samples that identify a momentum tail.
///
/// One shrinking sample happens in the middle of any gesture; three in a row
/// do not, because a user who is still pushing corrects the speed long before
/// that.
const DECAY_SAMPLES: u8 = 3;

/// Fraction of the gesture's strongest delta a tail must have fallen below.
///
/// Decay alone is not enough: the first frames of a hard flick can shrink
/// while the fingers are still down. Recovery only starts once the stream has
/// also lost a visible part of the energy the gesture peaked at.
const TAIL_PEAK_FRACTION: f32 = 0.7;

/// Growth over the previous delta that hands the gesture back to the user.
const REBOUND_RATIO: f32 = 1.35;

/// Fraction of the gesture's strongest delta a rebound must reach.
///
/// Momentum tails are not perfectly monotonic; a small bump inside a decaying
/// stream must not be mistaken for a fresh push.
const REBOUND_PEAK_FRACTION: f32 = 0.5;

/// Delta magnitude (physical pixels) at which a stream is spent.
const SPENT_MAGNITUDE: f32 = 1.5;

/// Recognizes the browser's post-lift momentum tail inside one web scroll
/// gesture.
///
/// The DOM `wheel` event carries no phase and no contact information, so the
/// platform layer synthesizes a gesture from cadence and reports contact for
/// its whole length — including the momentum the browser keeps delivering
/// after the fingers left the trackpad. An overscrolled edge held by contact
/// would therefore stay stretched until that tail runs dry, which on a hard
/// flick is most of a second of a frozen, rubber-banded viewport.
///
/// This tracker restores the missing lift from the only evidence a browser
/// gives: the shape of the delta stream. A run of [`DECAY_SAMPLES`] shrinking
/// deltas that has also dropped below [`TAIL_PEAK_FRACTION`] of the gesture's
/// peak is a tail, contact is dropped, and the bouncy edge recovers while the
/// tail is still arriving. A delta that grows back past [`REBOUND_RATIO`] is
/// the user pushing again and takes the contact back.
///
/// The tracker never touches distance: it only decides whether a frame still
/// counts as direct manipulation, so the scrolled amount is unchanged.
///
/// # Examples
///
/// ```ignore
/// let decay = WebOverscrollDecay::new();
///
/// // A gesture the user keeps feeding holds the stretched edge.
/// assert!(decay.observe(Vec2d { x: 0.0, y: 40.0 }));
/// assert!(decay.observe(Vec2d { x: 0.0, y: 42.0 }));
///
/// // Its momentum tail hands the edge over to the recovery spring.
/// for delta in [30.0, 22.0, 16.0] {
///     decay.observe(Vec2d { x: 0.0, y: delta });
/// }
/// assert!(!decay.observe(Vec2d { x: 0.0, y: 11.0 }));
/// ```
#[derive(Debug, Default)]
pub(crate) struct WebOverscrollDecay {
    /// Magnitude of the previous delta of the open gesture.
    last_magnitude: Cell<f32>,
    /// Strongest magnitude seen since the gesture opened or last rebounded.
    peak_magnitude: Cell<f32>,
    /// Consecutive shrinking samples observed since the last rebound.
    decaying_samples: Cell<u8>,
    /// The stream was recognized as a momentum tail.
    decayed: Cell<bool>,
}

impl WebOverscrollDecay {
    /// Creates a tracker with no gesture observed yet.
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Forgets the observed gesture, so the next delta opens a new one.
    ///
    /// Called on every gesture boundary the platform layer does report — the
    /// injected start of a web gesture, or a frame without contact at all.
    #[inline]
    pub(crate) fn reset(&self) {
        self.last_magnitude.set(0.0);
        self.peak_magnitude.set(0.0);
        self.decaying_samples.set(0);
        self.decayed.set(false);
    }

    /// Closes the observed gesture without forgetting the stream it was fed.
    ///
    /// Called when a gesture terminates while the browser is still delivering
    /// its momentum: the tail that arrives afterwards belongs to the very
    /// scroll that just ended, so its shape must keep being measured against
    /// the peak and the last magnitude of that scroll. Forgetting them — as
    /// [`reset`](Self::reset) does — makes the first tail delta of a hard
    /// flick, which can still be tens of pixels, read as a fresh push, hand
    /// the contact back and stretch the edge a second time.
    ///
    /// The stream therefore stays a tail, and only a delta that genuinely
    /// grows back past [`REBOUND_RATIO`] of its predecessor and
    /// [`REBOUND_PEAK_FRACTION`] of the peak returns contact to the user.
    #[inline]
    pub(crate) fn end_gesture(&self) {
        self.decaying_samples.set(DECAY_SAMPLES);
        self.decayed.set(true);
    }

    /// Folds one device delta in and reports whether the gesture still counts
    /// as direct manipulation.
    ///
    /// `delta` is the raw device distance for this frame, taken before
    /// overscroll resistance is applied: resistance shrinks every delta near
    /// an edge and would read as a tail on its own.
    pub(crate) fn observe(&self, delta: Vec2d) -> bool {
        let magnitude = delta.x.abs().max(delta.y.abs());
        let previous = self.last_magnitude.replace(magnitude);
        let peak = self.peak_magnitude.get();

        if magnitude > previous * REBOUND_RATIO && magnitude > peak * REBOUND_PEAK_FRACTION {
            // The stream grew back: the user is feeding the gesture again, and
            // the peak restarts here so the next tail is measured against this
            // push rather than against an older, stronger one.
            self.peak_magnitude.set(magnitude);
            self.decaying_samples.set(0);
            self.decayed.set(false);
            return true;
        }

        if magnitude > peak {
            self.peak_magnitude.set(magnitude);
        }

        if magnitude <= SPENT_MAGNITUDE {
            self.decayed.set(true);
            return false;
        }

        if magnitude < previous * DECAY_RATIO {
            self.decaying_samples
                .set(self.decaying_samples.get().saturating_add(1));
        }

        let tail = self.decaying_samples.get() >= DECAY_SAMPLES
            && magnitude < self.peak_magnitude.get() * TAIL_PEAK_FRACTION;
        if tail {
            self.decayed.set(true);
        }
        !self.decayed.get()
    }
}

/// Resolves the contact a delivered web scroll frame really carries.
///
/// `is_direct_manipulation` is what the platform layer claims, which in a
/// browser is `true` for the whole synthesized gesture, and `delta` is the raw
/// device distance of the frame, taken before overscroll resistance shrinks
/// it. The three gesture boundaries are folded in here so the reconstruction
/// has a single owner:
///
/// * a reported [`TouchPhase::Started`] opens a genuinely new gesture, so the
///   observed shape is forgotten and the user is back in contact;
/// * a terminating phase without contact is a gesture that ended while its
///   momentum is still in flight, so the shape is kept (see
///   [`WebOverscrollDecay::end_gesture`]);
/// * any other frame without contact is not part of a device gesture at all —
///   wheel input between gestures — and starts from scratch.
pub(crate) fn web_device_contact(
    decay: &WebOverscrollDecay,
    is_direct_manipulation: bool,
    delta: Vec2d,
    phase: TouchPhase,
) -> bool {
    if !is_direct_manipulation {
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            decay.end_gesture();
        } else {
            decay.reset();
        }
        return false;
    }
    if matches!(phase, TouchPhase::Started) {
        decay.reset();
    }
    decay.observe(delta)
}

#[cfg(test)]
mod tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::TouchPhase;

    use super::{WebOverscrollDecay, web_device_contact};

    /// A vertical device delta of `magnitude` physical pixels.
    fn down(magnitude: f32) -> Vec2d {
        Vec2d {
            x: 0.0,
            y: magnitude,
        }
    }

    #[test]
    fn a_gesture_the_user_keeps_feeding_stays_in_contact() {
        let decay = WebOverscrollDecay::new();

        for magnitude in [40.0, 42.0, 39.0, 44.0, 41.0, 43.0] {
            assert!(
                decay.observe(down(magnitude)),
                "a plateau of {magnitude} px is a finger on the trackpad"
            );
        }
    }

    #[test]
    fn a_decaying_tail_drops_contact_before_the_stream_runs_dry() {
        let decay = WebOverscrollDecay::new();
        assert!(decay.observe(down(40.0)));

        let released = [34.0, 28.0, 23.0, 19.0]
            .into_iter()
            .position(|magnitude| !decay.observe(down(magnitude)))
            .expect("a decaying stream must hand the edge to the recovery spring");
        assert!(
            released < 3,
            "recovery starts within the first frames of the tail, not at its end"
        );

        for magnitude in [15.0, 12.0, 9.0, 7.0] {
            assert!(
                !decay.observe(down(magnitude)),
                "the rest of the tail stays out of contact"
            );
        }
    }

    #[test]
    fn a_slow_gesture_is_not_a_tail_while_it_holds_its_energy() {
        let decay = WebOverscrollDecay::new();

        // Three shrinking samples that never lose a visible part of the peak:
        // a user easing off, not a browser coasting.
        for magnitude in [40.0, 38.0, 36.5, 35.0, 34.0] {
            assert!(decay.observe(down(magnitude)));
        }
    }

    #[test]
    fn a_renewed_push_takes_the_contact_back() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [40.0, 34.0, 28.0, 23.0, 19.0] {
            decay.observe(down(magnitude));
        }

        assert!(
            decay.observe(down(38.0)),
            "a delta that grows back is the user pushing again"
        );
        assert!(decay.observe(down(37.0)));
    }

    #[test]
    fn a_bump_inside_the_tail_is_not_a_renewed_push() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [60.0, 50.0, 40.0, 31.0, 24.0] {
            decay.observe(down(magnitude));
        }

        assert!(
            !decay.observe(down(26.0)),
            "jitter inside a decaying stream keeps the recovery running"
        );
    }

    #[test]
    fn a_spent_stream_drops_contact_at_once() {
        let decay = WebOverscrollDecay::new();
        assert!(decay.observe(down(40.0)));

        assert!(
            !decay.observe(down(0.2)),
            "a delta the user cannot feel is the end of the tail"
        );
    }

    #[test]
    fn a_horizontal_tail_is_measured_like_a_vertical_one() {
        let decay = WebOverscrollDecay::new();
        let sideways = |magnitude: f32| Vec2d {
            x: magnitude,
            y: 0.0,
        };

        assert!(decay.observe(sideways(-40.0)));
        assert!(decay.observe(sideways(-34.0)));
        assert!(decay.observe(sideways(-28.0)));
        assert!(!decay.observe(sideways(-23.0)));
    }

    #[test]
    fn the_tail_of_an_ended_hard_flick_never_reads_as_a_fresh_push() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [120.0, 100.0, 82.0, 66.0] {
            decay.observe(down(magnitude));
        }
        assert!(
            !decay.observe(down(54.0)),
            "the flick has decayed into a tail"
        );

        // The recovery terminates the gesture while the browser still has most
        // of the flick queued.
        decay.end_gesture();

        for magnitude in [44.0, 36.0, 29.0, 24.0] {
            assert!(
                !decay.observe(down(magnitude)),
                "{magnitude} px of leftover momentum must not bounce the edge again"
            );
        }
    }

    #[test]
    fn a_push_after_an_ended_gesture_takes_the_contact_back() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [120.0, 100.0, 82.0, 66.0, 54.0] {
            decay.observe(down(magnitude));
        }
        decay.end_gesture();

        assert!(
            decay.observe(down(95.0)),
            "a delta stronger than the tail it follows is the user scrolling again"
        );
    }

    #[test]
    fn an_ended_frame_keeps_the_shape_a_moved_frame_forgets() {
        let tail = |decay: &WebOverscrollDecay, phase| {
            for magnitude in [120.0, 100.0, 82.0, 66.0, 54.0] {
                web_device_contact(decay, true, down(magnitude), TouchPhase::Moved);
            }
            web_device_contact(decay, false, Vec2d::ZERO, phase);
            web_device_contact(decay, true, down(44.0), TouchPhase::Moved)
        };

        assert!(
            !tail(&WebOverscrollDecay::new(), TouchPhase::Ended),
            "a gesture that ended mid-momentum still owns the tail that follows"
        );
        assert!(
            tail(&WebOverscrollDecay::new(), TouchPhase::Moved),
            "a frame outside any device gesture starts a new stream"
        );
    }

    #[test]
    fn a_reported_start_after_an_ended_gesture_opens_a_fresh_stream() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [120.0, 100.0, 82.0, 66.0, 54.0] {
            web_device_contact(&decay, true, down(magnitude), TouchPhase::Moved);
        }
        web_device_contact(&decay, false, Vec2d::ZERO, TouchPhase::Ended);

        assert!(
            web_device_contact(&decay, true, down(12.0), TouchPhase::Started),
            "the platform opening a gesture outranks the reconstruction"
        );
    }

    #[test]
    fn a_reset_opens_a_fresh_gesture_in_contact() {
        let decay = WebOverscrollDecay::new();
        for magnitude in [40.0, 34.0, 28.0, 23.0, 19.0] {
            decay.observe(down(magnitude));
        }
        assert!(!decay.observe(down(15.0)));

        decay.reset();

        assert!(decay.observe(down(12.0)), "a new gesture starts in contact");
        assert!(decay.observe(down(11.5)));
    }
}
