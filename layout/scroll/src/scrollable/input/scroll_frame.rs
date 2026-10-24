//! What one delivered wheel / trackpad frame does to the scroll engine.
//!
//! Every scroll frame reaches the engine through
//! [`apply_scroll_frame`] — the one the window system delivered as well as the
//! one a recovered web gesture has to synthesize — so the overscroll hold, the
//! episode peak and the contact flag have a single owner.

use aimer_attribute::position::Vec2d;
use aimer_events::element::{ScrollDeltaKind, TouchPhase};
use aimer_utils::AnimInstant;

use crate::ScrollAxis;
use crate::scrollable::controller::ScrollState;
use crate::scrollable::device_contact::device_contact_for_frame;
use crate::scrollable::overscroll_source::OverscrollSource;
use crate::scrollable::recovery_end::scroll_frame_dropped;

/// Whether a scroll frame's `phase` means the wheel / trackpad gesture is still
/// in flight.
///
/// The platform layer normalizes every device gesture into one
/// [`TouchPhase::Started`], any number of [`TouchPhase::Moved`], and exactly one
/// [`TouchPhase::Ended`] or [`TouchPhase::Cancelled`]. macOS keeps delivering
/// post-lift momentum deltas as `Moved`, folded into the same gesture, so only a
/// terminating phase means the user has stopped scrolling.
#[inline]
fn device_gesture_in_flight(phase: TouchPhase) -> bool {
    !matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
}

/// Whether a device gesture should briefly hold recovery after first crossing
/// a scroll boundary.
///
/// The hold is deliberately not refreshed while already out of bounds. Native
/// momentum tails can deliver outward `Moved` events for much longer than the
/// user expects; refreshing on each event prevents the recovery timeout from
/// ever elapsing.
#[inline]
fn should_hold_overscroll_recovery(
    applied: bool,
    stretch_before: Vec2d,
    stretch_after: Vec2d,
    phase: TouchPhase,
) -> bool {
    let was_in_range = stretch_before.x == 0.0 && stretch_before.y == 0.0;
    let crossed_edge = stretch_after.x != 0.0 || stretch_after.y != 0.0;
    applied && was_in_range && crossed_edge && device_gesture_in_flight(phase)
}

/// What a delivered scroll frame does with the overscroll recovery hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverscrollHoldAction {
    /// Keep the edge stretched: the user's fingers are still on the device.
    HoldForContact,
    /// Keep the edge stretched for the short grace period that lets the frame
    /// which first crossed the boundary reach the screen before recovery.
    HoldForGrace,
    /// Nothing holds the edge, so recovery may run on the next frame.
    Release,
}

/// Decides what a delivered scroll frame does with the overscroll hold.
///
/// A held stretch must survive every frame of the gesture that produced it, so
/// contact is checked first and its `phase` is deliberately ignored: the
/// platform layer synthesizes a terminating phase whenever one of its smoothing
/// channels drains, which happens while the fingers are still down. Trusting
/// the phase there releases the stretch mid-gesture, and the next contact frame
/// takes it again — the content visibly flickers between stretched and resting.
///
/// Without contact the frame is either an inertial tail or wheel input, which
/// only holds the edge for the grace period described by
/// [`should_hold_overscroll_recovery`].
#[inline]
fn overscroll_hold_action(
    is_direct_manipulation: bool,
    applied: bool,
    stretch_before: Vec2d,
    stretch_after: Vec2d,
    phase: TouchPhase,
) -> OverscrollHoldAction {
    if is_direct_manipulation {
        let stretched = stretch_after.x != 0.0 || stretch_after.y != 0.0;
        return if stretched {
            OverscrollHoldAction::HoldForContact
        } else {
            OverscrollHoldAction::Release
        };
    }
    if should_hold_overscroll_recovery(applied, stretch_before, stretch_after, phase) {
        OverscrollHoldAction::HoldForGrace
    } else {
        OverscrollHoldAction::Release
    }
}

