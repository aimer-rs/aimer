//! Changes to the set of children an element exposes without being replaced.
//!
//! [`element_tree_generation`](super::element_tree_generation) advances when a
//! generated subtree is replaced. A windowed list does something different: it
//! stays where it is and slides the run of rows it keeps alive, building the
//! rows that scroll into view and dropping the ones that scroll out. Nothing
//! about the element changed, so no replacement is logged and the tree
//! generation stands still, yet `visit_children` now answers differently.
//!
//! Consumers that hold element addresses across frames, such as the event
//! dispatcher's path index, must not keep a pointer to a row that was dropped.
//! They watch this generation as well.

use std::sync::atomic::{AtomicU64, Ordering};

use super::notify_retained_render_structure_changed;

static LIVE_CHILDREN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Returns how many times an element changed which children it exposes.
///
/// A consumer that cached element addresses at generation `g` and finds the
/// generation still at `g` knows no windowed element dropped or built a child
/// since.
#[doc(hidden)]
#[inline]
pub fn live_children_generation() -> u64 {
    LIVE_CHILDREN_GENERATION.load(Ordering::Acquire)
}

/// Notifies the tree that an element's live children changed in place.
///
/// Call this from a container that builds or drops children while it stays in
/// the tree, after the change and before anything dispatches an event. It
/// invalidates every cached element address, advances
/// [`retained_render_structure_generation`](super::retained_render_structure_generation)
/// and, unlike [`notify_element_tree_changed`](super::notify_element_tree_changed),
/// leaves layout and paint damage alone: the container already knows what it
/// laid out.
#[doc(hidden)]
#[inline]
pub fn notify_live_children_changed() {
    LIVE_CHILDREN_GENERATION.fetch_add(1, Ordering::Release);
    notify_retained_render_structure_changed();
}
