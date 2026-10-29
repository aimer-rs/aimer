//! Regression tests for a container that changes its live children in place.
//!
//! A windowed list builds and drops rows while it stays in the tree, so neither
//! its identity nor the element-tree generation moves. The path index still has
//! to forget the dropped rows: it holds their addresses, and a row that was
//! freed and whose memory was reused read as garbage the next time an event
//! walked the index. The tests keep the dropped rows alive so a stale index
//! shows up as a failed assertion instead of a use-after-free.

use std::cell::UnsafeCell;

use super::*;

/// Shared access to the rows of a [`Window`] after the window is boxed.
#[derive(Clone)]
struct WindowHandle(Rc<UnsafeCell<Vec<AnyElement>>>);

impl WindowHandle {
    /// Replaces the live rows and returns the ones that left the window.
    fn slide(&self, rows: Vec<AnyElement>) -> Vec<AnyElement> {
        // SAFETY: the tests are single-threaded and never hold a borrow of the
        // rows across this call.
        std::mem::replace(unsafe { &mut *self.0.get() }, rows)
    }
}

struct Window {
    rows: Rc<UnsafeCell<Vec<AnyElement>>>,
}

fn window(rows: Vec<AnyElement>) -> (AnyElement, WindowHandle) {
    let rows = Rc::new(UnsafeCell::new(rows));
    let element = Window { rows: rows.clone() }.boxed();
    (element, WindowHandle(rows))
}

impl VisitorElement for Window {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        // SAFETY: see `WindowHandle::slide`; the borrow ends with the visit.
        for row in unsafe { &*self.rows.get() } {
            visitor(row.as_ref());
        }
    }

    fn debug_name(&self) -> &'static str {
        "LiveChildrenWindow"
    }
}

impl EventElement for Window {}
impl LayoutElement for Window {}
impl Drawable for Window {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Window {}

struct Row;

impl VisitorElement for Row {
    fn debug_name(&self) -> &'static str {
        "LiveChildrenRow"
    }
}

impl EventElement for Row {}
impl LayoutElement for Row {}
impl Drawable for Row {
    fn update(&self, _ctx: &BuildContext) {}
}
impl Rebuildable for Row {}

/// The dropped row must stop resolving as soon as the container reports the
/// change, even though nothing else about the tree moved.
#[test]
fn a_slid_window_makes_the_index_forget_dropped_rows() {
    let leaving = Row.boxed();
    let leaving_id = leaving.id();
    let (root, handle) = window(vec![leaving]);
    let mut dispatcher = EventDispatcher::new();
    assert!(dispatcher.resolve_element(root.as_ref(), leaving_id).is_some());
    assert_eq!(dispatcher.path_index_work(), (1, 0));

    let entering = Row.boxed();
    let entering_id = entering.id();
    // Held so the dropped row stays valid memory for the assertions.
    let _dropped = handle.slide(vec![entering]);
    notify_live_children_changed();

    assert!(
        dispatcher.resolve_element(root.as_ref(), leaving_id).is_none(),
        "a row that left the window must not resolve through the stale index"
    );
    assert!(dispatcher.resolve_element(root.as_ref(), entering_id).is_some());
    assert_eq!(dispatcher.path_index_work(), (2, 0));
}

/// The check must not depend on the frame epoch: the window slides during
/// layout, inside the epoch whose first look at the tree found it unchanged.
#[test]
fn a_slide_in_the_same_event_frame_is_noticed() {
    let leaving = Row.boxed();
    let leaving_id = leaving.id();
    let (root, handle) = window(vec![leaving]);
    let mut dispatcher = EventDispatcher::new();
    begin_event_frame();
    assert!(dispatcher.resolve_element(root.as_ref(), leaving_id).is_some());

    let _dropped = handle.slide(vec![Row.boxed()]);
    notify_live_children_changed();

    assert!(dispatcher.resolve_element(root.as_ref(), leaving_id).is_none());
}

/// A rebuilt index is current: the same change is not reacted to twice.
#[test]
fn a_reported_slide_rebuilds_the_index_once() {
    let (root, handle) = window(vec![Row.boxed()]);
    let mut dispatcher = EventDispatcher::new();
    let _ = dispatcher.resolve_element(root.as_ref(), root.id());

    let _dropped = handle.slide(vec![Row.boxed()]);
    notify_live_children_changed();
    for _ in 0..3 {
        begin_event_frame();
        let _ = dispatcher.resolve_element(root.as_ref(), root.id());
    }

    assert_eq!(dispatcher.path_index_work(), (2, 0));
}
