use aimer_utils::AnimInstant as Instant;

/// Longest pause inside one browser scroll gesture.
///
/// A browser reports wheel events without any phase, so the boundary between
/// two gestures has to be inferred from cadence. A trackpad flick and its
/// browser-synthesized momentum tail arrive at frame rate, while the pause
/// between two deliberate scrolls is far longer, so a gap of this length
/// separates one gesture from the next.
const IDLE_GAP_MS: u128 = 80;

/// Where the wheel event that was just received sits inside a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WebScrollTransition {
    /// The event opens a gesture and nothing was open before it.
    Begin,
    /// The event belongs to the gesture that is already open.
    Continue,
    /// The event opens a gesture while a stale one is still open, which must
    /// be closed before the new one starts.
    Restart,
}

/// Segments the phase-less browser wheel stream into discrete gestures.
///
/// Winit's web backend emits every `WindowEvent::MouseWheel` with a hardcoded
/// [`TouchPhase::Moved`](winit::event::TouchPhase::Moved), because the DOM
/// `wheel` event carries no phase at all — there is no browser API that
/// reports when the fingers touch or leave a trackpad. Widgets, however, are
/// written against a well-formed `Started → Moved* → Ended` sequence, so the
/// missing boundaries are inferred here and injected into the smoother:
///
/// * the first event after an idle pause opens a gesture, and
/// * a gesture closes once the stream goes quiet for [`IDLE_GAP_MS`], or as
///   soon as the cursor moves, because moving the pointer means the user
///   aimed at something else and the next wheel event is a new intent.
///
/// The tracker only decides *when* a gesture begins and ends; it never
/// touches distance, so the scrolled amount is unchanged.
///
/// # Examples
///
/// ```ignore
/// let mut phase = WebScrollPhase::new();
/// let start = Instant::now();
///
/// assert_eq!(phase.on_wheel_at(start), WebScrollTransition::Begin);
/// assert_eq!(
///     phase.on_wheel_at(start + Duration::from_millis(16)),
///     WebScrollTransition::Continue,
/// );
/// assert!(phase.poll_idle_at(start + Duration::from_millis(400)));
/// ```
#[derive(Debug, Default)]
pub(crate) struct WebScrollPhase {
    /// When the last wheel event arrived, if a gesture is open.
    last_input_at: Option<Instant>,
    /// A gesture is open, meaning `Started` was already injected.
    open: bool,
}

impl WebScrollPhase {
    /// Creates a tracker with no gesture in flight.
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Folds one wheel event, received at `now`, into the gesture state.
    ///
    /// [`WebScrollTransition::Restart`] is reported when the previous gesture
    /// is still open although the stream was idle for longer than
    /// [`IDLE_GAP_MS`]. That happens whenever no frame was rendered during the
    /// pause, so [`poll_idle_at`](Self::poll_idle_at) never got the chance to
    /// close it, and the caller has to inject the missing end before the new
    /// start.
    pub(crate) fn on_wheel_at(&mut self, now: Instant) -> WebScrollTransition {
        let transition = if !self.open {
            WebScrollTransition::Begin
        } else if self.is_idle_at(now) {
            WebScrollTransition::Restart
        } else {
            WebScrollTransition::Continue
        };
        self.open = true;
        self.last_input_at = Some(now);
        transition
    }

    /// Folds one wheel event received right now.
    #[inline]
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn on_wheel(&mut self) -> WebScrollTransition {
        self.on_wheel_at(Instant::now())
    }

    /// Closes an open gesture that has been quiet since [`IDLE_GAP_MS`].
    ///
    /// Returns whether this call closed the gesture, so a terminating phase is
    /// injected exactly once no matter how often the frame loop polls.
    pub(crate) fn poll_idle_at(&mut self, now: Instant) -> bool {
        if !self.open || !self.is_idle_at(now) {
            return false;
        }
        self.reset();
        true
    }

    /// Closes an open gesture that has been quiet up to this moment.
    #[inline]
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn poll_idle(&mut self) -> bool {
        self.poll_idle_at(Instant::now())
    }

