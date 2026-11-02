//! Pinch, rotate and two-finger pan recognition from two touch contacts.
//!
//! One pair of contacts describes a whole similarity transform — how far apart
//! they are (scale), the direction of the line between them (rotation) and where
//! their midpoint is (translation) — so all three are reported together by one
//! [`GestureEvent::ScaleUpdate`] rather than by three recognizers racing to claim
//! the same two fingers.
//!
//! Measured between the two contacts that arrived first, which is why
//! [`crate::gesture::state::ActiveTouches`] preserves arrival order: reading the
//! pair from a hash map, as this once did, could swap the reference points
//! between two moves of the same pinch and make the reported scale jump.

use std::f32::consts::{PI, TAU};

use aimer_events::pointer::PointerInfo;

use crate::gesture::state::{GestureState, Pinch};
use crate::gesture::{GestureEvent, GestureOutput};

/// The direction of the line from `first` to `second`, in radians.
///
/// Screen coordinates grow downwards, so an increasing angle is a **clockwise**
/// turn as the user sees it.
#[inline]
fn angle_between(first: &PointerInfo, second: &PointerInfo) -> f32 {
    (second.pos.y - first.pos.y).atan2(second.pos.x - first.pos.x)
}

/// `angle` folded into `(-π, π]`.
///
/// The line between two fingers crossing the ±π seam changes its measured angle
/// by almost a full turn while the hand turned a few degrees; folding the
/// difference is what keeps a rotation continuous across it.
#[inline]
fn shortest_turn(angle: f32) -> f32 {
    let mut turn = angle % TAU;
    if turn > PI {
        turn -= TAU;
    } else if turn <= -PI {
        turn += TAU;
    }
    turn
}

/// Starts a pinch from the two contacts currently down.
///
/// Reports nothing if a second contact is not actually down — the caller checked,
/// but the recognizer does not depend on it having done so.
pub fn begin(state: &mut GestureState) -> GestureOutput {
    let Some((first, second)) = state.touches.pinch_pair() else {
        return GestureOutput::new();
    };

    let focal = first.midpoint(&second);
    state.pinch = Some(Pinch {
        initial_distance: first.distance_to(&second),
        scale: 1.0,
        angle: angle_between(&first, &second),
        rotation: 0.0,
        focal: focal.pos,
    });

    GestureOutput::once(GestureEvent::ScaleStart {
        focal_x: focal.pos.x,
        focal_y: focal.pos.y,
    })
}

/// Reports how the two contacts have spread, turned and travelled since the
/// pinch began.
///
/// `scale` and `rotation` are relative to the start of the pinch; `delta_scale`,
/// `delta_rotation` and the focal delta are relative to the previous report,
/// which is what a transform wants to apply incrementally. `rotation` is in
/// radians, clockwise positive.
pub fn update(state: &mut GestureState) -> GestureOutput {
    let Some((first, second)) = state.touches.pinch_pair() else {
        return GestureOutput::new();
    };
    let Some(pinch) = state.pinch.as_mut() else {
        return GestureOutput::new();
    };
    // Two contacts at exactly the same point have no separation to scale
    // against, and dividing by it would report an infinite scale.
    if pinch.initial_distance <= 0.0 {
        return GestureOutput::new();
    }

    let scale = first.distance_to(&second) / pinch.initial_distance;
    let delta_scale = if pinch.scale > 0.0 {
        scale / pinch.scale
    } else {
        1.0
    };
    pinch.scale = scale;

    // Two contacts on the same point have no direction either; keep the last
    // known angle instead of folding `atan2(0, 0)` into the rotation.
    let angle = if first.distance_to(&second) > 0.0 {
        angle_between(&first, &second)
    } else {
        pinch.angle
    };
    let delta_rotation = shortest_turn(angle - pinch.angle);
    pinch.angle = angle;
    pinch.rotation += delta_rotation;

    let focal = first.midpoint(&second).pos;
    let focal_delta_x = focal.x - pinch.focal.x;
    let focal_delta_y = focal.y - pinch.focal.y;
    pinch.focal = focal;

    GestureOutput::once(GestureEvent::ScaleUpdate {
        focal_x: focal.x,
        focal_y: focal.y,
        scale,
        delta_scale,
        rotation: pinch.rotation,
        delta_rotation,
        focal_delta_x,
        focal_delta_y,
    })
}

