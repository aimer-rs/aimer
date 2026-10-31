//! Taking hold of the scroll bar thumb with a finger.
//!
//! A thumb is a few pixels wide, and a fingertip near the edge of a scrolling
//! page is usually after the page, not the bar. So a touch on the thumb does
//! not grab it at once: it starts a *hold*, and only a finger that rests for
//! [`SCROLLBAR_THUMB_HOLD_MS`] without wandering takes the thumb. A finger that
//! moves first was swiping, and the content gets that swipe.
//!
//! Everything here is pure with respect to time: nothing reads the clock, so
//! the hold is exercised by handing in instants rather than by sleeping.

use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_utils::AnimInstant;

use super::controller::{DragMode, ScrollState};
use crate::scrollable::constants::*;

/// How a finger's hold on the scroll bar thumb is progressing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ThumbGrab {
    /// No finger is on the thumb.
    Idle,
    /// A finger landed on `thumb` at `since` and has to keep still to take it.
    Holding {
        thumb: DragMode,
        since: AnimInstant,
        /// Where the finger landed, window logical pixels.
        origin: Vec2d,
        /// Where it is now, so the drag starts under the finger and not under
        /// the spot it landed on.
        latest: Vec2d,
        /// When the hold was last checked, so the cadence of the checks (the
        /// frames, in practice) can be told from how long the hold has run.
        polled_at: Option<AnimInstant>,
    },
    /// The hold completed at `since`: the thumb follows the finger.
    Armed { since: AnimInstant },
}

/// How a running hold looks at the moment it is checked, for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HoldProgress {
    /// How long the finger has been down.
    pub(crate) held: Duration,
    /// How long since the hold was last checked, or `None` for the first check.
    /// A hold checked every frame shows the frame interval here.
    pub(crate) since_last_poll: Option<Duration>,
}

impl Default for ThumbGrab {
    fn default() -> Self {
        Self::Idle
    }
}

impl ScrollState {
    /// Starts a hold: a finger landed on `thumb` at `at` (window logical
    /// pixels). Nothing is dragged until [`poll_thumb_hold`](Self::poll_thumb_hold)
    /// reports the hold complete.
    pub(crate) fn begin_thumb_hold(&self, thumb: DragMode, at: Vec2d, now: AnimInstant) {
        self.bar.grab.set(ThumbGrab::Holding {
            thumb,
            since: now,
            origin: at,
            latest: at,
            polled_at: None,
        });
    }

    /// Whether a finger is on the thumb but has not yet held it long enough.
    #[inline]
    pub(crate) fn thumb_holding(&self) -> bool {
        matches!(self.bar.grab.get(), ThumbGrab::Holding { .. })
    }

    /// Whether the thumb has been taken and follows the finger.
    #[inline]
    pub(crate) fn thumb_armed(&self) -> bool {
        matches!(self.bar.grab.get(), ThumbGrab::Armed { .. })
    }

    /// Folds the finger's new position into a hold, returning whether it is
    /// still one.
    ///
    /// A finger that travels further than a drag takes to start was swiping,
    /// and the hold is dropped so the content can have the gesture. A smaller
    /// wobble is just a contact patch rolling, and only updates where the drag
    /// will start.
    pub(crate) fn follow_thumb_hold(&self, at: Vec2d) -> bool {
        let ThumbGrab::Holding {
            thumb,
            since,
            origin,
            polled_at,
            ..
        } = self.bar.grab.get()
        else {
            return false;
        };
        let (dx, dy) = (at.x - origin.x, at.y - origin.y);
        if dx * dx + dy * dy > DRAG_START_THRESHOLD_DP * DRAG_START_THRESHOLD_DP {
            thumb_log!(
                "cancelled: finger moved {:.1} dp from where it landed (limit {} dp), so this is a swipe",
                (dx * dx + dy * dy).sqrt(),
                DRAG_START_THRESHOLD_DP
            );
            self.bar.grab.set(ThumbGrab::Idle);
            return false;
        }
        self.bar.grab.set(ThumbGrab::Holding {
            thumb,
            since,
            origin,
            latest: at,
            polled_at,
        });
        true
    }

