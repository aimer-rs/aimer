use std::cell::Cell;
use std::rc::Rc;

use super::controller::*;
use super::scroll_controller::ScrollController;
use aimer_animation::Curve;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_utils::AnimInstant;
use aimer_utils::callback::Callback;
use crate::scrollable::constants::*;
use crate::scrollable::overscroll_source::{OverscrollSource, OverscrollSources};
use crate::scrollable::scroll_behavior::ScrollBehavior;
use crate::scrollable::{ScrollAxis, ScrollBarPlacement};
use std::cell::RefCell;
use std::time::Duration;
use aimer_animation::curves;

fn ctrl_with_offset(offset: Vec2d) -> ScrollState {
    ScrollState::for_test_at(offset)
}

#[test]
fn scroll_state_uses_the_frame_requester() {
    let requests = Rc::new(Cell::new(0));
    let counted = requests.clone();
    let previous = aimer_events::window::set_thread_redraw_requester(move || {
        counted.set(counted.get() + 1);
    });

    ctrl_with_offset(Vec2d::ZERO).request_animation_frame();

    aimer_events::window::restore_thread_redraw_requester(previous);

    assert_eq!(requests.get(), 1);
}

#[test]
fn scrollbar_track_hit_testing_follows_its_placement() {
    let mut ctrl = ctrl_with_offset(Vec2d::ZERO);
    ctrl.cached_viewport.set((320.0, 500.0));
    ctrl.bar.v_thumb_rect.set(Some((312.0, 100.0, 6.0, 80.0)));

    ctrl.scroll_bar_placement = ScrollBarPlacement::Floating;
    assert!(ctrl.hit_test_v_track(Vec2d { x: 309.0, y: 300.0 }, 320.0, 500.0, 12.0,));

    ctrl.scroll_bar_placement = ScrollBarPlacement::Inline;
    assert!(!ctrl.hit_test_v_track(Vec2d { x: 309.0, y: 300.0 }, 320.0, 500.0, 12.0,));
    assert!(ctrl.hit_test_v_track(Vec2d { x: 321.0, y: 300.0 }, 320.0, 500.0, 12.0,));
}

// Regression for "moving the cursor pins a core": with per-move redraws
// gone, the highlight the draw pass resolves from the cursor only updates
// on a frame somebody asked for — and the moves that cross a thumb edge
// are the ones that must ask, exactly once per crossing.
#[test]
fn thumb_hover_changes_only_on_the_move_that_crosses_the_edge() {
    let ctrl = ctrl_with_offset(Vec2d::default());
    ctrl.bar.v_thumb_rect.set(Some((390.0, 100.0, 10.0, 80.0)));

    assert!(
        !ctrl.sync_thumb_hover(Vec2d { x: 200.0, y: 120.0 }),
        "a move far from the thumb changes nothing"
    );
    assert!(
        ctrl.sync_thumb_hover(Vec2d { x: 395.0, y: 120.0 }),
        "the move that enters the thumb reports the transition"
    );
    assert!(
        !ctrl.sync_thumb_hover(Vec2d { x: 395.0, y: 150.0 }),
        "a move within the thumb reports nothing further"
    );
    assert!(
        ctrl.sync_thumb_hover(Vec2d { x: 200.0, y: 120.0 }),
        "the move that leaves the thumb reports the transition"
    );
}

// Reconciliation contract: on rebuild the freshly built controller starts at
// its initial offset (top), then adopts the previous controller's live scroll
// position so the viewport doesn't snap back to the top.
#[test]
fn adopt_scroll_state_preserves_offset() {
    let prev = ctrl_with_offset(Vec2d { x: 3.0, y: 150.0 });
    prev.pointer_velocity.set(Vec2d { x: 0.0, y: -12.0 });
    prev.fling.spring_velocity.set(Vec2d { x: 0.0, y: -200.0 });
    prev.cached_content_size.set(ResolvedSize {
        width: 320.0,
        height: 1_200.0,
    });
    prev.cached_content_size_valid.set(true);

    let fresh = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    assert_eq!(
        fresh.scroll_offset.get().y,
        0.0,
        "fresh element starts at the top"
    );

    fresh.adopt_scroll_state(&prev);

    assert_eq!(fresh.scroll_offset.get().x, 0.0);
    assert_eq!(fresh.scroll_offset.get().y, 150.0);
    assert_eq!(fresh.pointer_velocity.get().y, -12.0);
    assert_eq!(fresh.fling.spring_velocity.get().y, -200.0);
    assert_eq!(fresh.cached_content_size.get().height, 1_200.0);
    assert!(fresh.cached_content_size_valid.get());
}

#[test]
fn setting_an_offset_clears_the_inactive_axis() {
    let mut ctrl = ctrl_with_offset(Vec2d::ZERO);
    ctrl.set_scroll_offset(Vec2d { x: 24.0, y: -18.0 });
    assert_eq!(ctrl.offset(), Vec2d { x: 0.0, y: -18.0 });

    ctrl.axis = ScrollAxis::Horizontal;
    ctrl.set_scroll_offset(Vec2d { x: 24.0, y: -18.0 });
    assert_eq!(ctrl.offset(), Vec2d { x: 24.0, y: 0.0 });
}

/// A bouncy engine parked 40 px past the start edge, in a viewport large
/// enough that [`MAX_OVERSCROLL_VIEWPORT_FRACTION`] does not clamp the
/// stretch.
fn overscrolled_ctrl() -> ScrollState {
    let ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 40.0 });
    ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });
    ctrl.cached_viewport.set((400.0, 800.0));
    ctrl.cached_content_size.set(ResolvedSize {
        width: 400.0,
        height: 2000.0,
    });
    ctrl
}

