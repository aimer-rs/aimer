use std::cell::Cell;

/// Caches whether an element's subtree contains any element that might
/// handle an event, so `dispatch_event`/`dispatch_captured_event_inner` can
/// skip purely decorative/structural subtrees under the pointer without
/// visiting every node inside them.
///
/// The cache is lazily populated (self-memoizing) on first query after
/// invalidation, and is cleared by the same `invalidate_layout()` sweep that
/// already visits the whole tree once per frame for [`LayoutCache`], so no
/// new per-frame full walk is introduced.
///
/// [`LayoutCache`]: crate::layout_cache::LayoutCache
pub struct InteractiveCache {
    value: Cell<Option<bool>>,
}

impl InteractiveCache {
    /// Creates an empty (uncomputed) cache.
    pub fn new() -> Self {
        Self {
            value: Cell::new(None),
        }
    }

    /// Returns the cached interactivity flag, or `None` if it hasn't been
    /// computed since the last invalidation.
    pub fn get(&self) -> Option<bool> {
        self.value.get()
    }

    /// Stores whether the subtree is interactive.
    pub fn set(&self, interactive: bool) {
        self.value.set(Some(interactive));
    }

    /// Clears the cached value (call at the start of each frame, alongside
    /// `LayoutCache::invalidate`).
    pub fn invalidate(&self) {
        self.value.set(None);
    }
}

impl Default for InteractiveCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_starts_empty() {
        let cache = InteractiveCache::new();
        assert_eq!(cache.get(), None);
    }

    #[test]
    fn set_then_get_round_trips_the_stored_value() {
        let cache = InteractiveCache::new();

        cache.set(true);
        assert_eq!(cache.get(), Some(true));

        cache.set(false);
        assert_eq!(cache.get(), Some(false));
    }

    #[test]
    fn invalidate_clears_a_previously_set_value() {
        let cache = InteractiveCache::new();

        cache.set(true);
        cache.invalidate();

        assert_eq!(cache.get(), None);
    }

    #[test]
    fn default_matches_new() {
        let cache = InteractiveCache::default();
        assert_eq!(cache.get(), None);
    }
}