/// Applies one delivered scroll frame to `ctrl` and reports whether it moved
/// the content.
///
/// This is the single owner of what a wheel / trackpad frame does to the
/// scroll engine: it resolves the contact the frame really carries, gates the
/// overscroll hold and peak on it, and applies the resisted distance. Both the
/// [`ElementEvent::Scroll`](aimer_events::element::ElementEvent::Scroll) arm
/// and the synthesized end of a recovered web
/// gesture (see
/// [`finish_overscroll_recovery`](crate::scrollable::recovery_end::finish_overscroll_recovery))
/// go through it, so a gesture always terminates the same way whether the
/// platform reported the lift or it had to be reconstructed.
///
/// A `false` return means the frame was discarded and neither the scroll
/// session nor a new animation frame should be started for it.
pub(crate) fn apply_scroll_frame(
    ctrl: &ScrollState,
    delta: Vec2d,
    kind: ScrollDeltaKind,
    phase: TouchPhase,
    is_direct_manipulation: bool,
) -> bool {
    // Whatever the edge does from here belongs to the wheel / trackpad, which
    // not every target trusts with a rubber band.
    ctrl.set_overscroll_source(OverscrollSource::Wheel);

    let scroll_delta = match ctrl.axis {
        ScrollAxis::Vertical => Vec2d { x: 0.0, y: delta.y },
        ScrollAxis::Horizontal => Vec2d { x: delta.x, y: 0.0 },
    };

    // Platforms that cannot report a lift have it reconstructed from the raw
    // stream, so the rest of this function reads one contact signal whatever
    // the target is.
    let is_direct_manipulation =
        device_contact_for_frame(ctrl, is_direct_manipulation, scroll_delta, phase);

    // The leftover of a gesture that already ended must not reach the content.
    if scroll_frame_dropped(ctrl, is_direct_manipulation, phase) {
        return false;
    }

    // A new physical gesture must never inherit the previous episode's stretch
    // gate. Contact landing again is the reliable signal for that; the phase is
    // a per-channel synthesis, so a channel that stays active across two
    // gestures never reports the boundary at all.
    if ctrl.begin_device_contact(is_direct_manipulation) || matches!(phase, TouchPhase::Started) {
        ctrl.reset_overscroll_peak();
    }

    let offset = ctrl.scroll_offset.get();

    // Past an edge the step is resisted more the further the content is already
    // stretched, so a gesture the user keeps feeding settles at the overscroll
    // limit instead of creeping outward for as long as events keep arriving.
    let scroll_delta = ctrl.resisted_overscroll_delta(scroll_delta);

    let applied = if matches!(kind, ScrollDeltaKind::Line) {
        ctrl.apply_line_wheel_delta(scroll_delta)
    } else {
        ctrl.apply_precise_scroll_delta(scroll_delta)
    };

    let stretch_before = ctrl.overscroll_distance(offset);
    let stretch_after = ctrl.overscroll_distance(ctrl.scroll_offset.get());
    match overscroll_hold_action(
        is_direct_manipulation,
        applied,
        stretch_before,
        stretch_after,
        phase,
    ) {
        OverscrollHoldAction::HoldForContact => {
            ctrl.hold_overscroll_for_direct_manipulation();
        }
        OverscrollHoldAction::HoldForGrace => {
            ctrl.release_direct_overscroll_hold();
            ctrl.hold_overscroll_recovery();
        }
        OverscrollHoldAction::Release => {
            ctrl.release_overscroll_recovery();
        }
    }
    if !device_gesture_in_flight(phase) && !is_direct_manipulation {
        ctrl.reset_overscroll_peak();
    }

    ctrl.last_event_time.set(Some(AnimInstant::now()));
    let offset_after = ctrl.scroll_offset.get();
    match ctrl.axis {
        ScrollAxis::Vertical => offset_after.y != offset.y,
        ScrollAxis::Horizontal => offset_after.x != offset.x,
    }
}

#[cfg(test)]
mod tests {
    use aimer_attribute::Vec2d;
    use aimer_events::element::{ScrollDeltaKind, TouchPhase};

    use super::{
        OverscrollHoldAction, apply_scroll_frame, device_gesture_in_flight, overscroll_hold_action,
        should_hold_overscroll_recovery,
    };
    use crate::ScrollAxis;
    use crate::scrollable::controller::ScrollState;

    #[test]
    fn only_a_terminating_scroll_phase_ends_the_device_gesture() {
        assert!(device_gesture_in_flight(TouchPhase::Started));
        assert!(device_gesture_in_flight(TouchPhase::Moved));
        assert!(!device_gesture_in_flight(TouchPhase::Ended));
        assert!(!device_gesture_in_flight(TouchPhase::Cancelled));
    }