#[test]
fn overscroll_recovery_waits_during_the_initial_grace_period() {
    let ctrl = overscrolled_ctrl();
    ctrl.hold_overscroll_recovery();

    let (offset, needs_redraw) = ctrl.update_momentum(ctrl.scroll_offset.get());

    assert_eq!(
        offset.y, 40.0,
        "the first overscroll frame keeps its complete stretch"
    );
    assert!(
        needs_redraw,
        "a held stretch keeps frames coming so recovery can start on its own"
    );
}

#[test]
fn direct_manipulation_holds_overscroll_past_the_grace_period() {
    let ctrl = overscrolled_ctrl();
    let held_at = AnimInstant::now();
    ctrl.hold_overscroll_recovery_at(held_at);
    ctrl.hold_overscroll_for_direct_manipulation();

    let well_past_grace =
        held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 1_000);

    assert!(ctrl.overscroll_recovery_held_at(well_past_grace));
    ctrl.release_overscroll_recovery();
    assert!(!ctrl.overscroll_recovery_held_at(well_past_grace));
}

#[test]
fn a_lift_hands_the_contact_hold_over_to_the_grace_period() {
    let ctrl = overscrolled_ctrl();
    ctrl.hold_overscroll_for_direct_manipulation();

    // The first frame after the fingers left keeps the stretch only for the
    // grace period, so recovery starts on its own from there.
    let held_at = AnimInstant::now();
    ctrl.release_direct_overscroll_hold();
    ctrl.hold_overscroll_recovery_at(held_at);

    assert!(ctrl.overscroll_recovery_held_at(held_at));
    let stale = held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 50);
    assert!(
        !ctrl.overscroll_recovery_held_at(stale),
        "a lifted gesture cannot hold the stretch forever"
    );
}

#[test]
fn overscroll_recovery_starts_once_the_device_gesture_ends() {
    let ctrl = overscrolled_ctrl();
    ctrl.hold_overscroll_recovery();
    ctrl.release_overscroll_recovery();

    let (offset, needs_redraw) = ctrl.update_momentum(ctrl.scroll_offset.get());

    assert!(
        offset.y < 40.0 && offset.y >= 0.0,
        "the spring pulls the content back toward the edge, got {}",
        offset.y
    );
    assert!(needs_redraw);
}

#[test]
fn held_overscroll_recovery_expires_after_the_grace_period() {
    let ctrl = overscrolled_ctrl();
    let held_at = AnimInstant::now();
    ctrl.hold_overscroll_recovery_at(held_at);

    assert!(ctrl.overscroll_recovery_held_at(held_at));

    let stale = held_at + Duration::from_millis(OVERSCROLL_HOLD_IDLE_MS as u64 + 50);
    assert!(
        !ctrl.overscroll_recovery_held_at(stale),
        "an unterminated gesture cannot hold the stretch forever"
    );
    assert!(
        !ctrl.overscroll.has_grace(),
        "the expired hold is cleared so later frames take the normal path"
    );
}

// The stretch a user perceives is relative to what they can see, so the cap
// belongs to the viewport. Measuring it against the content made a long
// page allow a stretch of hundreds of pixels.
#[test]
fn a_held_gesture_never_stretches_past_the_overscroll_cap() {
    let ctrl = overscrolled_ctrl();
    ctrl.hold_overscroll_recovery();

    let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 5_000.0 });

    assert_eq!(offset.y, 800.0 * MAX_OVERSCROLL_VIEWPORT_FRACTION);
}

#[test]
fn the_overscroll_cap_ignores_the_content_height() {
    let ctrl = overscrolled_ctrl();
    ctrl.cached_content_size.set(ResolvedSize {
        width: 400.0,
        height: 20_000.0,
    });
    ctrl.hold_overscroll_recovery();

    let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 5_000.0 });

    assert_eq!(
        offset.y,
        800.0 * MAX_OVERSCROLL_VIEWPORT_FRACTION,
        "a ten-times-longer page may not stretch ten times as far"
    );
}

// Out-of-range input must feel like a rubber band, not like a constant
// gear ratio: the further the content is pulled, the less each further
// step of the gesture moves it.
#[test]
fn outward_resistance_grows_with_the_stretch() {
    let ctrl = overscrolled_ctrl();
    let limit = ctrl.overscroll_limit().y;

    ctrl.scroll_offset.set(Vec2d { x: 0.0, y: 1.0 });
    let near_edge = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

    ctrl.scroll_offset.set(Vec2d {
        x: 0.0,
        y: limit * 0.5,
    });
    let half_way = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

    ctrl.scroll_offset.set(Vec2d { x: 0.0, y: limit });
    let at_limit = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 }).y;

    assert!(near_edge > half_way, "{near_edge} > {half_way}");
    assert!(half_way > at_limit, "{half_way} > {at_limit}");
    assert_eq!(at_limit, 0.0, "the limit absorbs the whole step");
    assert!(
        near_edge <= 30.0 * OOB_DRAG_RESISTANCE,
        "the base resistance stays the softest the edge ever is"
    );
}

