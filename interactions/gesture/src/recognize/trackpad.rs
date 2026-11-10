//! Gestures the platform recognizes itself: trackpad pinch and rotate, the
//! multi-finger pan, and smart zoom.
//!
//! A trackpad does not report contacts. The operating system watches the
//! fingers, decides they are pinching, and reports a *magnification*, so there
//! is no pair of positions to measure and the touch recognizer in [`super::scale`]
//! has nothing to work with. What arrives instead is an already-recognized
//! gesture with a phase, and this module turns it into the same
//! [`GestureEvent`]s a touch pinch produces — a consumer writes one
//! `on_scale` handler and gets both devices.
//!
//! The platform reports magnification and rotation as two independent gestures
//! that overlap freely (a twist usually pinches a little), so they share one
//! [`PlatformTransform`] session: it opens with the first of them and closes
//! with the last, and the consumer sees a single scale gesture.
//!
//! Every function is a pure transition over [`GestureState`], like the rest of
//! the recognizer.

use aimer_attribute::position::Vec2d;
use aimer_events::element::TouchPhase;

use crate::gesture::state::{GestureState, PlatformTransform};
use crate::gesture::{GestureEvent, GestureOutput};

/// The smallest factor one platform step may scale by.
///
/// A scale must stay positive: a step at or below `-100 %` would flip the content
/// through zero, and every consumer that divides by the scale would follow it
/// into infinity.
const MIN_STEP_FACTOR: f32 = 0.01;

/// `delta`, or `0.0` if the platform reported something that is not a number.
///
/// winit documents the pinch delta as possibly `NaN`; one such step must not
/// poison the accumulated scale for the rest of the gesture.
#[inline]
fn finite(delta: f32) -> f32 {
    if delta.is_finite() { delta } else { 0.0 }
}

/// The step of the transform session, as the consumer sees it.
#[inline]
fn update(session: &PlatformTransform, delta_scale: f32, delta_rotation: f32) -> GestureEvent {
    GestureEvent::ScaleUpdate {
        focal_x: session.focal.x,
        focal_y: session.focal.y,
        scale: session.scale,
        delta_scale,
        rotation: session.rotation,
        delta_rotation,
        focal_delta_x: 0.0,
        focal_delta_y: 0.0,
    }
}

/// The open transform session, opening one at `pos` first if there is none and
/// reporting that start into `output`.
fn session_at<'a>(
    state: &'a mut GestureState,
    pos: Vec2d,
    output: &mut GestureOutput,
) -> &'a mut PlatformTransform {
    state.platform_transform.get_or_insert_with(|| {
        output.push(GestureEvent::ScaleStart {
            focal_x: pos.x,
            focal_y: pos.y,
        });
        PlatformTransform {
            focal: pos,
            scale: 1.0,
            rotation: 0.0,
            pinching: false,
            rotating: false,
        }
    })
}

/// Closes the transform session once neither of its gestures is running.
fn close_if_idle(state: &mut GestureState, output: &mut GestureOutput) {
    if state
        .platform_transform
        .is_some_and(|session| !session.pinching && !session.rotating)
    {
        state.platform_transform = None;
        output.push(GestureEvent::ScaleEnd);
    }
}

/// Advances the platform pinch by one step.
///
/// `delta` is the fractional change in scale since the previous step, as the
/// platform reports it: `0.1` grows the content by 10 %.
///
/// A touch pinch in progress owns the scale gesture; the platform step is
/// dropped rather than interleaved with it.
pub fn pinch(state: &mut GestureState, pos: Vec2d, delta: f32, phase: TouchPhase) -> GestureOutput {
    if state.pinch.is_some() {
        return GestureOutput::new();
    }

    let delta = finite(delta);
    let mut output = GestureOutput::new();

    if phase == TouchPhase::Started
        && state
            .platform_transform
            .is_some_and(|session| session.pinching)
    {
        // The previous pinch never ended: its end went to some other element, or
        // was lost with the window. Close it so the consumer is not left
        // believing in two gestures at once.
        if let Some(session) = state.platform_transform.as_mut() {
            session.pinching = false;
        }
        close_if_idle(state, &mut output);
    }

    if matches!(phase, TouchPhase::Started | TouchPhase::Moved) {
        session_at(state, pos, &mut output).pinching = true;
    } else if !state
        .platform_transform
        .is_some_and(|session| session.pinching)
    {
        // The end of a pinch this detector never saw begin.
        return output;
    }

    if delta != 0.0
        && let Some(session) = state.platform_transform.as_mut()
    {
        let factor = (1.0 + delta).max(MIN_STEP_FACTOR);
        session.scale *= factor;
        output.push(update(session, factor, 0.0));
    }

    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        if let Some(session) = state.platform_transform.as_mut() {
            session.pinching = false;
        }
        close_if_idle(state, &mut output);
    }
    output
}

