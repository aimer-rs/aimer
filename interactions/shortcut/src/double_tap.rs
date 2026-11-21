//! Recognizes a modifier key that is tapped twice.

use std::time::{Duration, Instant};

use crate::{ChordMods, Key};

/// Recognizes a modifier key tapped twice, which is a shortcut of its own: the double `Shift` that opens a search in many
/// editors.
///
/// A *tap* is a modifier pressed alone and let go soon after, with no other key, modifier or click in between. Two taps of the
/// same modifier that follow each other closely are a double tap, and then the pair is spent, so a third tap starts a new one.
///
/// The detector knows nothing of the window: the application loop reports each change of the modifiers held
/// ([`DoubleTap::modifiers_changed`]) and every other key or click ([`DoubleTap::interrupt`]). The time is given, not read,
/// which keeps it testable.
///
/// # Examples
///
/// ```
/// use std::time::{Duration, Instant};
/// use aimer_shortcut::{ChordMods, DoubleTap};
///
/// let start = Instant::now();
/// let at = |ms| start + Duration::from_millis(ms);
/// let mut tap = DoubleTap::new();
/// tap.modifiers_changed(ChordMods::SHIFT, at(0));
/// tap.modifiers_changed(ChordMods::NONE, at(50));
/// tap.modifiers_changed(ChordMods::SHIFT, at(120));
/// assert_eq!(tap.modifiers_changed(ChordMods::NONE, at(170)), Some(ChordMods::SHIFT));
/// ```
#[derive(Debug, Default)]
pub struct DoubleTap {
    /// What was held at the last report.
    held: ChordMods,
    /// A modifier that went down alone and has stayed so, and when it went down.
    pressed: Option<(ChordMods, Instant)>,
    /// The modifier of the tap that is complete and waits for its pair, and when it was let go.
    first: Option<(ChordMods, Instant)>,
}

impl DoubleTap {
    /// How long a tap may be held, and how long may pass between the two taps of a pair, from the release of the first to the
    /// release of the second.
    pub const WINDOW: Duration = Duration::from_millis(400);

    /// Makes a detector that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports that the modifiers held are now `held`, at `now`. It answers the modifier that was double-tapped, when this
    /// change is the end of the second tap.
    pub fn modifiers_changed(&mut self, held: ChordMods, now: Instant) -> Option<ChordMods> {
        let before = std::mem::replace(&mut self.held, held);
        if held.is_empty() {
            // Let go: a tap, if it went down alone, stayed alone and was short.
            let (modifier, since) = self.pressed.take()?;
            if now.saturating_duration_since(since) > Self::WINDOW {
                self.first = None;
                return None;
            }
            return match self.first.take() {
                Some((tapped, at))
                    if tapped == modifier && now.saturating_duration_since(at) <= Self::WINDOW =>
                {
                    Some(modifier)
                }
                _ => {
                    self.first = Some((modifier, now));
                    None
                }
            };
        }
        // Down alone from nothing starts a tap; anything else spoils the one that was going.
        if before.is_empty() && Key::double_tap(held).is_some() {
            self.pressed = Some((held, now));
        } else {
            self.interrupt();
        }
        None
    }

    /// Reports a key (other than a modifier) or a click: what comes during a tap or between two makes them no double tap.
    pub fn interrupt(&mut self) {
        self.pressed = None;
        self.first = None;
    }
}

#[cfg(test)]
#[path = "double_tap_tests.rs"]
mod double_tap_tests;
