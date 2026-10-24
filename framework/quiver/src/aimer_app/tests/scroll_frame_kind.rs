//! Frames the scroll engine asks for itself are scroll-driven without being
//! skippable: the physics they exist to advance runs inside the draw.

use super::*;
use crate::aimer_app::{
    FrameRequestKind, complete_frame_ready_request, promote_pending_scroll_frame_request,
    try_begin_frame_ready_request,
};

const ALL: [FrameRequestKind; 3] = [
    FrameRequestKind::ScrollOnly,
    FrameRequestKind::ScrollPhysics,
    FrameRequestKind::Full,
];

#[test]
fn every_kind_survives_the_trip_through_the_pending_slot() {
    for kind in ALL {
        let pending = AtomicU8::new(0);
        assert!(try_begin_frame_ready_request(&pending, kind));
        assert_eq!(complete_frame_ready_request(&pending), kind);
        assert_eq!(pending.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn merging_keeps_the_strongest_reason() {
    use FrameRequestKind::*;
    assert_eq!(ScrollOnly.merge(ScrollOnly), ScrollOnly);
    assert_eq!(ScrollOnly.merge(ScrollPhysics), ScrollPhysics);
    assert_eq!(ScrollPhysics.merge(ScrollOnly), ScrollPhysics);
    assert_eq!(ScrollPhysics.merge(ScrollPhysics), ScrollPhysics);
    for kind in ALL {
        assert_eq!(kind.merge(Full), Full);
        assert_eq!(Full.merge(kind), Full);
    }
}

#[test]
fn requests_queued_while_one_is_pending_merge_into_it() {
    let pending = AtomicU8::new(0);
    assert!(try_begin_frame_ready_request(
        &pending,
        FrameRequestKind::ScrollOnly
    ));
    assert!(!try_begin_frame_ready_request(
        &pending,
        FrameRequestKind::ScrollPhysics
    ));
    assert_eq!(
        complete_frame_ready_request(&pending),
        FrameRequestKind::ScrollPhysics
    );

    assert!(try_begin_frame_ready_request(
        &pending,
        FrameRequestKind::ScrollPhysics
    ));
    assert!(!try_begin_frame_ready_request(
        &pending,
        FrameRequestKind::Full
    ));
    assert_eq!(
        complete_frame_ready_request(&pending),
        FrameRequestKind::Full
    );
}

#[test]
fn promoting_the_pending_request_makes_a_physics_request_full_too() {
    let pending_before = crate::aimer_app::FRAME_READY_PENDING.swap(0, Ordering::SeqCst);
    for kind in [
        FrameRequestKind::ScrollOnly,
        FrameRequestKind::ScrollPhysics,
    ] {
        crate::aimer_app::FRAME_READY_PENDING.store(kind.encode(), Ordering::SeqCst);
        promote_pending_scroll_frame_request();
        assert_eq!(
            crate::aimer_app::FRAME_READY_PENDING.load(Ordering::SeqCst),
            FrameRequestKind::Full.encode(),
            "{kind:?} stayed unpromoted"
        );
    }
    crate::aimer_app::FRAME_READY_PENDING.store(pending_before, Ordering::SeqCst);
}

#[test]
fn a_quiet_scroll_only_frame_is_skipped_but_a_physics_frame_never_is() {
    let mut app = AimerApp::start_headless(SizedBox::new());
    for _ in 0..4 {
        app.render_frame();
    }
    let preparation = app.app.begin_frame();

    assert!(
        app.app
            .should_skip_scroll_frame(FrameRequestKind::ScrollOnly, &preparation, false)
    );
    assert!(!app.app.should_skip_scroll_frame(
        FrameRequestKind::ScrollPhysics,
        &preparation,
        false
    ));
    assert!(
        !app.app
            .should_skip_scroll_frame(FrameRequestKind::Full, &preparation, false)
    );
}

#[test]
fn the_scroll_engines_request_is_recorded_as_a_physics_request() {
    let reason = Rc::new(Cell::new(None));
    let previous = aimer_events::window::set_thread_redraw_requester({
        let reason = reason.clone();
        move || crate::aimer_app::observe_frame_request(&reason)
    });

    aimer_events::window::request_scroll_frame();
    assert_eq!(reason.take(), Some(FrameRequestKind::ScrollPhysics));

    aimer_events::window::request_animation_frame();
    assert_eq!(reason.take(), Some(FrameRequestKind::Full));

    crate::aimer_app::request_scroll_frame(aimer_events::window::request_animation_frame);
    assert_eq!(reason.take(), Some(FrameRequestKind::ScrollOnly));

    aimer_events::window::restore_thread_redraw_requester(previous);
}