// The reported bug: holding a trackpad gesture at an edge kept feeding
// outward deltas, and a constant resistance let them creep on until a
// content-relative cap stopped them half a screen away.
#[test]
fn a_sustained_gesture_at_the_edge_converges_on_the_overscroll_limit() {
    let ctrl = overscrolled_ctrl();
    ctrl.scroll_offset.set(Vec2d::ZERO);
    let limit = ctrl.overscroll_limit().y;

    for _ in 0..400 {
        let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 });
        let next = ctrl.apply_wheel_delta(ctrl.scroll_offset.get(), step);
        ctrl.scroll_offset.set(next);
    }

    let stretch = ctrl.overscroll_distance(ctrl.scroll_offset.get()).y;
    assert!(
        stretch <= limit,
        "a held gesture may never pull past the limit, got {stretch} > {limit}"
    );
    assert!(
        stretch > limit * 0.5,
        "but it must still reach a clearly visible stretch, got {stretch}"
    );
}

#[test]
fn pulling_back_from_a_stretch_is_never_resisted() {
    let ctrl = overscrolled_ctrl();

    let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: -25.0 });

    assert_eq!(
        step.y, -25.0,
        "input that shortens the stretch answers the finger one to one"
    );
}

#[test]
fn an_in_range_step_is_never_resisted() {
    let ctrl = overscrolled_ctrl();
    ctrl.scroll_offset.set(Vec2d { x: 0.0, y: -200.0 });

    let step = ctrl.resisted_overscroll_delta(Vec2d { x: 0.0, y: 30.0 });

    assert_eq!(step.y, 30.0);
}

#[test]
fn a_non_bouncy_scrollable_ignores_the_recovery_hold() {
    let mut ctrl = overscrolled_ctrl();
    ctrl.scroll_behavior = ScrollBehavior::no_bounce();
    ctrl.hold_overscroll_recovery();

    let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 40.0 });

    assert_eq!(offset.y, 0.0, "a non-bouncy viewport clamps to the edge");
}

#[test]
fn overscroll_recovery_idle_timeout_is_short_enough_to_feel_immediate() {
    let timeout_ms = std::hint::black_box(OVERSCROLL_HOLD_IDLE_MS);
    assert!(
        timeout_ms <= 4,
        "edge recovery should be eligible by the next rendered frame"
    );
}

// A delta that stretches the content beyond the episode's existing peak is
// renewed user intent and must still move the content.
#[test]
fn a_stronger_device_delta_extends_the_overscroll_peak() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 15.0 });

    assert!(applied, "a stronger stretch is never ignored");
    assert_eq!(ctrl.scroll_offset.get().y, 55.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 55.0);
}

// Once recovery starts, weaker outward tail events must not pump the
// content back away from the edge.
#[test]
fn a_weaker_re_stretch_is_ignored_while_the_recovery_runs() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.scroll_offset.set(Vec2d { x: 0.0, y: 12.0 });

    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

    assert!(!applied, "20 px of stretch cannot beat a 40 px peak");
    assert_eq!(ctrl.scroll_offset.get().y, 12.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 40.0);
}

#[test]
fn a_delta_pulling_back_toward_the_edge_is_never_ignored() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -25.0 });

    assert!(applied);
    assert_eq!(ctrl.scroll_offset.get().y, 15.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 40.0);
}

#[test]
fn returning_in_range_keeps_the_peak_for_the_remaining_device_tail() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());

    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -80.0 });

    assert!(applied);
    assert_eq!(ctrl.scroll_offset.get().y, -40.0);
    assert_eq!(
        ctrl.overscroll.peak_stretch().y,
        40.0,
        "the old gesture's remaining events must stay gated after recovery"
    );
}

#[test]
fn a_late_device_tail_cannot_bounce_after_recovery_finishes() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.release_overscroll_recovery();

    assert_eq!(
        ctrl.overscroll.peak_stretch().y,
        40.0,
        "the peak must survive recovery start to guard the remaining tail"
    );

    let (offset, _) = ctrl.update_momentum(Vec2d { x: 0.0, y: 0.0 });
    ctrl.scroll_offset.set(offset);
    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

    assert_eq!(offset.y, 0.0);
    assert!(!applied, "the recovered gesture's outward tail is ignored");
    assert_eq!(ctrl.scroll_offset.get().y, 0.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 40.0);
}

#[test]
fn a_new_device_gesture_can_overscroll_the_same_edge() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.scroll_offset.set(Vec2d::ZERO);

    ctrl.reset_overscroll_peak();
    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 });

    assert!(applied);
    assert_eq!(ctrl.scroll_offset.get().y, 8.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 8.0);
}

// The peak only guards against a *live* device tail. Once the device stops
// sending events there is no tail left to guard, so a peak that nothing
// refreshed must stop gating — otherwise a missed gesture boundary wedges
// the edge and no later gesture can overscroll at all.
#[test]
fn an_idle_overscroll_peak_stops_gating_the_next_gesture() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.scroll_offset.set(Vec2d::ZERO);

    let idle = AnimInstant::now() + Duration::from_millis(OVERSCROLL_PEAK_IDLE_MS as u64 + 10);
    let applied = ctrl.apply_precise_scroll_delta_at(Vec2d { x: 0.0, y: 8.0 }, idle);

    assert!(applied, "a stale peak cannot block the edge forever");
    assert_eq!(ctrl.scroll_offset.get().y, 8.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 8.0);
}

// A native momentum tail delivers an event every frame. Each of those
// events — including the ones the peak rejects — proves the tail is still
// running, so it must keep the peak alive.
#[test]
fn a_rejected_tail_event_keeps_the_peak_alive() {
    let ctrl = overscrolled_ctrl();
    let started = AnimInstant::now();
    ctrl.record_overscroll_peak_at(ctrl.scroll_offset.get(), started);
    ctrl.scroll_offset.set(Vec2d::ZERO);

    let mut at = started;
    for _ in 0..12 {
        at += Duration::from_millis(OVERSCROLL_PEAK_IDLE_MS as u64 / 2);
        assert!(
            !ctrl.apply_precise_scroll_delta_at(Vec2d { x: 0.0, y: 8.0 }, at),
            "a weaker tail event stays gated while the tail keeps arriving"
        );
    }

    assert_eq!(ctrl.scroll_offset.get().y, 0.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, 40.0);
}

