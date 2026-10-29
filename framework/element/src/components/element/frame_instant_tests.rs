//! Every visit of an element within one frame samples the same instant.

use super::*;

use std::time::Duration;

#[test]
fn every_sample_in_a_frame_is_the_instant_the_frame_began() {
    begin_paint_frame(100, 100);
    let first = frame_instant();
    std::thread::sleep(Duration::from_millis(3));
    assert_eq!(frame_instant(), first, "a later visit of the same frame saw a later time");
    let _ = take_paint_frame_damage(100, 100);
}

#[test]
fn the_next_frame_starts_a_new_instant() {
    begin_paint_frame(100, 100);
    let first = frame_instant();
    let _ = take_paint_frame_damage(100, 100);
    std::thread::sleep(Duration::from_millis(3));

    begin_paint_frame(100, 100);
    assert!(frame_instant() > first, "the instant did not move on with the frame");
    let _ = take_paint_frame_damage(100, 100);
}

#[test]
fn between_frames_the_instant_is_now() {
    let _ = take_paint_frame_damage(100, 100);
    let before = aimer_utils::AnimInstant::now();
    let sampled = frame_instant();
    assert!(sampled >= before, "outside a frame there is no instant to hold on to");
}
