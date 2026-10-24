use aimer_attribute::position::Vec2d;
use aimer_events::element::TouchPhase;

use crate::scrollable::controller::ScrollState;

/// Whether a delivered scroll frame counts as the user physically driving the
/// device.
///
/// Native window systems report contact per frame and their value is
/// authoritative, so it is handed straight through.
#[cfg(not(target_arch = "wasm32"))]
#[inline]
pub(crate) fn device_contact_for_frame(
    _ctrl: &ScrollState,
    is_direct_manipulation: bool,
    _delta: Vec2d,
    _phase: TouchPhase,
) -> bool {
    is_direct_manipulation
}

/// Whether a delivered scroll frame counts as the user physically driving the
/// device.
///
/// A browser never reports a lift, so the platform layer keeps claiming
/// contact for the whole synthesized gesture — including the momentum the
/// browser delivers after the fingers left the trackpad. A stretched edge held
/// by contact would stay frozen until the last tail event arrives, which on a
/// hard flick is most of a second.
///
/// Contact is therefore re-derived from the shape of the delta stream by
/// [`WebOverscrollDecay`](crate::scrollable::web_overscroll::WebOverscrollDecay),
/// so bouncy recovery starts as soon as the tail decays. `delta` must be the
/// raw device distance of this frame, before overscroll resistance shrinks it.
#[cfg(target_arch = "wasm32")]
#[inline]
pub(crate) fn device_contact_for_frame(
    ctrl: &ScrollState,
    is_direct_manipulation: bool,
    delta: Vec2d,
    phase: TouchPhase,
) -> bool {
    crate::scrollable::web_overscroll::web_device_contact(
        &ctrl.web_overscroll_decay,
        is_direct_manipulation,
        delta,
        phase,
    )
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::element::TouchPhase;

    use super::device_contact_for_frame;
    use crate::scrollable::controller::ScrollState;

    /// A native window system owns the contact signal, so nothing between the
    /// event and the overscroll hold may reinterpret it — the web-only decay
    /// analysis must stay out of this path entirely.
    #[test]
    fn a_native_frame_reports_the_contact_the_platform_delivered() {
        let ctrl = ScrollState::for_test_at(Vec2d::ZERO);
        let steady = Vec2d { x: 0.0, y: 40.0 };
        let spent = Vec2d { x: 0.0, y: 0.2 };

        for reported in [true, false] {
            for delta in [steady, spent] {
                for phase in [TouchPhase::Started, TouchPhase::Moved, TouchPhase::Ended] {
                    assert_eq!(
                        device_contact_for_frame(&ctrl, reported, delta, phase),
                        reported
                    );
                }
            }
        }
    }
}