    #[test]
    fn overscroll_hold_starts_only_when_crossing_the_edge() {
        let in_range = Vec2d::ZERO;
        let first_stretch = Vec2d { x: 0.0, y: 4.0 };
        let extended_stretch = Vec2d { x: 0.0, y: 5.0 };

        assert!(should_hold_overscroll_recovery(
            true,
            in_range,
            first_stretch,
            TouchPhase::Moved,
        ));
        assert!(!should_hold_overscroll_recovery(
            true,
            first_stretch,
            extended_stretch,
            TouchPhase::Moved,
        ));
        assert!(!should_hold_overscroll_recovery(
            false,
            in_range,
            first_stretch,
            TouchPhase::Moved,
        ));
        assert!(!should_hold_overscroll_recovery(
            true,
            in_range,
            first_stretch,
            TouchPhase::Ended,
        ));
    }

    #[test]
    fn contact_holds_the_stretch_whatever_the_frame_phase_says() {
        let in_range = Vec2d::ZERO;
        let stretch = Vec2d { x: 0.0, y: 4.0 };

        // A smoothing channel that drains mid-gesture reports `Ended` while the
        // fingers are still down; that frame must not start recovery.
        assert_eq!(
            overscroll_hold_action(true, false, stretch, stretch, TouchPhase::Ended),
            OverscrollHoldAction::HoldForContact
        );
        assert_eq!(
            overscroll_hold_action(true, true, in_range, stretch, TouchPhase::Moved),
            OverscrollHoldAction::HoldForContact
        );
        // A rejected delta still keeps the stretch the gesture already produced.
        assert_eq!(
            overscroll_hold_action(true, false, stretch, stretch, TouchPhase::Moved),
            OverscrollHoldAction::HoldForContact
        );
    }

    #[test]
    fn contact_back_inside_the_range_has_nothing_to_hold() {
        let stretch = Vec2d { x: 0.0, y: 4.0 };

        assert_eq!(
            overscroll_hold_action(true, true, stretch, Vec2d::ZERO, TouchPhase::Moved),
            OverscrollHoldAction::Release
        );
    }

    #[test]
    fn a_non_moving_active_axis_does_not_claim_a_scroll_frame() {
        let mut ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        ctrl.scroll_behavior.bouncy = false;
        ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 100.0 });

        assert!(!apply_scroll_frame(
            &ctrl,
            Vec2d { x: 20.0, y: 0.0 },
            ScrollDeltaKind::Pixel,
            TouchPhase::Moved,
            false,
        ));
        assert_eq!(ctrl.scroll_offset.get(), Vec2d::ZERO);

        ctrl.axis = ScrollAxis::Horizontal;
        ctrl.cached_max_scroll.set(Vec2d { x: 100.0, y: 0.0 });
        ctrl.set_scroll_offset(Vec2d { x: -100.0, y: 0.0 });
        assert!(!apply_scroll_frame(
            &ctrl,
            Vec2d { x: -20.0, y: 40.0 },
            ScrollDeltaKind::Pixel,
            TouchPhase::Moved,
            false,
        ));
    }

    #[test]
    fn bouncy_edge_motion_remains_owned_by_the_child() {
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        ctrl.cached_viewport.set((100.0, 100.0));

        assert!(apply_scroll_frame(
            &ctrl,
            Vec2d { x: 20.0, y: 10.0 },
            ScrollDeltaKind::Pixel,
            TouchPhase::Moved,
            true,
        ));
        assert_ne!(ctrl.scroll_offset.get().y, 0.0);
    }

    #[test]
    fn a_lifted_gesture_falls_back_to_the_grace_period() {
        let in_range = Vec2d::ZERO;
        let first_stretch = Vec2d { x: 0.0, y: 4.0 };
        let extended_stretch = Vec2d { x: 0.0, y: 5.0 };

        assert_eq!(
            overscroll_hold_action(false, true, in_range, first_stretch, TouchPhase::Moved),
            OverscrollHoldAction::HoldForGrace
        );
        // The inertial tail never renews the wait.
        assert_eq!(
            overscroll_hold_action(
                false,
                true,
                first_stretch,
                extended_stretch,
                TouchPhase::Moved,
            ),
            OverscrollHoldAction::Release
        );
    }
}
