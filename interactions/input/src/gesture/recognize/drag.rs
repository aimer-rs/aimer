//! Drag and swipe recognition.
//!
//! A drag begins when movement crosses the slop for the pointer's device, and a
//! swipe is a drag that turned out to be fast and short — reported *alongside*
//! the drag's end, never instead of it.

use aimer_events::pointer::PointerInfo;
use aimer_utils::AnimInstant;

use crate::gesture::state::{Drag, GestureState, moved_beyond_slop};
use crate::gesture::{
    GestureEvent, GestureMask, GestureOutput, SWIPE_MAX_DURATION_MS, SWIPE_VELOCITY_THRESHOLD,
    SwipeDirection,
};

/// Advances whatever a movement means for the press in progress.
///
/// Up to two things at once, because a held long press that starts moving is
/// both: the long-press stream keeps reporting movement for a selection handle,
/// while the drag stream starts for anything built on drag — which is how a
/// long-press-then-drag works without a gesture arena to negotiate it.
pub fn moved(
    state: &mut GestureState,
    pointer: PointerInfo,
    now: AnimInstant,
    mask: GestureMask,
) -> GestureOutput {
    let Some(press) = state.press else {
        return GestureOutput::new();
    };
    let mut output = GestureOutput::new();

    if press.long_pressed && mask.intersects(GestureMask::LONG_PRESS) {
        let last = press.long_press_last.unwrap_or(press.pointer);
        output.push(GestureEvent::LongPressMoveUpdate {
            pointer,
            delta_x: pointer.pos.x - last.pos.x,
            delta_y: pointer.pos.y - last.pos.y,
        });
        if let Some(press) = state.press.as_mut() {
            press.long_press_last = Some(pointer);
        }
    }

    if !mask.intersects(GestureMask::DRAG.union(GestureMask::SWIPE)) {
        return output;
    }

    if let Some(drag) = state.drag.as_mut() {
        let delta_x = pointer.pos.x - drag.last.pos.x;
        let delta_y = pointer.pos.y - drag.last.pos.y;
        drag.last = pointer;
        output.push(GestureEvent::DragUpdate {
            pointer,
            delta_x,
            delta_y,
        });
    } else if moved_beyond_slop(&press.pointer, &pointer) {
        state.drag = Some(Drag {
            origin: press.pointer,
            last: pointer,
            started_at: now,
        });
        // Where the press began, not where the slop was crossed: reporting the
        // crossing point would make the dragged thing jump by the slop distance
        // the moment it picked up.
        output.push(GestureEvent::DragStart {
            pointer: press.pointer,
        });
    }

    output
}

/// Concludes a drag: always an end, and a swipe as well if it was a flick.
pub fn end(
    drag: &Drag,
    pointer: PointerInfo,
    now: AnimInstant,
    mask: GestureMask,
) -> GestureOutput {
    let mut output = GestureOutput::once(GestureEvent::DragEnd { pointer });

    if mask.intersects(GestureMask::SWIPE)
        && let Some(swipe) = swipe(drag, pointer, now)
    {
        output.push(swipe);
    }

    output
}

/// A [`GestureEvent::Swipe`] if the drag was fast enough, and brief enough, to
/// read as a flick rather than as deliberate dragging.
fn swipe(drag: &Drag, pointer: PointerInfo, now: AnimInstant) -> Option<GestureEvent> {
    let elapsed = now.duration_since(drag.started_at);
    if elapsed.as_millis() as u64 > SWIPE_MAX_DURATION_MS {
        return None;
    }

    let seconds = elapsed.as_secs_f32();
    if seconds <= 0.0 {
        return None;
    }

    let delta_x = pointer.pos.x - drag.origin.pos.x;
    let delta_y = pointer.pos.y - drag.origin.pos.y;
    let velocity = (delta_x * delta_x + delta_y * delta_y).sqrt() / seconds;
    if velocity <= SWIPE_VELOCITY_THRESHOLD {
        return None;
    }

    Some(GestureEvent::Swipe {
        direction: direction(delta_x, delta_y),
        velocity_x: delta_x / seconds,
        velocity_y: delta_y / seconds,
    })
}