#[test]
fn fresh_contact_clears_the_previous_gesture_peak() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.scroll_offset.set(Vec2d::ZERO);

    assert!(
        ctrl.begin_device_contact(true),
        "fingers landing on the device is a new gesture"
    );
    assert_eq!(ctrl.overscroll.peak_stretch(), Vec2d::ZERO);
    assert!(
        !ctrl.begin_device_contact(true),
        "later frames of the same contact are not a new gesture"
    );

    assert!(ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: 8.0 }));
    assert_eq!(ctrl.scroll_offset.get().y, 8.0);
}

#[test]
fn losing_contact_arms_the_next_gesture_detection() {
    let ctrl = overscrolled_ctrl();
    assert!(ctrl.begin_device_contact(true));
    assert!(!ctrl.begin_device_contact(false));
    assert!(
        ctrl.begin_device_contact(true),
        "a lift followed by a touch is a new gesture again"
    );
}

#[test]
fn the_opposite_edge_starts_a_new_peak_during_the_same_gesture() {
    let ctrl = overscrolled_ctrl();
    ctrl.record_overscroll_peak(ctrl.scroll_offset.get());
    ctrl.scroll_offset.set(Vec2d { x: 0.0, y: -1000.0 });

    let applied = ctrl.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -8.0 });

    assert!(applied);
    assert_eq!(ctrl.scroll_offset.get().y, -1008.0);
    assert_eq!(ctrl.overscroll.peak_stretch().y, -8.0);
}

#[test]
fn controller_reattach_preserves_in_flight_scroll_state() {
    let ctrl = ScrollController::new();
    let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: -120.0 }));
    let fling_started = AnimInstant::now();
    first.pointer_velocity.set(Vec2d { x: 0.0, y: -12.0 });
    first.fling.start_time.set(Some(fling_started));
    first.fling.start_offset.set(Vec2d { x: 0.0, y: -120.0 });
    first.fling.target_offset.set(Vec2d { x: 0.0, y: -360.0 });
    first.fling.duration.set(1.25);
    first.session.active.set(true);
    ctrl.attach(first);

    let rebuilt = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(rebuilt.clone());

    assert_eq!(rebuilt.scroll_offset.get().y, -120.0);
    assert_eq!(rebuilt.pointer_velocity.get().y, -12.0);
    assert!(rebuilt.fling.start_time.get().is_some());
    assert_eq!(rebuilt.fling.start_offset.get().y, -120.0);
    assert_eq!(rebuilt.fling.target_offset.get().y, -360.0);
    assert_eq!(rebuilt.fling.duration.get(), 1.25);
    assert!(rebuilt.session.active.get());

    let (_, needs_redraw) = rebuilt.update_momentum(rebuilt.scroll_offset.get());
    assert!(
        needs_redraw,
        "the rebuilt engine continues the remaining fling"
    );
}

#[test]
fn controller_reattach_preserves_active_drag() {
    let ctrl = ScrollController::new();
    let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: -120.0 }));
    first.drag_mode.set(DragMode::Content);
    first.active_touch_id.set(Some(7));
    first.last_pointer_pos.set(Some(Vec2d { x: 20.0, y: 80.0 }));
    ctrl.attach(first);

    let rebuilt = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(rebuilt.clone());

    assert_eq!(rebuilt.drag_mode.get(), DragMode::Content);
    assert_eq!(rebuilt.active_touch_id.get(), Some(7));
    let last_pointer_pos = rebuilt
        .last_pointer_pos
        .get()
        .expect("active drag keeps its last pointer position");
    assert_eq!(last_pointer_pos.x, 20.0);
    assert_eq!(last_pointer_pos.y, 80.0);
}

// Coalesced-events contract (the web "scroll too fast" bug): winit delivers
// one native `pointermove` on web as a BURST of coalesced samples that all
// read the same `Instant`. A naive per-sample `delta / dt` divides a tiny
// sub-delta by a ~0 dt, so the release-fling velocity explodes (~Nx the real
// finger speed for N coalesced samples). `accumulate_drag_velocity` folds a
// whole frame's coalesced sub-samples into ONE realistic value, so a drag fed
// as many fine same-instant sub-moves yields the same steady velocity as the
// identical travel fed as one coarse move per frame — NOT an inflated one.
#[test]
fn coalesced_moves_match_coarse_drag_velocity() {
    use std::time::Duration;

    let frame = Duration::from_millis(16); // ~60 Hz
    let t0 = AnimInstant::now();
    let travel_per_frame = 16.0_f32; // px moved each frame
    let frames = 6;
    let sub = 8; // coalesced sub-samples per frame on web

    // Native path: one move per frame.
    let native = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    let mut last_native = Vec2d { x: 0.0, y: 0.0 };
    let mut t = t0;
    for _ in 0..frames {
        if let Some((v, _)) = native.accumulate_drag_velocity(0.0, travel_per_frame, t) {
            last_native = v;
        }
        t += frame;
    }

    // Web path: the SAME travel each frame, split into `sub` coalesced
    // sub-moves that all share the frame's instant.
    let web = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    let mut last_web = Vec2d { x: 0.0, y: 0.0 };
    let mut t = t0;
    let step = travel_per_frame / sub as f32;
    for _ in 0..frames {
        for _ in 0..sub {
            if let Some((v, _)) = web.accumulate_drag_velocity(0.0, step, t) {
                last_web = v;
            }
        }
        t += frame;
    }

    // Steady-state velocities must match (both = travel / frame time); the
    // coalesced burst is not inflated by the number of sub-samples.
    assert!(
        (last_web.y - last_native.y).abs() < 0.5,
        "coalesced web velocity {} must match coarse native velocity {} (not inflated)",
        last_web.y,
        last_native.y
    );
    // Guard the direction of the bug: the fix must not let the web value run
    // away above the real per-frame speed.
    assert!(last_web.y <= last_native.y * 1.2 + 0.5);
}