    /// Takes the thumb if the hold has run its course by `now` and the bar is
    /// showing.
    ///
    /// Returns how long the finger had been down only on the call that arms it,
    /// so the caller can give the feedback (a haptic tick, the thumb growing)
    /// exactly once, and say how late it was. From then on
    /// the drag works like any thumb drag: the scroll offset follows the finger
    /// from the position it rested at.
    pub(crate) fn poll_thumb_hold(&self, now: AnimInstant) -> Option<Duration> {
        let ThumbGrab::Holding {
            thumb,
            since,
            latest,
            ..
        } = self.bar.grab.get()
        else {
            return None;
        };
        let held_for = now.duration_since(since);
        if held_for.as_millis() < u128::from(SCROLLBAR_THUMB_HOLD_MS) {
            return None;
        }
        // A thumb that is not on screen is not something the finger can be
        // holding. The hold itself is what brings the bar back, so this waits
        // out the fade-in, and the next check takes the thumb.
        if self.bar.opacity.get() < SCROLLBAR_TOUCH_MIN_OPACITY {
            return None;
        }
        let device = self.to_device(latest);
        let scroll = self.scroll_offset.get();
        let anchor = if thumb == DragMode::VerticalScrollbar {
            (device.y, scroll.y)
        } else {
            (device.x, scroll.x)
        };
        self.bar.anchor.set(anchor);
        self.drag_mode.set(thumb);
        self.last_pointer_pos.set(Some(latest));
        self.bar.grab.set(ThumbGrab::Armed { since: now });
        Some(held_for)
    }

    /// Records that a running hold was checked at `now` and reports how it
    /// looks, or `None` when no hold is running. Diagnostics only: it changes
    /// nothing about whether or when the thumb is taken.
    pub(crate) fn note_thumb_poll(&self, now: AnimInstant) -> Option<HoldProgress> {
        let ThumbGrab::Holding {
            thumb,
            since,
            origin,
            latest,
            polled_at,
        } = self.bar.grab.get()
        else {
            return None;
        };
        self.bar.grab.set(ThumbGrab::Holding {
            thumb,
            since,
            origin,
            latest,
            polled_at: Some(now),
        });
        Some(HoldProgress {
            held: now.duration_since(since),
            since_last_poll: polled_at.map(|previous| now.duration_since(previous)),
        })
    }

    /// Lets go of whatever the finger had: an unfinished hold, or a taken
    /// thumb. Returns what it was, so the caller can say how it ended.
    #[inline]
    pub(crate) fn end_thumb_grab(&self) -> ThumbGrab {
        self.bar.grab.replace(ThumbGrab::Idle)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_attribute::position::Vec2d;
    use aimer_utils::AnimInstant;

    use super::ThumbGrab;
    use crate::scrollable::constants::*;
    use crate::scrollable::controller::{DragMode, ScrollState};

    fn hold_for(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn at(x: f32, y: f32) -> Vec2d {
        Vec2d { x, y }
    }

    fn pressed(origin: Vec2d, now: AnimInstant) -> ScrollState {
        let state = ScrollState::for_test_at(Vec2d::ZERO);
        // The bar is showing unless a test says otherwise.
        state.bar.opacity.set(1.0);
        state.begin_thumb_hold(DragMode::VerticalScrollbar, origin, now);
        state
    }

    #[test]
    fn a_fresh_press_only_starts_a_hold() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);

        assert!(state.thumb_holding());
        assert!(!state.thumb_armed());
        assert_eq!(state.drag_mode.get(), DragMode::None, "nothing is dragged yet");
    }