/// The dominant axis of a movement, and its sign along that axis.
#[inline]
fn direction(delta_x: f32, delta_y: f32) -> SwipeDirection {
    if delta_x.abs() > delta_y.abs() {
        if delta_x > 0.0 {
            SwipeDirection::Right
        } else {
            SwipeDirection::Left
        }
    } else if delta_y > 0.0 {
        SwipeDirection::Down
    } else {
        SwipeDirection::Up
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aimer_attribute::position::Vec2d;

    use super::*;
    use crate::gesture::state::Press;

    fn touch(x: f32, y: f32) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, 0)
    }

    fn dragging(origin: PointerInfo, last: PointerInfo, started_at: AnimInstant) -> Drag {
        Drag {
            origin,
            last,
            started_at,
        }
    }

    fn pressed(pointer: PointerInfo, now: AnimInstant) -> Press {
        Press {
            pointer,
            down_at: now,
            long_pressed: false,
            long_press_last: None,
        }
    }

    #[test]
    fn the_dominant_axis_decides_the_direction() {
        assert_eq!(direction(10.0, 1.0), SwipeDirection::Right);
        assert_eq!(direction(-10.0, 1.0), SwipeDirection::Left);
        assert_eq!(direction(1.0, 10.0), SwipeDirection::Down);
        assert_eq!(direction(1.0, -10.0), SwipeDirection::Up);
    }

    // A perfectly diagonal movement has no dominant horizontal axis, so it falls
    // to the vertical branch rather than being undefined.
    #[test]
    fn a_perfectly_diagonal_movement_resolves_vertically() {
        assert_eq!(direction(10.0, 10.0), SwipeDirection::Down);
        assert_eq!(direction(10.0, -10.0), SwipeDirection::Up);
    }

    #[test]
    fn a_drag_that_took_too_long_is_never_a_swipe() {
        let started_at = AnimInstant::now();
        let drag = dragging(touch(0.0, 0.0), touch(500.0, 0.0), started_at);

        assert!(
            swipe(
                &drag,
                touch(500.0, 0.0),
                started_at + Duration::from_millis(SWIPE_MAX_DURATION_MS + 1)
            )
            .is_none(),
            "however fast it was, a long drag is not a flick"
        );
    }

    // Two events can share a timestamp — a synthesized move and release in the
    // same frame — and dividing by that zero would produce an infinite velocity.
    #[test]
    fn a_drag_with_no_elapsed_time_is_not_a_swipe() {
        let started_at = AnimInstant::now();
        let drag = dragging(touch(0.0, 0.0), touch(500.0, 0.0), started_at);

        assert!(swipe(&drag, touch(500.0, 0.0), started_at).is_none());
    }

    #[test]
    fn a_swipe_velocity_is_measured_from_where_the_press_began() {
        let started_at = AnimInstant::now();
        let drag = dragging(touch(0.0, 0.0), touch(50.0, 0.0), started_at);

        let swipe = swipe(
            &drag,
            touch(100.0, 0.0),
            started_at + Duration::from_millis(100),
        )
        .expect("1000 px/s is well over the threshold");

        assert_eq!(
            swipe,
            GestureEvent::Swipe {
                direction: SwipeDirection::Right,
                velocity_x: 1000.0,
                velocity_y: 0.0,
            }
        );
    }

    #[test]
    fn ending_a_drag_without_the_swipe_mask_reports_only_the_end() {
        let started_at = AnimInstant::now();
        let pointer = touch(200.0, 0.0);
        let drag = dragging(touch(0.0, 0.0), pointer, started_at);

        let output = end(
            &drag,
            pointer,
            started_at + Duration::from_millis(20),
            GestureMask::DRAG,
        );

        assert_eq!(
            output.iter().copied().collect::<Vec<_>>(),
            vec![GestureEvent::DragEnd { pointer }],
            "a flick is still a flick, but nobody asked about it"
        );
    }

    #[test]
    fn a_movement_with_no_press_is_ignored() {
        let mut state = GestureState::default();

        let output = moved(
            &mut state,
            touch(50.0, 50.0),
            AnimInstant::now(),
            GestureMask::ALL,
        );

        assert!(output.is_empty());
        assert!(!state.is_dragging());
    }

    #[test]
    fn a_drag_is_not_started_when_only_a_tap_is_listened_for() {
        let now = AnimInstant::now();
        let down = touch(0.0, 0.0);
        let mut state = GestureState {
            press: Some(pressed(down, now)),
            ..Default::default()
        };

        let output = moved(&mut state, touch(100.0, 0.0), now, GestureMask::TAP);

        assert!(output.is_empty());
        assert!(
            !state.is_dragging(),
            "no state is kept for a gesture nobody wants"
        );
    }

    // A swipe listener alone is enough: a flick has to be tracked as a drag
    // internally, even when only its final velocity is wanted.
    #[test]
    fn listening_only_for_swipes_still_tracks_the_drag() {
        let now = AnimInstant::now();
        let down = touch(0.0, 0.0);
        let mut state = GestureState {
            press: Some(pressed(down, now)),
            ..Default::default()
        };

        let output = moved(&mut state, touch(100.0, 0.0), now, GestureMask::SWIPE);

        assert_eq!(
            output.first(),
            Some(GestureEvent::DragStart { pointer: down })
        );
        assert!(state.is_dragging());
    }
}