/// Ends the pinch, because fewer than two contacts remain.
pub fn end(state: &mut GestureState) -> GestureOutput {
    state.clear_pinch();
    GestureOutput::once(GestureEvent::ScaleEnd)
}

#[cfg(test)]
mod tests {
    use aimer_attribute::position::Vec2d;
    use aimer_events::pointer::PointerInfo;

    use super::*;

    fn touch(x: f32, y: f32, id: u64) -> PointerInfo {
        PointerInfo::touch(Vec2d { x, y }, id)
    }

    fn pinching(first: PointerInfo, second: PointerInfo) -> GestureState {
        let mut state = GestureState::default();
        state.touches.insert(first);
        state.touches.insert(second);
        state
    }

    #[test]
    fn a_pinch_begins_at_the_midpoint_of_the_two_contacts() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 40.0, 2));

        let output = begin(&mut state);

        assert_eq!(
            output.first(),
            Some(GestureEvent::ScaleStart {
                focal_x: 50.0,
                focal_y: 20.0,
            })
        );
        assert_eq!(
            state.pinch.map(|pinch| pinch.scale),
            Some(1.0),
            "a pinch starts at its own scale"
        );
    }

    #[test]
    fn a_pinch_cannot_begin_with_one_contact() {
        let mut state = GestureState::default();
        state.touches.insert(touch(0.0, 0.0, 1));

        assert!(begin(&mut state).is_empty());
        assert!(state.pinch.is_none());
    }

    #[test]
    fn spreading_the_contacts_reports_the_scale_relative_to_the_start() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(200.0, 0.0, 2));
        let doubled = update(&mut state);

        assert_eq!(
            doubled.first(),
            Some(GestureEvent::ScaleUpdate {
                focal_x: 100.0,
                focal_y: 0.0,
                scale: 2.0,
                delta_scale: 2.0,
                rotation: 0.0,
                delta_rotation: 0.0,
                focal_delta_x: 50.0,
                focal_delta_y: 0.0,
            })
        );

        state.touches.insert(touch(300.0, 0.0, 2));
        let tripled = update(&mut state);

        assert_eq!(
            tripled.first(),
            Some(GestureEvent::ScaleUpdate {
                focal_x: 150.0,
                focal_y: 0.0,
                scale: 3.0,
                delta_scale: 1.5,
                rotation: 0.0,
                delta_rotation: 0.0,
                focal_delta_x: 50.0,
                focal_delta_y: 0.0,
            }),
            "the delta is against the previous report, the scale against the start"
        );
    }

    // Two contacts reported at the same point give a zero initial separation;
    // scaling against it would be a division by zero.
    #[test]
    fn two_contacts_at_the_same_point_report_no_scale() {
        let mut state = pinching(touch(50.0, 50.0, 1), touch(50.0, 50.0, 2));
        begin(&mut state);

        state.touches.insert(touch(150.0, 50.0, 2));

        assert!(update(&mut state).is_empty());
    }

    #[test]
    fn updating_without_a_pinch_in_progress_reports_nothing() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));

        assert!(update(&mut state).is_empty());
    }

    #[test]
    fn ending_a_pinch_forgets_it() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        let output = end(&mut state);

        assert_eq!(output.first(), Some(GestureEvent::ScaleEnd));
        assert!(state.pinch.is_none());
    }

    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-4,
            "expected {expected}, got {actual}"
        );
    }

    fn update_of(output: GestureOutput) -> (f32, f32, f32, f32, f32, f32) {
        let Some(GestureEvent::ScaleUpdate {
            scale,
            delta_scale,
            rotation,
            delta_rotation,
            focal_delta_x,
            focal_delta_y,
            ..
        }) = output.first()
        else {
            panic!("expected a scale update, got {output:?}");
        };
        (
            scale,
            delta_scale,
            rotation,
            delta_rotation,
            focal_delta_x,
            focal_delta_y,
        )
    }

    // Screen y grows downwards, so swinging the second finger from the right of
    // the first to below it is a clockwise turn as the user sees it.
    #[test]
    fn swinging_the_second_contact_below_the_first_is_a_clockwise_quarter_turn() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(0.0, 100.0, 2));
        let (scale, _, rotation, delta_rotation, _, _) = update_of(update(&mut state));

        approx(scale, 1.0);
        approx(rotation, std::f32::consts::FRAC_PI_2);
        approx(delta_rotation, std::f32::consts::FRAC_PI_2);
    }

    #[test]
    fn turning_the_other_way_reports_a_negative_rotation() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(0.0, -100.0, 2));
        let (_, _, rotation, _, _, _) = update_of(update(&mut state));

        approx(rotation, -std::f32::consts::FRAC_PI_2);
    }

    // The line between the fingers pointing left sits on the ±π seam. Turning a
    // few degrees across it changes the measured angle by nearly a full turn.
    #[test]
    fn a_turn_across_the_seam_stays_small() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(-100.0, -5.0, 2));
        begin(&mut state);

        state.touches.insert(touch(-100.0, 5.0, 2));
        let (_, _, rotation, delta_rotation, _, _) = update_of(update(&mut state));

        let expected = 2.0 * (5.0f32 / 100.0).atan();
        approx(delta_rotation, -expected);
        approx(rotation, -expected);
    }

    #[test]
    fn the_rotation_accumulates_across_updates() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(100.0, 100.0, 2));
        let (_, _, first, _, _, _) = update_of(update(&mut state));
        state.touches.insert(touch(0.0, 100.0, 2));
        let (_, _, second, delta, _, _) = update_of(update(&mut state));

        approx(first, std::f32::consts::FRAC_PI_4);
        approx(second, std::f32::consts::FRAC_PI_2);
        approx(delta, std::f32::consts::FRAC_PI_4);
    }

    #[test]
    fn dragging_both_contacts_together_is_a_pure_pan() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(10.0, 20.0, 1));
        state.touches.insert(touch(110.0, 20.0, 2));
        let (scale, delta_scale, rotation, _, focal_x, focal_y) = update_of(update(&mut state));

        approx(scale, 1.0);
        approx(delta_scale, 1.0);
        approx(rotation, 0.0);
        approx(focal_x, 10.0);
        approx(focal_y, 20.0);
    }

    #[test]
    fn the_focal_delta_is_measured_against_the_previous_report() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(100.0, 0.0, 2));
        begin(&mut state);

        state.touches.insert(touch(10.0, 0.0, 1));
        state.touches.insert(touch(110.0, 0.0, 2));
        update(&mut state);
        state.touches.insert(touch(14.0, 0.0, 1));
        state.touches.insert(touch(114.0, 0.0, 2));
        let (_, _, _, _, focal_x, _) = update_of(update(&mut state));

        approx(focal_x, 4.0);
    }

    // With no separation there is no direction. Folding atan2(0, 0) into the
    // rotation would swing it by whatever the previous angle was.
    #[test]
    fn contacts_on_the_same_point_keep_the_last_known_angle() {
        let mut state = pinching(touch(0.0, 0.0, 1), touch(0.0, 100.0, 2));
        begin(&mut state);

        state.touches.insert(touch(0.0, 0.0, 2));
        let (_, _, rotation, delta_rotation, _, _) = update_of(update(&mut state));

        approx(rotation, 0.0);
        approx(delta_rotation, 0.0);
    }
}
