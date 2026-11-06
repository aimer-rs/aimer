use std::time::Duration;

use crate::playback::Playback;

fn secs(s: f32) -> Duration {
    Duration::from_secs_f32(s)
}

/// 60 frames at 30 fps: two seconds.
fn two_seconds() -> Playback {
    Playback::new(0.0, 60.0, 30.0).playing(true)
}

#[test]
fn starts_at_the_first_frame() {
    let p = two_seconds();
    assert_eq!(p.frame(), 0.0);
    assert_eq!(p.duration(), 2.0);
    assert_eq!(Playback::new(30.0, 90.0, 30.0).frame(), 30.0);
}

#[test]
fn advances_in_real_time() {
    let mut p = two_seconds();
    let tick = p.advance(secs(0.5));
    assert_eq!(p.frame(), 15.0);
    assert!(tick.changed && tick.wants_frame);
}

#[test]
fn a_range_that_does_not_start_at_zero_is_offset() {
    let mut p = Playback::new(30.0, 90.0, 30.0).playing(true);
    p.advance(secs(1.0));
    assert_eq!(p.frame(), 60.0);
}

#[test]
fn looping_wraps_to_the_start() {
    let mut p = two_seconds().looping(true);
    p.advance(secs(2.5));
    assert!((p.frame() - 15.0).abs() < 1e-3);
    let tick = p.advance(secs(0.0));
    assert!(tick.wants_frame, "a loop never finishes");
    p.advance(secs(1.5));
    assert!(p.frame().abs() < 1e-3, "an exact lap lands on the first frame");
}

#[test]
fn without_looping_it_stops_on_the_last_frame() {
    let mut p = two_seconds();
    let tick = p.advance(secs(5.0));
    assert!(tick.changed);
    assert!(!tick.wants_frame);
    assert!(!p.is_playing());
    // The out point itself is not shown, so the last visible frame is the one before it.
    assert!(p.frame() < 60.0 && p.frame() > 59.99, "{}", p.frame());
    let after = p.advance(secs(1.0));
    assert!(!after.changed && !after.wants_frame);
}

#[test]
fn speed_scales_time_and_negative_speed_runs_backwards() {
    let mut fast = two_seconds().speed(2.0);
    fast.advance(secs(0.5));
    assert_eq!(fast.frame(), 30.0);

    let mut back = two_seconds().looping(true).speed(-1.0);
    back.advance(secs(0.5));
    assert!((back.frame() - 45.0).abs() < 1e-3, "wraps to the end then runs back: {}", back.frame());

    let mut stop = two_seconds().speed(-1.0);
    let tick = stop.advance(secs(1.0));
    assert!(!tick.wants_frame && !stop.is_playing(), "backwards without a loop stops at the start");
    assert_eq!(stop.frame(), 0.0);
}

#[test]
fn a_paused_playhead_does_not_move_or_ask_for_frames() {
    let mut p = Playback::new(0.0, 60.0, 30.0);
    let tick = p.advance(secs(1.0));
    assert_eq!((tick.changed, tick.wants_frame), (false, false));
    assert_eq!(p.frame(), 0.0);
}

#[test]
fn zero_elapsed_time_changes_nothing_but_keeps_asking() {
    let mut p = two_seconds();
    let tick = p.advance(Duration::ZERO);
    assert_eq!((tick.changed, tick.wants_frame), (false, true));
}

#[test]
fn a_held_or_broken_speed_never_moves_the_playhead() {
    for speed in [0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut p = two_seconds().speed(speed);
        let tick = p.advance(secs(1.0));
        assert_eq!((tick.changed, tick.wants_frame), (false, false), "speed {speed}");
        assert_eq!(p.frame(), 0.0);
    }
}

#[test]
fn progress_seeks_within_the_range() {
    let mut p = Playback::new(0.0, 60.0, 30.0);
    p.set_progress(0.25);
    assert_eq!(p.frame(), 15.0);
    p.set_progress(5.0);
    assert!(p.frame() > 59.99 && p.frame() < 60.0, "clamped to the last frame");
    p.set_progress(-1.0);
    assert_eq!(p.frame(), 0.0);
    p.set_progress(f32::NAN);
    assert_eq!(p.frame(), 0.0, "NaN is ignored");
    assert!(!p.is_playing(), "seeking does not start playback");
}

#[test]
fn a_single_frame_animation_is_still_and_never_asks_for_frames() {
    let mut p = Playback::new(10.0, 10.0, 30.0).playing(true).looping(true);
    let tick = p.advance(secs(1.0));
    assert_eq!((tick.changed, tick.wants_frame), (false, false));
    assert_eq!(p.frame(), 10.0);
    assert_eq!(p.duration(), 0.0);
}

#[test]
fn degenerate_ranges_and_rates_stay_finite() {
    for (start, end, rate) in [(60.0, 0.0, 30.0), (0.0, 60.0, 0.0), (0.0, f32::NAN, 30.0), (0.0, 60.0, f32::NAN)] {
        let mut p = Playback::new(start, end, rate).playing(true).looping(true);
        p.advance(secs(1.0));
        assert!(p.frame().is_finite(), "{start} {end} {rate}");
        assert!(p.duration().is_finite() && p.duration() >= 0.0);
    }
}

#[test]
fn a_huge_step_stays_finite() {
    let mut p = two_seconds().looping(true);
    p.advance(Duration::from_secs(10_000_000));
    assert!(p.frame().is_finite() && p.frame() >= 0.0 && p.frame() < 60.0);
}

#[test]
fn adopting_a_position_keeps_the_new_settings() {
    let mut old = two_seconds();
    old.advance(secs(1.0));
    let mut new = Playback::new(0.0, 60.0, 30.0).speed(3.0).looping(true);
    new.adopt_position(&old);
    assert_eq!(new.frame(), 30.0);
    assert!(!new.is_playing(), "settings, including paused, are the new ones");

    // A shorter animation clamps the carried position.
    let mut short = Playback::new(0.0, 15.0, 30.0);
    short.adopt_position(&old);
    assert!(short.frame() < 15.0 && short.frame() > 14.99);
}
