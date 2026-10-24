//! Framework-level `PageStorage` — a keyed store that outlives the widget
//! subtree.
//!
//! A window resize (or a tab swap, an `if/else` branch flip, an unmounted list
//! item) rebuilds and can fully tear down everything under a `Scrollable`,
//! re-running each nested `StatefulWidget::create_state()`. Reconciliation
//! carry only preserves state when the *old* element still exists to copy from,
//! so it cannot survive a full teardown.
//!
//! The framework already solves this for scroll position with a private,
//! scroll-only `thread_local` map (`aimer_container`'s `scroll_storage`). This
//! is the same mechanism, generalized and made public so **any** stateful
//! widget can persist **any** value across a rebuild by parking it here —
//! without each widget declaring its own `thread_local!`. It is Flutter's
//! `PageStorage`.
//!
//! Usage — two lines, no per-widget `thread_local!`:
//!
//! ```rust,ignore
//! use aimer::page_storage;
//!
//! // in create_state(): re-seed from the store instead of a hardcoded default
//! let current_index = page_storage::read_or("same-looking-tab", 0usize);
//!
//! // in the event handler that changes it: write through so a later rebuild restores it
//! page_storage::write("same-looking-tab", index);
//! ```
//!
//! ponytail: the render pipeline is single-threaded, so this is a
//! `thread_local` map with no lock — mirroring `scroll_storage`. Values are
//! type-erased (`Box<dyn Any>`); a `read::<T>` for the wrong `T` returns `None`
//! rather than panicking. Entries are never evicted, so an app that churns
//! through unbounded unique keys grows this map without limit. Upgrade path:
//! evict on an explicit dispose hook, or cap it with an LRU (same ceiling as
//! `scroll_storage`).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use crate::Key;

thread_local! {
    static STORE: RefCell<HashMap<Key, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Persist `value` under `key`, overwriting any previous value. Call this
/// wherever the value changes (e.g. inside a button's `on_press`) so a later
/// teardown/rebuild can restore it.
#[track_caller]
pub fn write<T: 'static>(key: impl Into<Key>, value: T) {
    let caller = Location::caller();
    let key = key.into().with_location(caller);
    STORE.with(|m| {
        m.borrow_mut().insert(key, Box::new(value));
    });
}

/// Read a clone of the value stored under `key`, if one was stored *and* it has
/// type `T`. Returns `None` for an unknown key or a type mismatch — so a widget
/// falls back to its own default on first build.
pub fn read<T: Clone + 'static>(key: impl Into<Key>) -> Option<T> {
    let key = key.into();
    STORE.with(|m| {
        m.borrow()
            .get(&key)
            .and_then(|v| v.downcast_ref::<T>().cloned())
    })
}

/// Read the stored value for `key`, or `default` if nothing (of type `T`) is
/// stored yet. The ergonomic form for re-seeding `create_state()`.
pub fn read_or<T: Clone + 'static>(key: impl Into<Key>, default: T) -> T {
    read(key).unwrap_or(default)
}

/// Drop the stored value for `key`. Use when a value should not outlive its
/// widget (the manual counterpart to the not-yet-implemented dispose hook).
pub fn remove(key: impl Into<Key>) {
    let key = key.into();
    STORE.with(|m| {
        m.borrow_mut().remove(&key);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Teardown-survival contract: a value written under a key is read back for a
    // fresh widget with the same key (the resize/rebuild path), an unknown key
    // restores the default, and a wrong-type read does not panic.
    #[test]
    fn write_then_read_round_trips() {
        assert_eq!(
            read_or("tab", 0usize),
            0,
            "unknown key falls back to default"
        );

        write("tab", 1usize); // user picked iOS
        assert_eq!(read::<usize>("tab"), Some(1), "written value restores");
        assert_eq!(
            read_or("tab", 0usize),
            1,
            "read_or restores the picked value, not the default"
        );

        // A later write overwrites (the store always holds the latest value).
        write("tab", 2usize);
        assert_eq!(read_or("tab", 0usize), 2);

        // A different key is independent.
        assert_eq!(read::<usize>("other"), None);

        // Wrong-type read returns None instead of panicking.
        assert_eq!(read::<String>("tab"), None);

        // remove clears it, so the next read falls back to the default.
        remove("tab");
        assert_eq!(read_or("tab", 0usize), 0);
    }
}
