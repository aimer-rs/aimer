//! Sparing a child a geometry forward it is about to repeat itself.
//!
//! Wrappers forward `sync_paint_geometry` to their child so that a subtree whose
//! own update does not run still publishes current bounds. When the child's
//! update does run, it syncs its own geometry from the context it is really
//! drawn with, and the earlier forward only did the same work first. Over a
//! chain of wrappers that is repeated at every level.
//!
//! A forward may be skipped when someone has promised that the child's update
//! follows, and the child itself shows that it will sync its own geometry
//! there: it has a render node holding a local paint, and no live compositor
//! animation that would take it down a path without a paint-state sync. The
//! promise comes from two places. An element syncing its own paint state makes
//! it for its children outside an animation-only pass, which leaves no subtree
//! unwalked. A wrapper whose `update` calls `child.update` right after may make
//! it for that one call.
//!
//! A forward that is not skipped runs with the promise withdrawn, so what it
//! forwards in turn is judged by its own situation and not by a promise made to
//! the element above it.

use std::cell::Cell;

// Plain thread-locals, not `aimer_tls`: both flags describe one synchronous call
// stack on the drawing thread, so they need no per-application scoping, and a
// const-initialised cell is a bare thread-local access even in debug builds.
std::thread_local! {
    /// Kill switch for diagnostics and differential tests.
    static SKIP_ENABLED: Cell<bool> = const { Cell::new(true) };
    /// Whether the child about to receive a forward is promised an update.
    static CHILD_UPDATE_FOLLOWS: Cell<bool> = const { Cell::new(false) };
}

/// Turns the skipping of redundant geometry forwards on or off.
///
/// On by default. Off, every wrapper forwards to its child as it did before the
/// skip existed, which is what differential tests compare against.
#[doc(hidden)]
pub fn set_geometry_forward_skip(enabled: bool) {
    SKIP_ENABLED.with(|flag| flag.set(enabled));
}

/// Whether redundant geometry forwards may be skipped at all.
#[inline]
pub(super) fn geometry_forward_skip_enabled() -> bool {
    SKIP_ENABLED.with(Cell::get)
}

/// Whether the child that would receive a forward now is promised an update.
#[inline]
pub(super) fn child_update_follows() -> bool {
    CHILD_UPDATE_FOLLOWS.with(Cell::get)
}

/// A scope in which the promise that a child's update follows is made, or
/// withdrawn. The previous state returns when it is dropped.
///
/// Make the promise only where `child.update` will really be called in the same
/// frame, whatever the child's own state: that call is what makes the skipped
/// forward redundant.
#[doc(hidden)]
#[must_use = "the promise lasts only while the guard is alive"]
pub struct ChildUpdateFollows {
    previous: bool,
}

impl ChildUpdateFollows {
    /// Sets the promise to `follows` until the guard is dropped.
    #[inline]
    pub fn enter(follows: bool) -> Self {
        let previous = CHILD_UPDATE_FOLLOWS.with(|flag| flag.replace(follows));
        Self { previous }
    }
}

impl Drop for ChildUpdateFollows {
    #[inline]
    fn drop(&mut self) {
        CHILD_UPDATE_FOLLOWS.with(|flag| flag.set(self.previous));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_promise_is_scoped_and_nests() {
        assert!(!child_update_follows());
        {
            let _outer = ChildUpdateFollows::enter(true);
            assert!(child_update_follows());
            {
                let _withdrawn = ChildUpdateFollows::enter(false);
                assert!(!child_update_follows());
            }
            assert!(child_update_follows());
        }
        assert!(!child_update_follows());
    }

    #[test]
    fn the_skip_is_on_until_switched_off() {
        assert!(geometry_forward_skip_enabled());
        set_geometry_forward_skip(false);
        assert!(!geometry_forward_skip_enabled());
        set_geometry_forward_skip(true);
        assert!(geometry_forward_skip_enabled());
    }
}