// Small reverse-flick contract: the release fling reads `smoothed_velocity()`,
// a weighted average of the velocity ring buffer. After a fast fling leaves the
// buffer full of old-direction samples, a SMALL reverse flick pushes only 1–2
// opposite samples — not enough to flip that average, so without clearing the
// buffer the release still coasts the OLD way (the reported bug). Clearing the
// history on a detected reversal (as handle_scroll now does) makes the release
// reflect only the new direction.
#[test]
fn small_reverse_flick_clears_stale_velocity_history() {
    let c = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    let slice = Duration::from_micros(4_000);
    let slice_s = 0.004;
    let mut at = AnimInstant::now();

    // Prior fling: the horizon is full of positive (old-direction) samples.
    for _ in 0..6 {
        at += slice;
        c.push_velocity(Vec2d { x: 0.0, y: 100.0 }, slice_s, at);
    }

    // A small reverse flick adds a single opposite sample.
    at += slice;
    c.push_velocity(Vec2d { x: 0.0, y: -20.0 }, slice_s, at);
    // Without clearing, the average is still the OLD direction.
    assert!(
        c.smoothed_velocity(at).y > 0.0,
        "stale samples keep the average pointing the old way"
    );

    // Reversal handling clears the buffer before the new sample is recorded.
    c.clear_velocity_history();
    c.push_velocity(Vec2d { x: 0.0, y: -20.0 }, slice_s, at);
    assert!(
        c.smoothed_velocity(at).y < 0.0,
        "after clearing, the release follows the new direction"
    );
}

/// An engine tuned like a browser build: bouncy edges declared, but the
/// wheel left out of the trusted devices.
fn web_ctrl() -> ScrollState {
    let mut ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    ctrl.overscroll_sources = OverscrollSources::WEB_DEFAULT;
    ctrl.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });
    ctrl
}

#[test]
fn a_wheel_gesture_does_not_bounce_where_the_bitmap_leaves_the_wheel_out() {
    let ctrl = web_ctrl();
    ctrl.set_overscroll_source(OverscrollSource::Wheel);

    assert!(
        !ctrl.bouncy(),
        "a browser wheel stream is not trusted with a rubber band"
    );
    let next = ctrl.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 40.0 });
    assert_eq!(next.y, 0.0, "the edge clamps instead of stretching");
}

#[test]
fn a_touch_gesture_still_bounces_where_only_the_wheel_is_left_out() {
    let ctrl = web_ctrl();
    ctrl.set_overscroll_source(OverscrollSource::Touch);

    assert!(
        ctrl.bouncy(),
        "a touch screen reports its lift, so it keeps the rubber band"
    );
    let next = ctrl.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 40.0 });
    assert_eq!(next.y, 40.0, "the finger stretches the edge as usual");
}

#[test]
fn a_rigid_viewport_stays_rigid_for_every_trusted_device() {
    let mut ctrl = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    ctrl.scroll_behavior.bouncy = false;
    ctrl.overscroll_sources = OverscrollSources::ALL;

    for source in [OverscrollSource::Wheel, OverscrollSource::Touch] {
        ctrl.set_overscroll_source(source);
        assert!(!ctrl.bouncy(), "the bitmap never grants a bounce");
    }
}

#[test]
fn the_active_device_survives_reconciliation() {
    let prev = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    prev.set_overscroll_source(OverscrollSource::Touch);
    let fresh = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });

    fresh.adopt_scroll_state(&prev);

    assert_eq!(
        fresh.overscroll.source(),
        OverscrollSource::Touch,
        "a rebuild mid-gesture must not re-judge the edge as a wheel"
    );
}

// Regression: a non-bouncy scrollable must actually move when a wheel /
// trackpad delta arrives while the offset is anywhere strictly inside the
// valid range. The old handler compared the current offset against
// `clamp_offset(offset)` and zeroed the delta when they were equal — which
// they always are for an in-range offset — so wheel/trackpad scrolling was
// completely dead. `apply_wheel_delta` must instead apply the delta first.
#[test]
fn wheel_delta_moves_non_bouncy_from_midrange() {
    let mut c = ctrl_with_offset(Vec2d { x: 0.0, y: -100.0 });
    c.scroll_behavior.bouncy = false;
    // Valid vertical range is [-1000, 0]; -100 is strictly inside it.
    c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

    // Scroll further down (offset grows more negative).
    let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: -100.0 }, Vec2d { x: 0.0, y: -20.0 });
    assert_eq!(
        next.y, -120.0,
        "wheel delta must move an in-range non-bouncy offset"
    );

    // Scroll back up.
    let up = c.apply_wheel_delta(Vec2d { x: 0.0, y: -100.0 }, Vec2d { x: 0.0, y: 15.0 });
    assert_eq!(up.y, -85.0);
}

