//! Press, tap, double-tap and long-press recognition.
//!
//! Every function here is pure with respect to time: nothing calls
//! `AnimInstant::now()`, so a five-hundred-millisecond threshold is exercised by
//! handing in an instant rather than by sleeping.

use aimer_events::pointer::PointerInfo;
use aimer_utils::AnimInstant;

use crate::gesture::state::{GestureState, Press, Tap, moved_beyond_tap_tolerance};
use crate::gesture::{
    DOUBLE_TAP_TIMEOUT, GestureEvent, GestureMask, GestureOutput, LONG_PRESS_DURATION,
};

/// Begins tracking a press, reporting it immediately.
///
/// [`GestureEvent::TapDown`] fires before it is known whether this becomes a tap,
/// a drag or a long press, because that is precisely what a pressed visual needs:
/// waiting until the gesture resolves would mean the highlight appears on
/// release.
pub fn press(state: &mut GestureState, pointer: PointerInfo, now: AnimInstant) -> GestureOutput {
    state.press = Some(Press {
        pointer,
        down_at: now,
        long_pressed: false,
        long_press_last: None,
    });
    state.drag = None;

    GestureOutput::once(GestureEvent::TapDown { pointer })
}

/// Reports a long press once the threshold has passed, while the pointer is
/// still down.
///
/// Idempotent: a press produces the long press exactly once, however often this
/// is polled. A press that has already become a drag is not a long press — the
/// pointer plainly did not rest.
pub fn poll_long_press(
    state: &mut GestureState,
    now: AnimInstant,
    mask: GestureMask,
) -> GestureOutput {
    if !mask.intersects(GestureMask::LONG_PRESS) || state.is_dragging() {
        return GestureOutput::new();
    }

    let Some(press) = state.press.as_mut() else {
        return GestureOutput::new();
    };
    if press.long_pressed || now.duration_since(press.down_at) < LONG_PRESS_DURATION {
        return GestureOutput::new();
    }

    press.long_pressed = true;
    press.long_press_last = Some(press.pointer);
    let pointer = press.pointer;

    let mut output = GestureOutput::new();
    output.push(GestureEvent::LongPress { pointer });
    output.push(GestureEvent::LongPressStart { pointer });
    output
}

/// Whether a press was held long enough to be a long press but was never
/// reported as one.
///
/// Polling is a courtesy, not a guarantee — a detector in a static frame may
/// never be polled at all — so a late release has to be recognized on its own.
/// Without this, holding for a second and letting go would silently report a
/// plain tap.
pub fn is_unreported_long_press(press: &Press, now: AnimInstant, mask: GestureMask) -> bool {
    mask.intersects(GestureMask::LONG_PRESS)
        && !press.long_pressed
        && now.duration_since(press.down_at) >= LONG_PRESS_DURATION
}

/// Settles a release that neither a drag nor a long press claimed.
///
/// Either the pointer stayed within its device's tap tolerance — a tap, possibly
/// the second half of a double tap — or it did not, in which case the press
/// failed and is reported cancelled rather than silently forgotten.
///
/// Every gesture in `mask` is taken to own the press; see
/// [`tap_or_cancel_claiming`].
pub fn tap_or_cancel(
    state: &mut GestureState,
    press: &Press,
    pointer: PointerInfo,
    now: AnimInstant,
    mask: GestureMask,
) -> GestureOutput {
    tap_or_cancel_claiming(state, press, pointer, now, mask, mask)
}

/// [`tap_or_cancel`], told which gestures own the press.
///
/// A double tap that is owned replaces the second tap. One that is only
/// observed is reported *after* it, so the tap is not taken from a handler that
/// never asked for a double tap.
pub fn tap_or_cancel_claiming(
    state: &mut GestureState,
    press: &Press,
    pointer: PointerInfo,
    now: AnimInstant,
    mask: GestureMask,
    claims: GestureMask,
) -> GestureOutput {
    if moved_beyond_tap_tolerance(&press.pointer, &pointer) {
        state.last_tap = None;
        return GestureOutput::once(GestureEvent::TapCancel);
    }

    let mut output = GestureOutput::new();
    output.push(GestureEvent::TapUp { pointer });

    if mask.intersects(GestureMask::DOUBLE_TAP) && doubles(state.last_tap, pointer, now) {
        state.last_tap = None;
        if claims.intersects(GestureMask::DOUBLE_TAP) {
            output.push(GestureEvent::DoubleTap { pointer });
        } else {
            output.push(GestureEvent::Tap { pointer });
            output.push(GestureEvent::DoubleTap { pointer });
        }
        return output;
    }

    state.last_tap = Some(Tap { pointer, at: now });
    output.push(GestureEvent::Tap { pointer });
    output
}

