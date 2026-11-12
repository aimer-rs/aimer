use super::*;

struct Source(Cell<u64>);

impl PaintRevisionSource for Source {
    fn revision(&self) -> u64 {
        self.0.get()
    }
}

fn watch(element: ElementId, source: &Rc<Source>, recorded: u64) -> Rc<Cell<Option<u64>>> {
    let shared: Rc<dyn PaintRevisionSource> = source.clone();
    let recorded = Rc::new(Cell::new(Some(recorded)));
    watch_paint_revision(element, Rc::downgrade(&shared), recorded.clone());
    recorded
}

#[test]
fn a_changed_revision_invalidates_exactly_its_element() {
    let (changed, unchanged) = (ElementId::next(), ElementId::next());
    let (moving, still) = (Rc::new(Source(Cell::new(1))), Rc::new(Source(Cell::new(1))));
    watch(changed, &moving, 1);
    watch(unchanged, &still, 1);

    moving.0.set(2);
    note_paint_revision_changed();
    sync_paint_revision_watchers();

    assert!(take_local_paint_invalidated(changed), "the element whose revision moved");
    assert!(!take_local_paint_invalidated(unchanged), "an element whose revision did not");
}

#[test]
fn a_revision_that_matches_what_was_recorded_invalidates_nothing() {
    let element = ElementId::next();
    let source = Rc::new(Source(Cell::new(3)));
    watch(element, &source, 3);

    note_paint_revision_changed();
    sync_paint_revision_watchers();

    assert!(!take_local_paint_invalidated(element));
}

#[test]
fn nothing_is_compared_until_a_change_is_reported() {
    let element = ElementId::next();
    let source = Rc::new(Source(Cell::new(1)));
    watch(element, &source, 1);
    note_paint_revision_changed();
    sync_paint_revision_watchers();

    // The value moves without being reported: polling is what the report pays for.
    source.0.set(2);
    sync_paint_revision_watchers();
    assert!(!take_local_paint_invalidated(element));

    note_paint_revision_changed();
    sync_paint_revision_watchers();
    assert!(take_local_paint_invalidated(element));
}

#[test]
fn a_source_that_is_gone_is_forgotten() {
    let element = ElementId::next();
    let source = Rc::new(Source(Cell::new(1)));
    watch(element, &source, 1);
    let before = watcher_count();

    drop(source);
    note_paint_revision_changed();
    sync_paint_revision_watchers();

    assert_eq!(watcher_count(), before - 1);
    assert!(!take_local_paint_invalidated(element));
}

#[test]
fn registering_again_replaces_the_earlier_registration() {
    let element = ElementId::next();
    let (old, new) = (Rc::new(Source(Cell::new(1))), Rc::new(Source(Cell::new(5))));
    watch(element, &old, 1);
    let before = watcher_count();
    watch(element, &new, 5);
    assert_eq!(watcher_count(), before, "one element, one registration");

    old.0.set(9);
    note_paint_revision_changed();
    sync_paint_revision_watchers();
    assert!(!take_local_paint_invalidated(element), "the replaced source is not watched");
}