    /// Closes an open gesture immediately.
    ///
    /// Returns whether a gesture was open, which is what the cursor-move path
    /// uses to decide if a terminating phase is owed to the widget tree.
    pub(crate) fn end(&mut self) -> bool {
        let was_open = self.open;
        self.reset();
        was_open
    }

    /// Whether a gesture is currently open.
    #[inline]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Forgets any gesture without reporting its end.
    #[inline]
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether the stream has been quiet for longer than [`IDLE_GAP_MS`].
    #[inline]
    fn is_idle_at(&self, now: Instant) -> bool {
        self.last_input_at
            .is_some_and(|at| now.duration_since(at).as_millis() > IDLE_GAP_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ms(base: Instant, millis: u64) -> Instant {
        base + Duration::from_millis(millis)
    }

    #[test]
    fn the_first_wheel_event_opens_a_gesture() {
        let mut phase = WebScrollPhase::new();

        assert_eq!(
            phase.on_wheel_at(Instant::now()),
            WebScrollTransition::Begin
        );
        assert!(phase.is_open());
    }

    #[test]
    fn events_inside_the_idle_gap_stay_one_gesture() {
        let mut phase = WebScrollPhase::new();
        let base = Instant::now();

        phase.on_wheel_at(base);

        assert_eq!(
            phase.on_wheel_at(ms(base, 16)),
            WebScrollTransition::Continue
        );
        assert_eq!(
            phase.on_wheel_at(ms(base, 32)),
            WebScrollTransition::Continue
        );
        assert_eq!(
            phase.on_wheel_at(ms(base, 32 + IDLE_GAP_MS as u64)),
            WebScrollTransition::Continue,
            "a pause of exactly the idle gap still belongs to the same gesture"
        );
    }

    #[test]
    fn an_event_after_the_idle_gap_restarts_the_gesture() {
        let mut phase = WebScrollPhase::new();
        let base = Instant::now();

        phase.on_wheel_at(base);

        assert_eq!(
            phase.on_wheel_at(ms(base, IDLE_GAP_MS as u64 + 1)),
            WebScrollTransition::Restart,
            "the stale gesture must be closed before the new one opens"
        );
        assert!(phase.is_open());
    }

    #[test]
    fn an_idle_stream_ends_the_gesture_exactly_once() {
        let mut phase = WebScrollPhase::new();
        let base = Instant::now();
        phase.on_wheel_at(base);

        assert!(!phase.poll_idle_at(ms(base, IDLE_GAP_MS as u64)));
        assert!(phase.is_open());
        assert!(phase.poll_idle_at(ms(base, IDLE_GAP_MS as u64 + 1)));
        assert!(!phase.is_open());
        assert!(!phase.poll_idle_at(ms(base, 500)));
    }

    #[test]
    fn a_gesture_that_already_ended_begins_the_next_one_cleanly() {
        let mut phase = WebScrollPhase::new();
        let base = Instant::now();
        phase.on_wheel_at(base);
        assert!(phase.poll_idle_at(ms(base, 300)));

        assert_eq!(
            phase.on_wheel_at(ms(base, 400)),
            WebScrollTransition::Begin,
            "nothing is open, so no stale gesture has to be closed"
        );
    }

    #[test]
    fn a_cursor_move_ends_the_open_gesture() {
        let mut phase = WebScrollPhase::new();
        let base = Instant::now();
        phase.on_wheel_at(base);

        assert!(phase.end());
        assert!(!phase.is_open());
        assert_eq!(
            phase.on_wheel_at(ms(base, 16)),
            WebScrollTransition::Begin,
            "aiming at another target starts a new scroll intent"
        );
    }

    #[test]
    fn ending_an_idle_tracker_reports_nothing_to_close() {
        let mut phase = WebScrollPhase::new();

        assert!(!phase.end());
        assert!(!phase.is_open());
    }

    #[test]
    fn a_reset_tracker_forgets_the_open_gesture() {
        let mut phase = WebScrollPhase::new();
        phase.on_wheel_at(Instant::now());

        phase.reset();

        assert!(!phase.is_open());
        assert!(!phase.end());
    }
}
