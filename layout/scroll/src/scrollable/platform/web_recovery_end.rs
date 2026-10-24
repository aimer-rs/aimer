use std::cell::Cell;

use aimer_events::element::TouchPhase;

/// Closes a web scroll gesture the moment its bouncy edge has recovered.
///
/// A browser never reports a lift, so the platform layer keeps a synthesized
/// gesture open for as long as `wheel` events keep arriving — including the
/// momentum tail the browser delivers on its own after the fingers left the
/// trackpad. Once the stretched edge has sprung back the gesture is over from
/// the user's point of view, but that tail is still in flight: every remaining
/// delta pushes the content past the edge again and the viewport bounces a
/// second time, seconds after the flick.
///
/// This tracker watches the drawn frames for the recovery finishing and, on
/// that single frame, asks the caller to terminate the gesture with a
/// [`TouchPhase::Ended`] scroll frame. Everything the browser still delivers
/// for that gesture is then dropped, until the user pushes again (contact
/// returns) or the platform opens a new gesture with a fresh phase.
///
/// # Examples
///
/// ```ignore
/// let end = WebRecoveryEnd::new();
///
/// // Frames while the spring pulls the edge back.
/// assert!(!end.observe_frame(true, false));
/// // The frame the content lands on its edge closes the gesture, once.
/// assert!(end.observe_frame(false, false));
/// assert!(!end.observe_frame(false, false));
///
/// // Once the terminating frame is delivered, what the browser still has
/// // queued no longer reaches the content.
/// end.drain_tail();
/// assert!(end.drops_frame(false, TouchPhase::Moved));
/// // A renewed push takes over again.
/// assert!(!end.drops_frame(true, TouchPhase::Moved));
/// ```
#[derive(Debug, Default)]
pub(crate) struct WebRecoveryEnd {
    /// A stretched edge is currently springing back with nothing holding it.
    recovering: Cell<bool>,
    /// The remainder of the closed gesture is being discarded.
    draining: Cell<bool>,
}

impl WebRecoveryEnd {
    /// Creates a tracker for a viewport that is not recovering.
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Folds one drawn frame in and reports whether the bouncy recovery just
    /// finished, so the caller must end the gesture.
    ///
    /// `stretched` is whether the content is past an edge on this frame and
    /// `held` whether something (contact or the crossing grace period) is
    /// keeping it there. A stretch that is held is not a recovery, so letting
    /// go of it — the user scrolling back into range himself — never reports
    /// an end. Returns `true` on exactly one frame per recovery.
    pub(crate) fn observe_frame(&self, stretched: bool, held: bool) -> bool {
        if stretched {
            if !held {
                self.recovering.set(true);
            }
            return false;
        }
        self.recovering.replace(false)
    }

    /// Starts discarding whatever the closed gesture still has in flight.
    ///
    /// Called once the terminating frame of the recovery has been delivered —
    /// that frame belongs to the gesture too, so arming the drain before it is
    /// dispatched would swallow the very end it announces.
    #[inline]
    pub(crate) fn drain_tail(&self) {
        self.draining.set(true);
    }

