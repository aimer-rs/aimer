//! How a reverse engine presents its position: the controller, the callbacks
//! and a rebuilt engine all speak in distance from the content end.

use std::rc::Rc;

use super::controller::*;
use super::scroll_controller::ScrollController;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;

/// A vertical reverse engine laid out over 350 px of content in a 100 px
/// viewport (range 250), parked `distance` px from the end.
fn laid_out(distance: f32) -> Rc<ScrollState> {
    let mut state = ScrollState::for_test_at(Vec2d {
        x: 0.0,
        y: distance - 250.0,
    });
    state.reverse = true;
    state.cached_max_scroll.set(Vec2d { x: 0.0, y: 250.0 });
    state.cached_viewport.set((100.0, 100.0));
    state.cached_content_size.set(ResolvedSize {
        width: 100.0,
        height: 350.0,
    });
    state.cached_content_size_valid.set(true);
    Rc::new(state)
}

#[test]
fn the_controller_reports_the_distance_from_the_end() {
    let ctrl = ScrollController::new();
    ctrl.attach(laid_out(0.0));
    assert_eq!(ctrl.offset().y, 0.0, "at the end is zero");

    let ctrl = ScrollController::new();
    ctrl.attach(laid_out(100.0));
    assert_eq!(ctrl.offset().y, 100.0);
}

#[test]
fn jump_to_counts_from_the_end() {
    let state = laid_out(0.0);
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());

    ctrl.jump_to(Vec2d { x: 0.0, y: 100.0 });
    assert_eq!(state.scroll_offset.get().y, -150.0);

    ctrl.jump_to(Vec2d::ZERO);
    assert_eq!(state.scroll_offset.get().y, -250.0, "zero shows the newest");
}

#[test]
fn jump_to_clamps_to_the_start_of_the_content() {
    let state = laid_out(0.0);
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());

    ctrl.jump_to(Vec2d { x: 0.0, y: 9999.0 });

    assert_eq!(state.scroll_offset.get().y, 0.0);
    assert_eq!(ctrl.offset().y, 250.0);
}

#[test]
fn animate_to_targets_a_distance_from_the_end() {
    let state = laid_out(0.0);
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());

    ctrl.animate_to(
        Vec2d { x: 0.0, y: 40.0 },
        std::time::Duration::ZERO,
        aimer_animation::curves::LINEAR,
    );

    assert_eq!(state.scroll_offset.get().y, -210.0);
}

#[test]
fn jump_to_before_the_first_layout_is_kept_for_it() {
    // No range yet, so clamping would throw the request away.
    let mut state = ScrollState::for_test_at(Vec2d::ZERO);
    state.reverse = true;
    let state = Rc::new(state);
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());

    ctrl.jump_to(Vec2d { x: 0.0, y: 100.0 });

    assert_eq!(state.scroll_offset.get().y, 100.0, "seeded as a distance");
    assert_eq!(ctrl.offset().y, 100.0);
}

#[test]
fn a_pending_position_is_a_distance_from_the_end() {
    let ctrl = ScrollController::new();
    ctrl.jump_to(Vec2d { x: 0.0, y: 60.0 });
    let mut state = ScrollState::for_test_at(Vec2d::ZERO);
    state.reverse = true;
    let state = Rc::new(state);

    ctrl.attach(state.clone());

    assert_eq!(state.scroll_offset.get().y, 60.0);
}

#[test]
fn a_forward_controller_is_unchanged() {
    let state = ScrollState::for_test_at(Vec2d { x: 0.0, y: -80.0 });
    state.cached_max_scroll.set(Vec2d { x: 0.0, y: 250.0 });
    let state = Rc::new(state);
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());

    assert_eq!(ctrl.offset().y, 80.0);
    ctrl.jump_to(Vec2d { x: 0.0, y: 30.0 });
    assert_eq!(state.scroll_offset.get().y, -30.0);
}

#[test]
fn a_rebuilt_engine_keeps_the_distance_from_the_end() {
    let previous = laid_out(100.0);
    let mut next = ScrollState::for_test_at(Vec2d::ZERO);
    next.reverse = true;

    next.adopt_scroll_state(&previous);

    // The new engine has no range yet, so the distance is held as a seed and
    // its first layout resolves it against the real range.
    assert_eq!(next.scroll_offset.get().y, 100.0);
    assert_eq!(next.logical_offset().y, 100.0);
}

#[test]
fn switching_to_reverse_starts_at_the_end() {
    let previous = {
        let state = ScrollState::for_test_at(Vec2d { x: 0.0, y: -80.0 });
        Rc::new(state)
    };
    let mut next = ScrollState::for_test_at(Vec2d::ZERO);
    next.reverse = true;

    next.adopt_scroll_state(&previous);

    assert_eq!(next.scroll_offset.get().y, 0.0, "distance 0 is the end");
}

#[test]
fn the_stored_offset_is_the_distance_from_the_end() {
    let state = laid_out(100.0);

    assert_eq!(state.persisted_offset(state.scroll_offset.get()).y, 100.0);

    let forward = {
        let s = ScrollState::for_test_at(Vec2d { x: 0.0, y: -80.0 });
        s.cached_max_scroll.set(Vec2d { x: 0.0, y: 250.0 });
        s
    };
    assert_eq!(
        forward.persisted_offset(forward.scroll_offset.get()).y,
        -80.0,
        "a forward scrollable stores its offset exactly as before"
    );
}

#[test]
fn adopting_a_seed_into_an_engine_that_has_a_range_keeps_the_distance() {
    // A rebuild builds a fresh engine, then hands the position back to the old
    // one the surviving element still carries. The fresh engine holds the
    // distance as a seed (no range yet); the old one has a real range, so the
    // seed must be re-expressed against it rather than read as an offset.
    let old = laid_out(100.0);
    let mut fresh = ScrollState::for_test_at(Vec2d::ZERO);
    fresh.reverse = true;
    fresh.adopt_scroll_state(&old);
    assert_eq!(fresh.scroll_offset.get().y, 100.0, "held as a seed");

    old.adopt_scroll_state(&fresh);

    assert_eq!(old.scroll_offset.get().y, -150.0, "still 100 px from the end");
    assert_eq!(old.logical_offset().y, 100.0);
}