/// Whether this tap lands soon enough, and close enough, to double the previous
/// one.
#[inline]
fn doubles(last_tap: Option<Tap>, pointer: PointerInfo, now: AnimInstant) -> bool {
    last_tap.is_some_and(|last| {
        now.duration_since(last.at) < DOUBLE_TAP_TIMEOUT
            && !moved_beyond_tap_tolerance(&last.pointer, &pointer)
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_attribute::position::Vec2d;

    use super::*;

    fn touch(x: f32, y: f32) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, 0)
    }

    fn held(pointer: PointerInfo, down_at: AnimInstant, long_pressed: bool) -> Press {
        Press {
            pointer,
            down_at,
            long_pressed,
            long_press_last: long_pressed.then_some(pointer),
        }
    }

    #[test]
    fn pressing_records_the_press_and_forgets_any_previous_drag() {
        let now = AnimInstant::now();
        let pointer = touch(3.0, 4.0);
        let mut state = GestureState {
            drag: Some(crate::gesture::state::Drag {
                origin: pointer,
                last: pointer,
                started_at: now,
            }),
            ..Default::default()
        };

        let output = press(&mut state, pointer, now);

        assert_eq!(output.first(), Some(GestureEvent::TapDown { pointer }));
        assert_eq!(state.press.map(|press| press.pointer), Some(pointer));
        assert!(!state.is_dragging());
    }

    #[test]
    fn a_long_press_is_not_recognized_while_the_pointer_is_dragging() {
        let now = AnimInstant::now();
        let pointer = touch(0.0, 0.0);
        let mut state = GestureState {
            press: Some(held(pointer, now, false)),
            drag: Some(crate::gesture::state::Drag {
                origin: pointer,
                last: pointer,
                started_at: now,
            }),
            ..Default::default()
        };

        let output = poll_long_press(&mut state, now + LONG_PRESS_DURATION, GestureMask::ALL);

        assert!(output.is_empty(), "a moving pointer is not resting");
    }

    #[test]
    fn a_long_press_is_not_recognized_when_nobody_listens_for_one() {
        let now = AnimInstant::now();
        let pointer = touch(0.0, 0.0);
        let mut state = GestureState {
            press: Some(held(pointer, now, false)),
            ..Default::default()
        };

        let output = poll_long_press(&mut state, now + LONG_PRESS_DURATION, GestureMask::TAP);

        assert!(output.is_empty());
        assert!(!state.is_long_pressed());
    }

    #[test]
    fn an_already_reported_long_press_is_not_unreported() {
        let now = AnimInstant::now();
        let press_state = held(touch(0.0, 0.0), now, true);

        assert!(!is_unreported_long_press(
            &press_state,
            now + LONG_PRESS_DURATION,
            GestureMask::ALL
        ));
    }

    #[test]
    fn a_short_press_is_never_an_unreported_long_press() {
        let now = AnimInstant::now();
        let press_state = held(touch(0.0, 0.0), now, false);

        assert!(!is_unreported_long_press(
            &press_state,
            now + Duration::from_millis(100),
            GestureMask::ALL
        ));
        assert!(is_unreported_long_press(
            &press_state,
            now + LONG_PRESS_DURATION,
            GestureMask::ALL
        ));
    }

    #[test]
    fn a_release_outside_the_slop_cancels_and_forgets_the_previous_tap() {
        let now = AnimInstant::now();
        let down = touch(0.0, 0.0);
        let away = touch(40.0, 0.0);
        let mut state = GestureState {
            last_tap: Some(Tap {
                pointer: down,
                at: now,
            }),
            ..Default::default()
        };

        let output = tap_or_cancel(
            &mut state,
            &held(down, now, false),
            away,
            now + Duration::from_millis(20),
            GestureMask::ALL,
        );

        assert_eq!(
            output.iter().copied().collect::<Vec<_>>(),
            vec![GestureEvent::TapCancel]
        );
        assert!(
            state.last_tap.is_none(),
            "a failed press must not seed a double tap"
        );
    }

    #[test]
    fn a_tap_records_itself_so_the_next_one_can_double_it() {
        let now = AnimInstant::now();
        let pointer = touch(1.0, 1.0);
        let mut state = GestureState::default();

        tap_or_cancel(
            &mut state,
            &held(pointer, now, false),
            pointer,
            now,
            GestureMask::ALL,
        );

        assert_eq!(state.last_tap.map(|tap| tap.pointer), Some(pointer));
        assert!(doubles(state.last_tap, pointer, now));
    }

    #[test]
    fn a_double_tap_clears_the_record_so_a_third_tap_is_single() {
        let now = AnimInstant::now();
        let pointer = touch(1.0, 1.0);
        let mut state = GestureState {
            last_tap: Some(Tap { pointer, at: now }),
            ..Default::default()
        };

        let output = tap_or_cancel(
            &mut state,
            &held(pointer, now, false),
            pointer,
            now + Duration::from_millis(50),
            GestureMask::ALL,
        );

        assert_eq!(
            output.iter().copied().collect::<Vec<_>>(),
            vec![
                GestureEvent::TapUp { pointer },
                GestureEvent::DoubleTap { pointer },
            ]
        );
        assert!(
            state.last_tap.is_none(),
            "three taps are a double tap then a single, not two doubles"
        );
    }
}