// A non-bouncy scrollable never overscrolls: a delta past the edge clamps to
// the boundary instead of running away.
#[test]
fn wheel_delta_clamps_non_bouncy_at_boundary() {
    let mut c = ctrl_with_offset(Vec2d { x: 0.0, y: -990.0 });
    c.scroll_behavior.bouncy = false;
    c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

    // Overshoot the bottom edge (-1000): must clamp, not exceed.
    let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: -990.0 }, Vec2d { x: 0.0, y: -50.0 });
    assert_eq!(
        next.y, -1000.0,
        "non-bouncy offset clamps at the bottom edge"
    );

    // Overshoot the top edge (0): must clamp to 0.
    let top = c.apply_wheel_delta(Vec2d { x: 0.0, y: -10.0 }, Vec2d { x: 0.0, y: 40.0 });
    assert_eq!(top.y, 0.0, "non-bouncy offset clamps at the top edge");
}

// A bouncy scrollable keeps the raw (out-of-range) offset so the visual
// rubber-band + spring-back can act; `apply_wheel_delta` must not clamp it.
#[test]
fn wheel_delta_allows_overscroll_when_bouncy() {
    let c = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    assert!(c.scroll_behavior.bouncy, "default behavior is bouncy");
    c.cached_max_scroll.set(Vec2d { x: 0.0, y: 1000.0 });

    // Overscroll past the top edge (0) is preserved, not clamped.
    let next = c.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: 30.0 });
    assert_eq!(
        next.y, 30.0,
        "bouncy offset keeps the overscroll for the rubber-band"
    );

    // A normal in-range scroll still moves.
    let down = c.apply_wheel_delta(Vec2d { x: 0.0, y: 0.0 }, Vec2d { x: 0.0, y: -30.0 });
    assert_eq!(down.y, -30.0);
}

// -- Public ScrollController handle -------------------------------------

/// Build an attached controller over a fresh engine with a vertical range
/// of `[0, max_y]` (logical == internal, since `last_scale` is 1.0).
fn attached(max_y: f32) -> (ScrollController, Rc<ScrollState>) {
    let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    state.cached_max_scroll.set(Vec2d { x: 0.0, y: max_y });
    let ctrl = ScrollController::new();
    ctrl.attach(state.clone());
    (ctrl, state)
}

#[test]
fn controller_is_attached_toggles() {
    let ctrl = ScrollController::new();
    assert!(!ctrl.is_attached(), "a fresh controller is detached");
    let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(state);
    assert!(ctrl.is_attached(), "after attach the controller is live");
}

// `jump_to` takes a logical, positive-toward-content-end position and maps it
// to the internal negated/scaled offset; `offset()` reads it back logical.
#[test]
fn controller_jump_to_converts_and_reads_back() {
    let (ctrl, state) = attached(1000.0);

    ctrl.jump_to(Vec2d { x: 0.0, y: 120.0 });
    assert_eq!(
        state.scroll_offset.get().y,
        -120.0,
        "internal offset is negated"
    );
    assert_eq!(
        ctrl.offset().y,
        120.0,
        "public offset reads back the logical position"
    );
}

// `jump_to` clamps to the valid range (never past the bottom edge).
#[test]
fn controller_jump_to_clamps_to_range() {
    let (ctrl, _state) = attached(1000.0);
    ctrl.jump_to(Vec2d { x: 0.0, y: 5000.0 });
    assert_eq!(
        ctrl.offset().y,
        1000.0,
        "over-scroll target clamps to max extent"
    );
}

// A position requested before the controller is attached is remembered and
// applied on attach (Flutter's `initialScrollOffset` behaviour).
#[test]
fn controller_jump_to_before_attach_is_pending_then_applied() {
    let ctrl = ScrollController::new();
    ctrl.jump_to(Vec2d { x: 0.0, y: 75.0 });
    // Detached: reported via the pending value.
    assert_eq!(ctrl.offset().y, 75.0);

    let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(state.clone());
    assert_eq!(
        state.scroll_offset.get().y,
        -75.0,
        "pending position applied on attach"
    );
}

// `max_extent` reports the scrollable range in logical pixels.
#[test]
fn controller_max_extent_reports_logical() {
    let (ctrl, _state) = attached(640.0);
    assert_eq!(ctrl.max_extent().y, 640.0);
}

// `animate_to` with a real duration arms the curve-driven fling rather than
// snapping: the fling target/curve are set and the position hasn't moved yet.
#[test]
fn controller_animate_to_arms_curve_driven_fling() {
    let (ctrl, state) = attached(1000.0);

    ctrl.animate_to(
        Vec2d { x: 0.0, y: 200.0 },
        Duration::from_millis(300),
        curves::LINEAR,
    );

    assert!(
        state.fling.start_time.get().is_some(),
        "a timed animation arms the fling"
    );
    assert_eq!(
        state.fling.curve.borrow().clone(),
        Some(curves::LINEAR.into_any()),
        "the requested curve drives the fling"
    );
    assert_eq!(
        state.fling.target_offset.get().y,
        -200.0,
        "target stored in internal convention"
    );
    assert_eq!(
        state.scroll_offset.get().y,
        0.0,
        "position has not jumped — it will ease over time"
    );
}

// A zero-duration `animate_to` degenerates to an instant jump.
#[test]
fn controller_animate_to_zero_duration_jumps_immediately() {
    let (ctrl, state) = attached(1000.0);
    ctrl.animate_to(Vec2d { x: 0.0, y: 300.0 }, Duration::ZERO, curves::EASE_IN_OUT);
    assert_eq!(ctrl.offset().y, 300.0);
    assert!(
        state.fling.start_time.get().is_none(),
        "no fling is left running"
    );
}

