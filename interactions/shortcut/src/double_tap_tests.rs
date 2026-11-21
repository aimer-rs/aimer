use std::time::{Duration, Instant};

use super::*;

const SHIFT: ChordMods = ChordMods::SHIFT;
const CTRL: ChordMods = ChordMods::CTRL;
const NONE: ChordMods = ChordMods::NONE;

fn ms(base: Instant, millis: u64) -> Instant {
    base + Duration::from_millis(millis)
}

#[test]
fn two_quick_taps_of_one_modifier_are_a_double_tap() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    assert_eq!(tap.modifiers_changed(SHIFT, ms(t0, 0)), None);
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 60)), None);
    assert_eq!(tap.modifiers_changed(SHIFT, ms(t0, 150)), None);
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 210)), Some(SHIFT));
}

#[test]
fn a_double_tap_is_spent_so_a_third_tap_starts_over() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    for (at, held) in [(0, SHIFT), (50, NONE), (100, SHIFT)] {
        tap.modifiers_changed(held, ms(t0, at));
    }
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), Some(SHIFT));
    tap.modifiers_changed(SHIFT, ms(t0, 200));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 250)), None);
}

#[test]
fn taps_that_are_too_far_apart_are_not_a_double_tap() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(NONE, ms(t0, 50));
    tap.modifiers_changed(SHIFT, ms(t0, 900));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 950)), None);
    // The late tap is the first of a new pair.
    tap.modifiers_changed(SHIFT, ms(t0, 1000));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 1050)), Some(SHIFT));
}

#[test]
fn a_modifier_that_is_held_is_not_a_tap() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    // Held for a shift-click, say, and then tapped: the long hold is no tap.
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(NONE, ms(t0, 2000));
    tap.modifiers_changed(SHIFT, ms(t0, 2050));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 2100)), None);
}

#[test]
fn a_key_between_the_taps_cancels_them() {
    // Typing a capital letter holds shift and releases it; two of them in a row are not a double tap.
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.interrupt();
    tap.modifiers_changed(NONE, ms(t0, 50));
    tap.modifiers_changed(SHIFT, ms(t0, 100));
    tap.interrupt();
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), None);
}

#[test]
fn a_key_between_two_clean_taps_cancels_the_first() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(NONE, ms(t0, 50));
    tap.interrupt();
    tap.modifiers_changed(SHIFT, ms(t0, 100));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), None);
}

#[test]
fn another_modifier_joining_cancels_the_tap() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(SHIFT | CTRL, ms(t0, 20));
    tap.modifiers_changed(NONE, ms(t0, 60));
    tap.modifiers_changed(SHIFT, ms(t0, 100));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), None);
}

#[test]
fn different_modifiers_do_not_pair_up() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(NONE, ms(t0, 50));
    tap.modifiers_changed(CTRL, ms(t0, 100));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), None);
}

#[test]
fn reset_forgets_a_tap_that_is_half_done() {
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    tap.modifiers_changed(SHIFT, ms(t0, 0));
    tap.modifiers_changed(NONE, ms(t0, 50));
    tap.interrupt();
    tap.modifiers_changed(SHIFT, ms(t0, 100));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 120)), None);
}

#[test]
fn a_release_with_no_press_seen_is_ignored() {
    // The window may have been given focus with the key already down.
    let t0 = Instant::now();
    let mut tap = DoubleTap::new();
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 0)), None);
    tap.modifiers_changed(SHIFT, ms(t0, 100));
    assert_eq!(tap.modifiers_changed(NONE, ms(t0, 150)), None);
}
