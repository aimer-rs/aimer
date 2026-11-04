//! When a player's controls show: the platform-free rules, with time passed in.

use std::time::Duration;

use aimer_time::Instant;

/// How long controls stay up after the last pointer activity while playing.
pub(crate) const HIDE_AFTER: Duration = Duration::from_secs(3);

/// The controls' visibility, from the pointer activity seen so far.
///
/// - Paused, finished or not started: always shown.
/// - Playing: shown for [`HIDE_AFTER`] after the last activity, then hidden.
/// - A mouse shows them on any movement. On a touch screen a tap toggles
///   them instead (a tap on hidden controls only shows them, it does not
///   press what is under the finger), and the big centre play/pause button
///   appears.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ControlsVisibility {
    last_activity: Option<Instant>,
    /// The last input came from a touch screen.
    touch: bool,
}

impl ControlsVisibility {
    /// A pointer moved or pressed over the player.
    pub(crate) fn note_activity(&mut self, now: Instant, touch: bool) {
        self.last_activity = Some(now);
        self.touch = touch;
    }

    /// A tap on the picture (not on a control) on a touch screen. Returns
    /// whether the controls are now shown.
    pub(crate) fn tap(&mut self, now: Instant, playing: bool) -> bool {
        self.touch = true;
        if self.visible(now, playing) {
            // Hide at once; paused controls stay up regardless.
            self.last_activity = None;
        } else {
            self.last_activity = Some(now);
        }
        self.visible(now, playing)
    }

    /// Whether the controls show at `now`.
    pub(crate) fn visible(&self, now: Instant, playing: bool) -> bool {
        !playing
            || self
                .last_activity
                .is_some_and(|last| now.saturating_duration_since(last) < HIDE_AFTER)
    }

    /// When the controls will hide by themselves, if they will.
    pub(crate) fn hides_at(&self, playing: bool) -> Option<Instant> {
        if !playing {
            return None;
        }
        self.last_activity.map(|last| last + HIDE_AFTER)
    }

    /// Whether the last input was touch, so the centre button shows.
    pub(crate) fn is_touch(&self) -> bool {
        self.touch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn paused_controls_always_show() {
        let now = Instant::now();
        let state = ControlsVisibility::default();
        assert!(state.visible(now, false));
        assert!(state.visible(at(now, 3_600_000), false));
        assert_eq!(state.hides_at(false), None);
    }

    #[test]
    fn playing_controls_hide_three_seconds_after_the_last_activity() {
        let start = Instant::now();
        let mut state = ControlsVisibility::default();
        assert!(!state.visible(start, true), "nothing has shown them yet");

        state.note_activity(start, false);
        assert!(state.visible(at(start, 2_999), true));
        assert!(!state.visible(at(start, 3_000), true));
        assert_eq!(state.hides_at(true), Some(at(start, 3_000)));

        // More movement keeps them up.
        state.note_activity(at(start, 2_000), false);
        assert!(state.visible(at(start, 4_500), true));
        assert!(!state.visible(at(start, 5_000), true));
    }

    #[test]
    fn a_tap_on_a_touch_screen_toggles_the_controls() {
        let start = Instant::now();
        let mut state = ControlsVisibility::default();
        assert!(state.tap(start, true), "the first tap shows them");
        assert!(state.is_touch());
        assert!(!state.tap(at(start, 500), true), "a second tap hides them");
        assert!(state.tap(at(start, 900), true));
        assert!(!state.visible(at(start, 3_900), true), "and they still time out");
    }

    #[test]
    fn a_tap_while_paused_cannot_hide_them() {
        let start = Instant::now();
        let mut state = ControlsVisibility::default();
        assert!(state.tap(start, false));
        assert!(state.tap(at(start, 100), false));
    }

    #[test]
    fn a_mouse_after_touch_leaves_touch_mode() {
        let start = Instant::now();
        let mut state = ControlsVisibility::default();
        state.tap(start, true);
        state.note_activity(at(start, 10), false);
        assert!(!state.is_touch());
    }
}