// Once the animation's duration has elapsed, the draw-phase `update_momentum`
// lands exactly on the target and clears the fling.
#[test]
fn controller_animate_to_reaches_target_after_duration() {
    let (ctrl, state) = attached(1000.0);
    ctrl.animate_to(
        Vec2d { x: 0.0, y: 250.0 },
        Duration::from_millis(300),
        curves::LINEAR,
    );

    // Pretend the whole duration has already passed.
    state
        .fling.start_time
        .set(Some(AnimInstant::now() - Duration::from_millis(400)));
    let (offset, _redraw) = state.update_momentum(state.scroll_offset.get());

    assert!(
        (offset.y - (-250.0)).abs() < 1.0,
        "animation settles on the target (got {})",
        offset.y
    );
    assert!(
        state.fling.start_time.get().is_none(),
        "the fling is cleared once complete"
    );
    assert!(
        state.fling.curve.borrow().is_none(),
        "the animation curve is cleared once complete"
    );
}

#[test]
fn line_wheel_delta_applies_immediately_without_momentum() {
    let (_ctrl, state) = attached(1000.0);
    state.start_animation(Vec2d { x: 0.0, y: -100.0 }, 1.0, curves::EASE_OUT);
    state.pointer_velocity.set(Vec2d { x: 0.0, y: -8.0 });

    state.apply_line_wheel_delta(Vec2d { x: 0.0, y: -20.0 });

    assert_eq!(state.scroll_offset.get().x, 0.0);
    assert_eq!(state.scroll_offset.get().y, -20.0);
    assert_eq!(state.pointer_velocity.get().x, 0.0);
    assert_eq!(state.pointer_velocity.get().y, 0.0);
    assert!(state.fling.start_time.get().is_none());
}

#[test]
fn precise_device_delta_does_not_add_widget_side_momentum() {
    let (_ctrl, state) = attached(1000.0);
    state.pointer_velocity.set(Vec2d { x: 0.0, y: -8.0 });

    state.apply_precise_scroll_delta(Vec2d { x: 0.0, y: -4.0 });

    assert_eq!(state.scroll_offset.get().y, -4.0);
    assert_eq!(state.pointer_velocity.get().x, 0.0);
    assert_eq!(state.pointer_velocity.get().y, 0.0);
    assert!(state.fling.momentum_start.get().is_none());
}

// -- Scroll-lifecycle callbacks (on_scroll_start / on_scroll_end) --------

/// A recording callback: pushes each offset it receives into a shared log
/// so tests can assert how many times (and with what offset) it fired.
fn recorder() -> (Callback<Vec2d>, Rc<RefCell<Vec<Vec2d>>>) {
    let log: Rc<RefCell<Vec<Vec2d>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    let cb = Callback::from(move |o: Vec2d| sink.borrow_mut().push(o));
    (cb, log)
}

// `begin_scroll` / `end_scroll` are edge-triggered: each fires its callback
// exactly once per idle↔scrolling transition, never per redundant call.
#[test]
fn scroll_callbacks_are_edge_triggered() {
    let state = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    let (start_cb, starts) = recorder();
    let (end_cb, ends) = recorder();
    *state.session.on_start.borrow_mut() = start_cb;
    *state.session.on_end.borrow_mut() = end_cb;

    state.begin_scroll();
    state.begin_scroll(); // already scrolling → ignored
    assert_eq!(
        starts.borrow().len(),
        1,
        "start fires once on the idle→scrolling edge"
    );
    assert_eq!(ends.borrow().len(), 0, "end has not fired yet");

    state.end_scroll();
    state.end_scroll(); // already idle → ignored
    assert_eq!(
        ends.borrow().len(),
        1,
        "end fires once on the scrolling→idle edge"
    );

    // A brand-new session fires start again.
    state.begin_scroll();
    assert_eq!(starts.borrow().len(), 2, "a fresh session re-fires start");
}

// The start callback receives the logical offset at the moment scrolling
// begins.
#[test]
fn scroll_start_reports_logical_offset() {
    let state = ctrl_with_offset(Vec2d { x: 0.0, y: -150.0 });
    let (start_cb, starts) = recorder();
    *state.session.on_start.borrow_mut() = start_cb;

    state.begin_scroll();
    assert_eq!(
        starts.borrow()[0].y,
        150.0,
        "offset is reported logical (positive toward content end)"
    );
}

// A programmatic `jump_to` is a self-contained session: it fires exactly one
// start and one end, in that order, around the instant position change.
#[test]
fn jump_to_fires_one_start_then_one_end() {
    let (ctrl, _state) = attached(1000.0);
    let (start_cb, starts) = recorder();
    let (end_cb, ends) = recorder();
    ctrl.on_scroll_start(start_cb);
    ctrl.on_scroll_end(end_cb);

    ctrl.jump_to(Vec2d { x: 0.0, y: 120.0 });

    assert_eq!(starts.borrow().len(), 1, "jump fires start once");
    assert_eq!(ends.borrow().len(), 1, "jump fires end once");
    assert_eq!(ends.borrow()[0].y, 120.0, "end reports the landing offset");
}

