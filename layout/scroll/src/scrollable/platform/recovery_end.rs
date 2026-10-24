use aimer_attribute::position::Vec2d;
use aimer_events::element::TouchPhase;

use crate::scrollable::controller::ScrollState;

/// Terminates the scroll gesture whose bouncy edge has just recovered.
///
/// Native window systems report the end of a gesture themselves and stop
/// delivering deltas once the user lifts, so a settled recovery is simply the
/// end of an animation and nothing has to be synthesized.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub(crate) fn finish_overscroll_recovery(_ctrl: &ScrollState, _offset: Vec2d) {}

/// Terminates the scroll gesture whose bouncy edge has just recovered.
///
/// A browser keeps delivering the momentum it accumulated long after the
/// fingers left the trackpad, and the platform layer has to keep the
/// synthesized gesture open while they arrive. Once the stretched edge has
/// sprung back, those leftover deltas would push the content past the edge
/// again and the viewport would bounce a second time.
///
/// On the frame the recovery lands, the gesture is therefore closed the way a
/// platform that reports lifts would close it — a [`TouchPhase::Ended`] scroll
/// frame through the ordinary
/// [`apply_scroll_frame`](crate::scrollable::scroll_frame::apply_scroll_frame)
/// path, so hold, peak and contact are cleared by their single owner — and the
/// distance still queued in the momentum and spring integrators is dropped.
/// What the browser delivers afterwards is discarded by
/// [`scroll_frame_dropped`].
///
/// Call once per drawn frame with the offset that frame settled on.
#[cfg(target_arch = "wasm32")]
pub(crate) fn finish_overscroll_recovery(ctrl: &ScrollState, offset: Vec2d) {
    use aimer_events::element::ScrollDeltaKind;
    use aimer_utils::AnimInstant;

    let stretch = ctrl.overscroll_distance(offset);
    let stretched = stretch.x != 0.0 || stretch.y != 0.0;
    let held = stretched && ctrl.overscroll_recovery_held_at(AnimInstant::now());
    if !ctrl.web_recovery_end.observe_frame(stretched, held) {
        return;
    }

    ctrl.pointer_velocity.set(Vec2d::ZERO);
    ctrl.spring_velocity.set(Vec2d::ZERO);
    ctrl.momentum_start_time.set(None);
    ctrl.clear_velocity_history();
    ctrl.cancel_fling();

    crate::scrollable::scroll_frame::apply_scroll_frame(
        ctrl,
        Vec2d::ZERO,
        ScrollDeltaKind::Pixel,
        TouchPhase::Ended,
        false,
    );
    ctrl.web_recovery_end.drain_tail();
}

/// Whether an arriving scroll frame must be discarded instead of scrolled.
///
/// Native platforms deliver exactly the deltas the user produced, so every
/// frame is applied.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub(crate) fn scroll_frame_dropped(
    _ctrl: &ScrollState,
    _is_direct_manipulation: bool,
    _phase: TouchPhase,
) -> bool {
    false
}

/// Whether an arriving scroll frame must be discarded instead of scrolled.
///
/// Everything the browser still has queued for a gesture that
/// [`finish_overscroll_recovery`] already ended belongs to a scroll the user
/// stopped driving, so it is dropped rather than allowed to stretch the edge
/// again. A frame that carries contact again (the delta grew back, so the user
/// is pushing) or any phase the platform reports itself hands the content back
/// at once.
#[cfg(target_arch = "wasm32")]
#[inline]
pub(crate) fn scroll_frame_dropped(
    ctrl: &ScrollState,
    is_direct_manipulation: bool,
    phase: TouchPhase,
) -> bool {
    ctrl.web_recovery_end
        .drops_frame(is_direct_manipulation, phase)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::TouchPhase;

    use super::{finish_overscroll_recovery, scroll_frame_dropped};
    use crate::scrollable::controller::ScrollState;

    /// A native platform ends its own gestures and stops delivering once the
    /// user lifts, so nothing here may synthesize an end or swallow a delta —
    /// the browser-only path must stay out of the native frame loop entirely.
    #[test]
    fn a_native_recovery_neither_ends_a_gesture_nor_drops_a_frame() {
        let ctrl = ScrollState::for_test_at(Vec2d { x: 0.0, y: 40.0 });
        ctrl.pointer_velocity.set(Vec2d { x: 0.0, y: -600.0 });

        for _ in 0..4 {
            finish_overscroll_recovery(&ctrl, ctrl.scroll_offset.get());
        }

        assert_eq!(
            ctrl.pointer_velocity.get().y,
            -600.0,
            "a native fling keeps the distance it still owes"
        );
        for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended] {
            for contact in [true, false] {
                assert!(!scroll_frame_dropped(&ctrl, contact, phase));
            }
        }
    }
}
