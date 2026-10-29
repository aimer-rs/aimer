//! What a window records for its next frame stays with that window.
//!
//! Each test makes two windows by hand, the way a host with two windows would:
//! one state per window, swapped onto the thread while that window runs.

use super::*;

/// Runs `body` with `state` on the thread, then takes it back off.
fn within<R>(state: &mut WindowPaintState, body: impl FnOnce() -> R) -> R {
    state.swap_with_thread();
    let result = body();
    state.swap_with_thread();
    result
}

/// Draws one frame of the given size and reports its damage.
fn frame(width: u32, height: u32) -> DamageSet {
    begin_paint_frame(width, height);
    take_paint_frame_damage(width, height)
}

#[test]
fn an_event_invalidation_survives_the_frame_of_another_window() {
    let (mut first, mut second) = (WindowPaintState::default(), WindowPaintState::default());
    let hovered = ElementId::next();

    within(&mut first, || mark_event_paint_invalidated(hovered));
    within(&mut second, || frame(800, 600));

    assert!(
        within(&mut first, || take_event_paint_invalidated(hovered)),
        "another window's frame cleared the invalidation"
    );
}

#[test]
fn an_event_invalidation_is_not_visible_to_another_window() {
    let (mut first, mut second) = (WindowPaintState::default(), WindowPaintState::default());
    let hovered = ElementId::next();

    within(&mut first, || mark_event_paint_invalidated(hovered));

    assert!(!within(&mut second, || take_event_paint_invalidated(hovered)));
}

#[test]
fn a_full_repaint_is_spent_by_the_window_it_was_asked_of() {
    let (mut first, mut second) = (WindowPaintState::default(), WindowPaintState::default());
    // Both windows have drawn once, so neither owes a first-frame repaint.
    within(&mut first, || frame(400, 300));
    within(&mut second, || frame(800, 600));

    within(&mut first, mark_paint_damage_full);

    assert!(
        !within(&mut second, || frame(800, 600)).is_full(),
        "the other window repainted in full for a request that was not its own"
    );
    assert!(
        within(&mut first, || frame(400, 300)).is_full(),
        "the window the request was made of did not repaint in full"
    );
}

#[test]
fn each_window_remembers_the_target_it_drew() {
    let (mut first, mut second) = (WindowPaintState::default(), WindowPaintState::default());

    assert!(within(&mut first, || frame(400, 300)).is_full(), "a first frame");
    assert!(within(&mut second, || frame(800, 600)).is_full(), "a first frame");

    assert!(
        !within(&mut first, || frame(400, 300)).is_full(),
        "a window of another size made this one's target look new"
    );
    assert!(!within(&mut second, || frame(800, 600)).is_full());
}

#[test]
fn a_window_that_changed_size_repaints_in_full() {
    let mut window = WindowPaintState::default();
    within(&mut window, || frame(400, 300));

    assert!(within(&mut window, || frame(500, 300)).is_full());
}

#[test]
fn swapping_twice_puts_everything_back() {
    let mut window = WindowPaintState::default();
    let marked = ElementId::next();
    mark_event_paint_invalidated(marked);

    window.swap_with_thread();
    assert!(!take_event_paint_invalidated(marked), "the thread's state leaked in");
    window.swap_with_thread();

    assert!(take_event_paint_invalidated(marked), "the thread's state was lost");
}

#[test]
fn what_was_recorded_with_no_window_active_goes_to_the_next_one() {
    let mut window = WindowPaintState::default();
    within(&mut window, || frame(400, 300));
    let orphan = ElementId::next();
    mark_event_paint_invalidated(orphan);
    mark_paint_damage_full();

    // What a host does as a window becomes the active one.
    window.swap_with_thread();
    window.absorb_into_thread();

    assert!(take_event_paint_invalidated(orphan));
    assert!(frame(400, 300).is_full(), "an unattributed full repaint was dropped");
    window.swap_with_thread();
}
