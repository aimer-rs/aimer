//! Keyed scroll-offset store — the Flutter `PageStorage` equivalent.
//!
//! Reconciliation (`adopt_scroll_state`) only carries a scroll position when
//! the old element still exists to copy from. A *full teardown* — a swapped
//! tab, an `if/else` branch, a list item that unmounts — has no old element, so
//! on the next build the `Scrollable` would re-seed from
//! `ScrollBehavior::scroll_offset` (usually the top). Flutter solves this by
//! writing the offset into a `PageStorageBucket` that lives outside the widget
//! subtree; this store plays the same role, keyed by a `Scrollable`'s explicit
//! `storage_key`.
//!
//! Offsets are kept in *logical* (unscaled) pixels, mirroring
//! `ScrollBehavior::scroll_offset`, so the saved value survives a DPI/scale
//! change between teardown and re-creation (the reader re-applies `ctx.scale`).
//!
//! ponytail: the render pipeline is single-threaded, so this is a
//! `thread_local` map with no lock. Entries are never evicted, so an app that
//! churns through unbounded unique `storage_key`s grows this map without limit.
//! Upgrade path: evict on an explicit dispose hook, or cap it with an LRU.

use aimer_attribute::position::Vec2d;
use aimer_widget::Key;
use hashbrown::HashMap;
use std::cell::RefCell;

thread_local! {
    static SCROLL_OFFSETS: RefCell<HashMap<Key, Vec2d>> = RefCell::new(HashMap::new());
}

/// Save a scrollable's live *logical* (unscaled) offset under its storage key.
pub(crate) fn save_offset(key: &Key, logical_offset: Vec2d) {
    SCROLL_OFFSETS.with(|m| {
        m.borrow_mut().insert(key.clone(), logical_offset);
    });
}

/// Read the last saved logical offset for a storage key, if one was stored.
pub(crate) fn read_offset(key: &Key) -> Option<Vec2d> {
    SCROLL_OFFSETS.with(|m| m.borrow().get(key).copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Teardown-survival contract: an offset saved under a key is read back for a
    // fresh Scrollable with the same key, while an unknown key restores nothing
    // (so it falls back to the declared scroll_behavior offset).
    #[test]
    fn save_then_read_round_trips() {
        let key = Key::Value("list-a".into());
        assert!(read_offset(&key).is_none(), "unknown key restores nothing");

        save_offset(&key, Vec2d { x: 0.0, y: 240.0 });
        let restored = read_offset(&key).expect("saved offset restores");
        assert_eq!(restored.y, 240.0);

        // A later save overwrites (the store always holds the latest position).
        save_offset(&key, Vec2d { x: 0.0, y: 55.0 });
        assert_eq!(read_offset(&key).expect("latest offset restores").y, 55.0);

        // A different key is independent.
        assert!(read_offset(&Key::Value("list-b".into())).is_none());
    }
}