/// Advances the platform rotation by one step.
///
/// `delta` is the change since the previous step in radians, clockwise
/// positive. The sign is already flipped from winit's counterclockwise
/// convention by the platform layer.
pub fn rotate(state: &mut GestureState, pos: Vec2d, delta: f32, phase: TouchPhase) -> GestureOutput {
    if state.pinch.is_some() {
        return GestureOutput::new();
    }

    let delta = finite(delta);
    let mut output = GestureOutput::new();

    if phase == TouchPhase::Started
        && state
            .platform_transform
            .is_some_and(|session| session.rotating)
    {
        if let Some(session) = state.platform_transform.as_mut() {
            session.rotating = false;
        }
        close_if_idle(state, &mut output);
    }

    if matches!(phase, TouchPhase::Started | TouchPhase::Moved) {
        session_at(state, pos, &mut output).rotating = true;
    } else if !state
        .platform_transform
        .is_some_and(|session| session.rotating)
    {
        return output;
    }

    if delta != 0.0
        && let Some(session) = state.platform_transform.as_mut()
    {
        session.rotation += delta;
        output.push(update(session, 1.0, delta));
    }

    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        if let Some(session) = state.platform_transform.as_mut() {
            session.rotating = false;
        }
        close_if_idle(state, &mut output);
    }
    output
}

/// Advances the platform multi-finger pan by one step.
///
/// `delta` is the movement since the previous step, in logical pixels.
pub fn pan(state: &mut GestureState, pos: Vec2d, delta: Vec2d, phase: TouchPhase) -> GestureOutput {
    let (delta_x, delta_y) = (finite(delta.x), finite(delta.y));
    let mut output = GestureOutput::new();

    if phase == TouchPhase::Started && state.platform_pan {
        // See `pinch`: the previous pan's end never arrived.
        output.push(GestureEvent::PanEnd);
        state.platform_pan = false;
    }

    if matches!(phase, TouchPhase::Started | TouchPhase::Moved) {
        if !state.platform_pan {
            state.platform_pan = true;
            output.push(GestureEvent::PanStart { x: pos.x, y: pos.y });
        }
    } else if !state.platform_pan {
        return output;
    }

    if delta_x != 0.0 || delta_y != 0.0 {
        output.push(GestureEvent::PanUpdate {
            x: pos.x,
            y: pos.y,
            delta_x,
            delta_y,
        });
    }

    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
        state.platform_pan = false;
        output.push(GestureEvent::PanEnd);
    }
    output
}

/// The platform's smart-zoom gesture at `pos`.
#[inline]
pub fn smart_zoom(pos: Vec2d) -> GestureOutput {
    GestureOutput::once(GestureEvent::SmartZoom { x: pos.x, y: pos.y })
}