    /// Reports whether an arriving scroll frame belongs to the tail of an
    /// already-closed gesture and must be discarded.
    ///
    /// `contact` is the contact this frame carries after the web reconstruction
    /// in [`device_contact_for_frame`](crate::scrollable::device_contact::device_contact_for_frame):
    /// it turns `true` again when the delta grows back, which is the user
    /// pushing rather than the browser coasting. A [`TouchPhase::Started`]
    /// opens a gesture of its own and likewise hands the content back.
    ///
    /// A terminating phase does not: it ends the very gesture whose leftovers
    /// are being discarded — the frame this tracker asks for is one of them —
    /// so it must never disarm the drain.
    pub(crate) fn drops_frame(&self, contact: bool, phase: TouchPhase) -> bool {
        if !self.draining.get() {
            return false;
        }
        if contact || matches!(phase, TouchPhase::Started) {
            self.draining.set(false);
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::element::TouchPhase;

    use super::WebRecoveryEnd;

    /// Drives `frames` recovering frames and returns how many of them reported
    /// the end of the gesture.
    fn ends_reported(end: &WebRecoveryEnd, frames: &[(bool, bool)]) -> usize {
        frames
            .iter()
            .filter(|(stretched, held)| end.observe_frame(*stretched, *held))
            .count()
    }

    #[test]
    fn a_settled_recovery_ends_the_gesture_exactly_once() {
        let end = WebRecoveryEnd::new();

        assert_eq!(
            ends_reported(
                &end,
                &[(true, false), (true, false), (false, false), (false, false)]
            ),
            1,
            "the frame the edge lands on closes the gesture, and no later one"
        );
    }

    #[test]
    fn an_idle_viewport_never_ends_a_gesture() {
        let end = WebRecoveryEnd::new();

        assert_eq!(ends_reported(&end, &[(false, false); 4]), 0);
    }

    #[test]
    fn a_stretch_the_user_holds_and_releases_himself_ends_no_gesture() {
        let end = WebRecoveryEnd::new();

        assert_eq!(
            ends_reported(&end, &[(true, true), (true, true), (false, false)]),
            0,
            "scrolling back into range with the fingers down is not a recovery"
        );
    }

    #[test]
    fn a_held_stretch_that_is_let_go_still_ends_its_recovery() {
        let end = WebRecoveryEnd::new();

        assert_eq!(
            ends_reported(&end, &[(true, true), (true, false), (false, false)]),
            1,
            "the hold releasing is what starts the recovery this end belongs to"
        );
    }

    #[test]
    fn a_second_overscroll_ends_its_own_recovery() {
        let end = WebRecoveryEnd::new();

        assert_eq!(
            ends_reported(
                &end,
                &[(true, false), (false, false), (true, false), (false, false),]
            ),
            2
        );
    }

    #[test]
    fn nothing_is_dropped_before_a_recovery_ends() {
        let end = WebRecoveryEnd::new();
        end.observe_frame(true, false);

        assert!(
            !end.drops_frame(false, TouchPhase::Moved),
            "a gesture that is still stretching owns its deltas"
        );
    }

    #[test]
    fn the_terminating_frame_itself_is_not_dropped() {
        let end = WebRecoveryEnd::new();
        end.observe_frame(true, false);
        assert!(end.observe_frame(false, false));

        assert!(
            !end.drops_frame(false, TouchPhase::Ended),
            "the end announced by the recovery must reach the engine"
        );
    }

    #[test]
    fn the_tail_of_a_closed_gesture_is_dropped() {
        let end = WebRecoveryEnd::new();
        end.observe_frame(true, false);
        assert!(end.observe_frame(false, false));
        end.drain_tail();

        for _ in 0..4 {
            assert!(
                end.drops_frame(false, TouchPhase::Moved),
                "what the browser still has queued must not bounce the edge again"
            );
        }
    }

    #[test]
    fn a_renewed_push_takes_the_content_back() {
        let end = WebRecoveryEnd::new();
        end.observe_frame(true, false);
        end.observe_frame(false, false);
        end.drain_tail();
        assert!(end.drops_frame(false, TouchPhase::Moved));

        assert!(
            !end.drops_frame(true, TouchPhase::Moved),
            "a delta the user is feeding is not tail"
        );
        assert!(
            !end.drops_frame(false, TouchPhase::Moved),
            "and the rest of that gesture keeps scrolling"
        );
    }

    #[test]
    fn a_new_gesture_takes_the_content_back() {
        let end = WebRecoveryEnd::new();
        end.observe_frame(true, false);
        end.observe_frame(false, false);
        end.drain_tail();
        assert!(end.drops_frame(false, TouchPhase::Moved));

        assert!(
            !end.drops_frame(false, TouchPhase::Started),
            "a reported gesture boundary outranks the reconstruction"
        );
        assert!(!end.drops_frame(false, TouchPhase::Moved));
    }
}
