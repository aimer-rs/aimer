//! The container's half of taking the scroll bar thumb with a finger.
//!
//! The hold itself is state (see `state::thumb_grab`); this is what has to
//! happen around the moment it completes, because that moment arrives either
//! with a pointer move or, for a finger that rests perfectly still, with a
//! frame.

use aimer_events::pointer::PointerSource;
use aimer_native::haptic::{Haptics, ImpactStyle};
use aimer_utils::AnimInstant;
use aimer_widget::{Element, PointerKey};

use crate::raw_scroll::RawScrollableContainer;

/// The impact that confirms the thumb is held.
const THUMB_FEEDBACK: ImpactStyle = ImpactStyle::Medium;

/// What prompted a check of the hold.
///
/// A finger at rest sends no events, so a hold is normally completed by a frame;
/// a finger that moves a little can complete it earlier, on the move. Which one
/// did is the first thing to know when the hold feels late.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoldTrigger {
    Frame,
    Move,
}

impl std::fmt::Display for HoldTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Frame => "frame",
            Self::Move => "move",
        })
    }
}

impl<E: Element> RawScrollableContainer<E> {
    /// Wakes the haptic hardware as a finger lands on the thumb. The impact is
    /// a moment away; the first one after idle is late by the time the engine
    /// takes to start, which is long enough to feel like the hold was slow.
    pub(super) fn warm_thumb_feedback(&self) {
        thumb_log!("haptic prepared ({THUMB_FEEDBACK:?})");
        Haptics::prepare_impact(THUMB_FEEDBACK);
    }

    /// Advances a finger's hold on the thumb to `now`, taking the thumb if the
    /// hold has completed. Returns whether this call took it.
    ///
    /// While the hold is still running it asks for another frame, since a
    /// finger at rest produces no events to wake anything.
    pub(crate) fn settle_thumb_hold(&self, now: AnimInstant, trigger: HoldTrigger) -> bool {
        if let Some(progress) = self.ctrl.note_thumb_poll(now) {
            let opacity = self.ctrl.bar.opacity.get();
            let waiting_for_bar = progress.held.as_millis()
                >= u128::from(crate::scrollable::constants::SCROLLBAR_THUMB_HOLD_MS)
                && opacity < crate::scrollable::constants::SCROLLBAR_TOUCH_MIN_OPACITY;
            thumb_log!(
                "check via {trigger}: finger down {} ms, bar opacity {opacity:.2} ({}){}",
                progress.held.as_millis(),
                progress.since_last_poll.map_or("first check".to_string(), |gap| {
                    format!("{} ms since the last check", gap.as_millis())
                }),
                if waiting_for_bar {
                    ", hold complete but waiting for the bar to show"
                } else {
                    ""
                },
            );
        }
        let Some(held_for) = self.ctrl.poll_thumb_hold(now) else {
            if self.ctrl.thumb_holding() {
                self.ctrl.request_animation_frame();
            }
            return false;
        };
        // The hold is checked when a frame or a move arrives, so this is how
        // long the finger really waited, which is the threshold plus however
        // late that check ran.
        let threshold = crate::scrollable::constants::SCROLLBAR_THUMB_HOLD_MS;
        thumb_log!(
            "taken via {trigger} after {} ms (threshold {threshold} ms, {} ms late)",
            held_for.as_millis(),
            held_for.as_millis().saturating_sub(u128::from(threshold)),
        );
        if let Some(id) = self.ctrl.active_touch_id.get() {
            let finger = PointerKey::new(PointerSource::Touch, id);
            // The content was handed this press when it landed. The finger is
            // the bar's now, so whatever the content was making of it (a tap,
            // a long press of its own) is over, and nothing waiting to read it
            // as a swipe may.
            let _ = self
                .event_dispatcher
                .borrow_mut()
                .cancel_pointer(&self.child, finger);
            aimer_widget::claim_pointer(finger);
        }
        // The one confirmation that the thumb is held: the finger has not moved
        // and the bar has not yet, so there is nothing else to feel or see.
        Haptics::impact(THUMB_FEEDBACK);
        thumb_log!("haptic impact fired ({THUMB_FEEDBACK:?})");
        self.ctrl.request_animation_frame();
        true
    }
}