/// Abandons the platform gestures in progress, reporting each one that had been
/// announced.
pub fn cancel(state: &mut GestureState, output: &mut GestureOutput) {
    if state.platform_transform.take().is_some() {
        output.push(GestureEvent::ScaleEnd);
    }
    if std::mem::take(&mut state.platform_pan) {
        output.push(GestureEvent::PanEnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Vec2d = Vec2d { x: 40.0, y: 60.0 };

    fn events(output: GestureOutput) -> Vec<GestureEvent> {
        output.into_iter().collect()
    }

    fn scale_update(scale: f32, delta_scale: f32, rotation: f32, delta_rotation: f32) -> GestureEvent {
        GestureEvent::ScaleUpdate {
            focal_x: AT.x,
            focal_y: AT.y,
            scale,
            delta_scale,
            rotation,
            delta_rotation,
            focal_delta_x: 0.0,
            focal_delta_y: 0.0,
        }
    }

    fn approx(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-5,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn a_pinch_opens_a_scale_gesture_at_the_cursor() {
        let mut state = GestureState::default();

        let started = events(pinch(&mut state, AT, 0.0, TouchPhase::Started));

        assert_eq!(
            started,
            vec![GestureEvent::ScaleStart {
                focal_x: 40.0,
                focal_y: 60.0
            }]
        );
        assert!(state.is_engaged());
    }

    #[test]
    fn the_scale_accumulates_multiplicatively_across_steps() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);

        let first = events(pinch(&mut state, AT, 0.5, TouchPhase::Moved));
        let second = events(pinch(&mut state, AT, 0.5, TouchPhase::Moved));

        assert_eq!(first, vec![scale_update(1.5, 1.5, 0.0, 0.0)]);
        assert_eq!(second, vec![scale_update(2.25, 1.5, 0.0, 0.0)]);
    }

    #[test]
    fn shrinking_steps_scale_the_content_down() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);

        let shrunk = events(pinch(&mut state, AT, -0.25, TouchPhase::Moved));

        assert_eq!(shrunk, vec![scale_update(0.75, 0.75, 0.0, 0.0)]);
    }

    #[test]
    fn a_first_step_that_carries_a_delta_starts_and_updates_together() {
        let mut state = GestureState::default();

        let started = events(pinch(&mut state, AT, 0.1, TouchPhase::Started));

        assert_eq!(started.len(), 2);
        assert!(matches!(started[0], GestureEvent::ScaleStart { .. }));
        assert!(matches!(started[1], GestureEvent::ScaleUpdate { .. }));
    }

    #[test]
    fn ending_the_pinch_ends_the_scale_gesture_and_forgets_it() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.1, TouchPhase::Started);

        let ended = events(pinch(&mut state, AT, 0.0, TouchPhase::Ended));

        assert_eq!(ended, vec![GestureEvent::ScaleEnd]);
        assert_eq!(state.platform_transform, None);
        assert!(!state.is_engaged());
    }

    #[test]
    fn a_cancelled_pinch_ends_like_a_finished_one() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);

        let cancelled = events(pinch(&mut state, AT, 0.0, TouchPhase::Cancelled));

        assert_eq!(cancelled, vec![GestureEvent::ScaleEnd]);
    }

    #[test]
    fn a_pinch_already_under_way_is_joined_rather_than_ignored() {
        let mut state = GestureState::default();

        let joined = events(pinch(&mut state, AT, 0.2, TouchPhase::Moved));

        assert_eq!(joined.len(), 2, "an implicit start, then the step");
        assert!(matches!(joined[0], GestureEvent::ScaleStart { .. }));
    }

    #[test]
    fn the_end_of_a_pinch_that_never_began_here_reports_nothing() {
        let mut state = GestureState::default();

        assert!(pinch(&mut state, AT, 0.0, TouchPhase::Ended).is_empty());
        assert!(!state.is_engaged());
    }

    // A pinch whose end went elsewhere would otherwise leave the consumer
    // believing it was still zooming when the next one starts.
    #[test]
    fn starting_over_a_pinch_that_never_ended_closes_the_old_one_first() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);
        pinch(&mut state, AT, 1.0, TouchPhase::Moved);

        let restarted = events(pinch(&mut state, AT, 0.0, TouchPhase::Started));

        assert!(matches!(restarted[0], GestureEvent::ScaleEnd));
        assert!(matches!(restarted[1], GestureEvent::ScaleStart { .. }));
        assert_eq!(
            state.platform_transform.map(|session| session.scale),
            Some(1.0),
            "the new pinch starts from scratch"
        );
    }

    // winit documents the pinch delta as possibly NaN. One bad step must not turn
    // the accumulated scale into NaN for the rest of the gesture.
    #[test]
    fn a_non_numeric_step_is_treated_as_no_movement() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);
        pinch(&mut state, AT, 0.5, TouchPhase::Moved);

        assert!(pinch(&mut state, AT, f32::NAN, TouchPhase::Moved).is_empty());
        assert!(pinch(&mut state, AT, f32::INFINITY, TouchPhase::Moved).is_empty());

        let next = events(pinch(&mut state, AT, 0.0, TouchPhase::Ended));
        assert_eq!(next, vec![GestureEvent::ScaleEnd]);
    }

    #[test]
    fn a_step_cannot_scale_the_content_through_zero() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);

        let crushed = events(pinch(&mut state, AT, -3.0, TouchPhase::Moved));

        let Some(GestureEvent::ScaleUpdate {
            scale, delta_scale, ..
        }) = crushed.first().copied()
        else {
            panic!("expected an update, got {crushed:?}");
        };
        assert!(scale > 0.0 && delta_scale > 0.0);
    }

    #[test]
    fn a_rotation_reports_the_angle_and_leaves_the_scale_alone() {
        let mut state = GestureState::default();
        rotate(&mut state, AT, 0.0, TouchPhase::Started);

        let turned = events(rotate(&mut state, AT, 0.5, TouchPhase::Moved));
        let more = events(rotate(&mut state, AT, 0.25, TouchPhase::Moved));

        assert_eq!(turned, vec![scale_update(1.0, 1.0, 0.5, 0.5)]);
        assert_eq!(more, vec![scale_update(1.0, 1.0, 0.75, 0.25)]);
    }

    #[test]
    fn a_pinch_and_a_rotation_share_one_scale_gesture() {
        let mut state = GestureState::default();

        let start_pinch = events(pinch(&mut state, AT, 0.0, TouchPhase::Started));
        let start_rotate = events(rotate(&mut state, AT, 0.0, TouchPhase::Started));
        let twist = events(rotate(&mut state, AT, 0.5, TouchPhase::Moved));
        let end_pinch = events(pinch(&mut state, AT, 0.0, TouchPhase::Ended));
        let end_rotate = events(rotate(&mut state, AT, 0.0, TouchPhase::Ended));

        assert_eq!(start_pinch.len(), 1, "the first gesture opens the session");
        assert!(
            start_rotate.is_empty(),
            "the second joins it instead of starting another"
        );
        assert_eq!(twist, vec![scale_update(1.0, 1.0, 0.5, 0.5)]);
        assert!(
            end_pinch.is_empty(),
            "the session stays open while the rotation runs"
        );
        assert_eq!(end_rotate, vec![GestureEvent::ScaleEnd]);
    }

    #[test]
    fn a_pinch_and_a_rotation_accumulate_into_the_same_update() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);
        rotate(&mut state, AT, 0.0, TouchPhase::Started);
        pinch(&mut state, AT, 1.0, TouchPhase::Moved);

        let twist = events(rotate(&mut state, AT, 0.5, TouchPhase::Moved));

        let Some(GestureEvent::ScaleUpdate {
            scale, rotation, ..
        }) = twist.first().copied()
        else {
            panic!("expected an update, got {twist:?}");
        };
        approx(scale, 2.0);
        approx(rotation, 0.5);
    }

    #[test]
    fn a_platform_pinch_stands_down_while_two_fingers_are_pinching() {
        use crate::gesture::state::Pinch;

        let mut state = GestureState {
            pinch: Some(Pinch {
                initial_distance: 100.0,
                scale: 1.0,
                angle: 0.0,
                rotation: 0.0,
                focal: AT,
            }),
            ..Default::default()
        };

        assert!(pinch(&mut state, AT, 0.5, TouchPhase::Moved).is_empty());
        assert!(rotate(&mut state, AT, 0.5, TouchPhase::Moved).is_empty());
        assert_eq!(state.platform_transform, None);
    }

    #[test]
    fn a_pan_runs_start_update_end() {
        let mut state = GestureState::default();

        let started = events(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started));
        let moved = events(pan(
            &mut state,
            AT,
            Vec2d { x: 3.0, y: -4.0 },
            TouchPhase::Moved,
        ));
        let ended = events(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Ended));

        assert_eq!(started, vec![GestureEvent::PanStart { x: 40.0, y: 60.0 }]);
        assert_eq!(
            moved,
            vec![GestureEvent::PanUpdate {
                x: 40.0,
                y: 60.0,
                delta_x: 3.0,
                delta_y: -4.0
            }]
        );
        assert_eq!(ended, vec![GestureEvent::PanEnd]);
        assert!(!state.is_engaged());
    }

    #[test]
    fn a_pan_that_does_not_move_reports_no_update() {
        let mut state = GestureState::default();
        pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started);

        assert!(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Moved).is_empty());
    }

    #[test]
    fn a_pan_that_never_began_here_cannot_end() {
        let mut state = GestureState::default();

        assert!(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Ended).is_empty());
        assert!(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Cancelled).is_empty());
    }

    #[test]
    fn starting_a_pan_over_one_that_never_ended_closes_the_old_one() {
        let mut state = GestureState::default();
        pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started);

        let restarted = events(pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started));

        assert_eq!(
            restarted,
            vec![
                GestureEvent::PanEnd,
                GestureEvent::PanStart { x: 40.0, y: 60.0 }
            ]
        );
    }

    #[test]
    fn a_non_numeric_pan_step_is_treated_as_no_movement() {
        let mut state = GestureState::default();
        pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started);

        let nan = Vec2d {
            x: f32::NAN,
            y: f32::INFINITY,
        };

        assert!(pan(&mut state, AT, nan, TouchPhase::Moved).is_empty());
    }

    #[test]
    fn smart_zoom_reports_where_it_happened() {
        assert_eq!(
            events(smart_zoom(AT)),
            vec![GestureEvent::SmartZoom { x: 40.0, y: 60.0 }]
        );
    }

    #[test]
    fn cancelling_ends_every_platform_gesture_that_was_announced() {
        let mut state = GestureState::default();
        pinch(&mut state, AT, 0.0, TouchPhase::Started);
        pan(&mut state, AT, Vec2d::ZERO, TouchPhase::Started);

        let mut output = GestureOutput::new();
        cancel(&mut state, &mut output);

        assert_eq!(
            events(output),
            vec![GestureEvent::ScaleEnd, GestureEvent::PanEnd]
        );
        assert!(!state.is_engaged());
    }

    #[test]
    fn cancelling_with_nothing_running_reports_nothing() {
        let mut state = GestureState::default();
        let mut output = GestureOutput::new();

        cancel(&mut state, &mut output);

        assert!(output.is_empty());
    }
}
