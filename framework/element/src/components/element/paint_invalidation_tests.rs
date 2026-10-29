//! The per-element questions "was my paint invalidated?" are asked for every
//! element on every frame, almost always with nothing recorded. Answering them
//! must stay exact whether or not anything is.

use super::*;

#[test]
fn an_event_invalidation_is_consumed_by_the_first_question_only() {
    let marked = ElementId::next();
    let other = ElementId::next();

    assert!(!take_event_paint_invalidated(marked), "nothing was recorded yet");
    mark_event_paint_invalidated(marked);

    assert!(!take_event_paint_invalidated(other));
    assert!(take_event_paint_invalidated(marked));
    assert!(!take_event_paint_invalidated(marked), "it is consumed, not sticky");
}

#[test]
fn several_event_invalidations_are_each_consumed_once() {
    let ids = (0..5).map(|_| ElementId::next()).collect::<Vec<_>>();
    for id in &ids {
        mark_event_paint_invalidated(*id);
    }
    mark_event_paint_invalidated(ids[2]);

    for id in &ids {
        assert!(take_event_paint_invalidated(*id));
        assert!(!take_event_paint_invalidated(*id));
    }
}

#[test]
fn a_local_invalidation_is_consumed_by_its_element_and_no_other() {
    let marked = ElementId::next();
    let other = ElementId::next();
    assert!(!take_local_paint_invalidated(marked));

    mark_paint_element_invalidated(marked);
    assert!(!take_local_paint_invalidated(other));
    assert!(take_local_paint_invalidated(marked));
    assert!(!take_local_paint_invalidated(marked), "it is consumed, not sticky");
}

#[test]
fn a_local_invalidation_survives_the_rebuilds_that_follow_it() {
    // A rebuild pass reconciles many elements before any of them is drawn, and
    // every rebuild that marks a descendant advances the generation. Each
    // element must still hear that its paint is stale when it is drawn.
    let ids = (0..8).map(|_| ElementId::next()).collect::<Vec<_>>();
    for id in &ids {
        mark_paint_element_invalidated(*id);
        REBUILD_INVALIDATION_GENERATION.fetch_add(1, Ordering::AcqRel);
    }
    for id in &ids {
        assert!(take_local_paint_invalidated(*id));
    }
}