// `animate_to` announces the session start immediately but leaves it open —
// the draw loop fires `end` only once the animation settles (see
// `draw_scroll`), so no `end` is emitted at arming time.
#[test]
fn animate_to_fires_start_but_not_end_until_settled() {
    let (ctrl, state) = attached(1000.0);
    let (start_cb, starts) = recorder();
    let (end_cb, ends) = recorder();
    ctrl.on_scroll_start(start_cb);
    ctrl.on_scroll_end(end_cb);

    ctrl.animate_to(
        Vec2d { x: 0.0, y: 200.0 },
        Duration::from_millis(300),
        curves::LINEAR,
    );
    assert_eq!(starts.borrow().len(), 1, "animation fires start when armed");
    assert_eq!(
        ends.borrow().len(),
        0,
        "end waits for the animation to settle"
    );

    // Simulate the draw loop reporting the motion as fully settled.
    state.end_scroll();
    assert_eq!(ends.borrow().len(), 1, "end fires once the session settles");
}

// Callbacks are held on the controller, so they keep firing after a rebuild
// re-attaches the controller to a freshly built engine.
#[test]
fn callbacks_survive_reattach_across_rebuild() {
    let ctrl = ScrollController::new();
    let (start_cb, starts) = recorder();
    ctrl.on_scroll_start(start_cb);

    // First build.
    let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(first);

    // Rebuild: a brand-new engine adopts the same controller.
    let second = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(second.clone());

    second.begin_scroll();
    assert_eq!(
        starts.borrow().len(),
        1,
        "the callback still fires on the re-attached engine"
    );
}

// Registering a callback before the controller is attached still works: it is
// stored on the controller and shared into the engine on attach.
#[test]
fn callback_registered_before_attach_is_applied_on_attach() {
    let ctrl = ScrollController::new();
    let (start_cb, starts) = recorder();
    ctrl.on_scroll_start(start_cb);

    let state = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(state.clone());

    state.begin_scroll();
    assert_eq!(
        starts.borrow().len(),
        1,
        "a pre-attach registration is applied on attach"
    );
}

// -- Per-frame scroll update callback (on_scroll) -----------------------

// `notify_scroll` is level-triggered: the first call only establishes the
// baseline (no fire, so the initial render is silent); afterwards it fires on
// every call where the logical offset actually moved.
#[test]
fn on_scroll_is_level_triggered_after_baseline() {
    let state = ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 });
    let (cb, log) = recorder();
    *state.session.on_scroll.borrow_mut() = cb;

    // First frame: only establishes the baseline, no callback.
    state.notify_scroll();
    assert_eq!(
        log.borrow().len(),
        0,
        "the initial frame establishes the baseline silently"
    );

    // Offset moves → fires with the new logical offset.
    state.scroll_offset.set(Vec2d { x: 0.0, y: -40.0 });
    state.notify_scroll();
    assert_eq!(log.borrow().len(), 1, "a genuine move fires the callback");
    assert_eq!(
        log.borrow()[0].y,
        40.0,
        "reports the logical offset (positive toward content end)"
    );

    // Another distinct move fires again (fires per frame, not per session).
    state.scroll_offset.set(Vec2d { x: 0.0, y: -90.0 });
    state.notify_scroll();
    assert_eq!(
        log.borrow().len(),
        2,
        "each frame with movement fires again"
    );
    assert_eq!(log.borrow()[1].y, 90.0);
}

// An unchanged (or sub-epsilon) offset does not re-fire — idle frames and
// sub-pixel jitter are collapsed to no-ops.
#[test]
fn on_scroll_does_not_fire_without_movement() {
    let state = ctrl_with_offset(Vec2d { x: 0.0, y: -40.0 });
    let (cb, log) = recorder();
    *state.session.on_scroll.borrow_mut() = cb;

    state.notify_scroll(); // baseline at y = 40 (logical)
    state.notify_scroll(); // identical offset → no fire
    assert_eq!(log.borrow().len(), 0, "an unchanged offset does not fire");

    // Sub-epsilon jitter is ignored too.
    state.scroll_offset.set(Vec2d {
        x: 0.0,
        y: -40.0 - SCROLL_NOTIFY_EPSILON / 2.0,
    });
    state.notify_scroll();
    assert_eq!(log.borrow().len(), 0, "sub-epsilon jitter is suppressed");
}

// A programmatic `jump_to` moves the offset, so the very next drawn frame
// reports the new position through `on_scroll`.
#[test]
fn on_scroll_fires_after_jump_on_next_frame() {
    let (ctrl, state) = attached(1000.0);
    let (cb, log) = recorder();
    ctrl.on_scroll(cb);

    // Establish the baseline (as the first render frame would).
    state.notify_scroll();
    assert_eq!(log.borrow().len(), 0);

    ctrl.jump_to(Vec2d { x: 0.0, y: 150.0 });
    // The draw loop calls `notify_scroll` once per frame; simulate that.
    state.notify_scroll();
    assert_eq!(
        log.borrow().len(),
        1,
        "the jump is reported on the next frame"
    );
    assert_eq!(log.borrow()[0].y, 150.0);
}

// The per-frame callback is held on the controller, so it keeps firing after
// a rebuild re-attaches the controller to a freshly built engine.
#[test]
fn on_scroll_survives_reattach_across_rebuild() {
    let ctrl = ScrollController::new();
    let (cb, log) = recorder();
    ctrl.on_scroll(cb);

    let first = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(first);

    // Rebuild: a brand-new engine adopts the same controller.
    let second = Rc::new(ctrl_with_offset(Vec2d { x: 0.0, y: 0.0 }));
    ctrl.attach(second.clone());

    second.notify_scroll(); // baseline on the fresh engine
    second.scroll_offset.set(Vec2d { x: 0.0, y: -60.0 });
    second.notify_scroll();
    assert_eq!(
        log.borrow().len(),
        1,
        "the callback still fires on the re-attached engine"
    );
    assert_eq!(log.borrow()[0].y, 60.0);
}
