//! Telling a subtree that a value its paint is recorded from has changed.
//!
//! A container compares the revision of its decoration with the one it recorded
//! each time it is walked, so a button's hover changes what the next walk records
//! without a rebuild, a generation change or an invalidation. A subtree left alone
//! for a frame would never see it.
//!
//! The owner of such a value says that it changed with
//! [`note_paint_revision_changed`], which costs a counter. A container that paints
//! from one registers with [`watch_paint_revision`], and when the counter has moved
//! since the last frame, [`sync_paint_revision_watchers`] marks exactly the
//! containers whose revision differs from the one they recorded as invalidated, so
//! the subtree check sees them and nothing else.

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use aimer_cupid::utilities::IdBuildHasher;

use super::*;

/// A value an element records its paint from, that can change without a rebuild.
#[doc(hidden)]
pub trait PaintRevisionSource {
    /// The current revision of the value.
    fn revision(&self) -> u64;
}

struct Watch {
    source: Weak<dyn PaintRevisionSource>,
    /// The revision the element recorded its paint from, shared with the element.
    recorded: Rc<Cell<Option<u64>>>,
}

aimer_tls::thread_local! {
    static WATCHERS: RefCell<HashMap<ElementId, Watch, IdBuildHasher>> =
        RefCell::new(HashMap::default());
    /// The epoch the watchers were last compared at.
    static POLLED_EPOCH: Cell<u64> = const { Cell::new(0) };
}

/// Registers `element` as painting from `source`, whose revision it last recorded
/// its paint from is held in `recorded`. Registering again replaces the earlier
/// registration.
#[doc(hidden)]
pub fn watch_paint_revision(
    element: ElementId,
    source: Weak<dyn PaintRevisionSource>,
    recorded: Rc<Cell<Option<u64>>>,
) {
    WATCHERS.with(|watchers| {
        watchers.borrow_mut().insert(element, Watch { source, recorded });
    });
}

/// Marks every watching element whose source moved since it recorded as
/// invalidated. Does nothing unless a revision was reported changed since the last
/// call, and forgets watchers whose source is gone.
pub(crate) fn sync_paint_revision_watchers() {
    let epoch = paint_revision_epoch();
    if POLLED_EPOCH.with(|polled| polled.replace(epoch)) == epoch {
        return;
    }
    let mut stale = Vec::new();
    WATCHERS.with(|watchers| {
        for (element, watch) in watchers.borrow().iter() {
            match watch.source.upgrade() {
                None => stale.push(*element),
                Some(source) => {
                    if watch.recorded.get() != Some(source.revision()) {
                        mark_paint_element_invalidated(*element);
                    }
                }
            }
        }
    });
    if !stale.is_empty() {
        WATCHERS.with(|watchers| {
            let mut watchers = watchers.borrow_mut();
            for element in stale {
                watchers.remove(&element);
            }
        });
    }
}

#[cfg(test)]
pub(crate) fn watcher_count() -> usize {
    WATCHERS.with(|watchers| watchers.borrow().len())
}