    #[test]
    fn the_thumb_is_not_taken_before_the_hold_completes() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);

        let armed = state.poll_thumb_hold(t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS - 1));

        assert!(armed.is_none());
        assert!(state.thumb_holding());
        assert_eq!(state.drag_mode.get(), DragMode::None);
    }

    #[test]
    fn the_hold_arms_the_drag_exactly_once() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);
        let due = t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS);

        assert_eq!(
            state.poll_thumb_hold(due),
            Some(hold_for(SCROLLBAR_THUMB_HOLD_MS)),
            "the poll that completes the hold reports how long the finger had been down"
        );
        assert!(state.thumb_armed());
        assert!(!state.thumb_holding());
        assert_eq!(state.drag_mode.get(), DragMode::VerticalScrollbar);
        assert!(
            state.poll_thumb_hold(due + hold_for(16)).is_none(),
            "a later poll must not fire the feedback again"
        );
    }

    #[test]
    fn the_drag_is_anchored_where_the_finger_rested() {
        let t0 = AnimInstant::now();
        let state = ScrollState::for_test_at(at(0.0, -120.0));
        state.last_scale.set(1.0);
        state.bar.opacity.set(1.0);
        state.begin_thumb_hold(DragMode::VerticalScrollbar, at(95.0, 40.0), t0);
        // The finger drifted a little while it rested.
        assert!(state.follow_thumb_hold(at(96.0, 44.0)));

        assert!(state.poll_thumb_hold(t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS)).is_some());

        let (along, scroll) = state.bar.anchor.get();
        assert_eq!(along, 44.0, "measured from where the finger is, not where it landed");
        assert_eq!(scroll, -120.0, "and from the scroll position at that moment");
        assert_eq!(state.last_pointer_pos.get(), Some(at(96.0, 44.0)));
    }

    #[test]
    fn a_finger_that_moves_off_before_the_hold_completes_was_swiping() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);

        let still_holding = state.follow_thumb_hold(at(95.0, 40.0 + DRAG_START_THRESHOLD_DP + 1.0));

        assert!(!still_holding);
        assert!(!state.thumb_holding());
        assert!(
            state.poll_thumb_hold(t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS * 2)).is_none(),
            "a cancelled hold never arms later"
        );
        assert_eq!(state.drag_mode.get(), DragMode::None);
    }

    #[test]
    fn a_small_wobble_does_not_cancel_the_hold() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);

        assert!(state.follow_thumb_hold(at(95.0, 40.0 + DRAG_START_THRESHOLD_DP)));
        assert!(state.thumb_holding());
    }

    #[test]
    fn lifting_ends_both_a_hold_and_a_grab() {
        let t0 = AnimInstant::now();
        let held = pressed(at(95.0, 40.0), t0);
        held.end_thumb_grab();
        assert!(!held.thumb_holding());
        assert!(held.poll_thumb_hold(t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS)).is_none());

        let grabbed = pressed(at(95.0, 40.0), t0);
        assert!(grabbed.poll_thumb_hold(t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS)).is_some());
        grabbed.end_thumb_grab();
        assert!(!grabbed.thumb_armed());
    }

    #[test]
    fn ending_a_grab_says_what_it_ended() {
        let t0 = AnimInstant::now();
        let held = pressed(at(95.0, 40.0), t0);
        assert!(matches!(held.end_thumb_grab(), ThumbGrab::Holding { .. }));
        assert_eq!(held.end_thumb_grab(), ThumbGrab::Idle, "and then there is nothing");

        let grabbed = pressed(at(95.0, 40.0), t0);
        let due = t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS);
        assert!(grabbed.poll_thumb_hold(due).is_some());
        assert_eq!(grabbed.end_thumb_grab(), ThumbGrab::Armed { since: due });
    }

    #[test]
    fn the_poll_cadence_is_reported_while_the_hold_runs() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);

        let first = state.note_thumb_poll(t0 + hold_for(16)).unwrap();
        assert_eq!(first.held, hold_for(16));
        assert_eq!(first.since_last_poll, None, "nothing polled before it");

        let second = state.note_thumb_poll(t0 + hold_for(70)).unwrap();
        assert_eq!(second.held, hold_for(70));
        assert_eq!(second.since_last_poll, Some(hold_for(54)), "a slow frame shows up here");

        state.end_thumb_grab();
        assert!(state.note_thumb_poll(t0 + hold_for(90)).is_none(), "nothing to report once it ends");
    }

    /// A thumb that is not on screen cannot be taken: the hold may run its full
    /// time, but it completes only once the bar is showing.
    #[test]
    fn a_hold_on_a_hidden_bar_completes_only_once_the_bar_shows() {
        let t0 = AnimInstant::now();
        let state = pressed(at(95.0, 40.0), t0);
        let due = t0 + hold_for(SCROLLBAR_THUMB_HOLD_MS + 20);

        state.bar.opacity.set(SCROLLBAR_TOUCH_MIN_OPACITY - 0.01);
        assert!(state.poll_thumb_hold(due).is_none(), "still too faint to be held");
        assert!(state.thumb_holding(), "but the hold is not lost");

        state.bar.opacity.set(SCROLLBAR_TOUCH_MIN_OPACITY);
        assert!(
            state.poll_thumb_hold(due + hold_for(8)).is_some(),
            "the first check after the bar shows takes it"
        );
    }

    #[test]
    fn polling_with_nothing_pressed_does_nothing() {
        let state = ScrollState::for_test_at(Vec2d::ZERO);
        assert!(state.poll_thumb_hold(AnimInstant::now()).is_none());
        assert!(!state.thumb_holding() && !state.thumb_armed());
    }
}
