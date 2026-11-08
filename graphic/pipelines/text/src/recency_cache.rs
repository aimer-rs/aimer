//! A cache that sheds what no recent frame read, bounded by count and bytes.
//!
//! The text pipeline keeps shaped strings and positioned layouts across
//! frames. Their cost varies by orders of magnitude — a label is a kilobyte, a
//! paragraph tens of kilobytes — so a bound on the *number* of entries says
//! little about memory, and clearing everything once the bound is crossed
//! punishes exactly the frame that can least afford it: it rebuilds the whole
//! screen from scratch.
//!
//! [`RecencyCache`] evicts by recency instead. Every read and insert stamps the
//! entry with the current frame's generation. When a frame *begins* with the
//! cache over its entry capacity or its byte budget, the entries no recent
//! frame read are dropped. The previous frame's working set always survives,
//! so a flood of strings nothing will ask for again — a counter, a clock, a
//! streamed reply, a resize's transient widths — is shed without ever taking
//! the live screen with it.
//!
//! Eviction runs only in [`begin_frame`](RecencyCache::begin_frame), never
//! mid-frame, so a key that a frame confirmed is resident stays resident until
//! that frame ends. Both bounds are soft: a single frame that genuinely uses
//! more than the budget keeps all of it, since evicting the working set would
//! only force the next frame to rebuild it.

use std::hash::Hash;

use hashbrown::HashMap;

/// How many frames an entry may go unread before an over-budget frame is
/// allowed to evict it.
///
/// One: eviction runs at the start of a frame, before that frame stamps
/// anything, so the entries the *previous* frame used — the screen's current
/// working set — are exactly the ones a linger of one frame protects.
const LINGER_FRAMES: u64 = 1;

struct Slot<V> {
    value: V,
    /// What the caller said this entry costs when it was inserted.
    bytes: usize,
    /// The last frame that read or inserted the entry.
    last_used: u64,
}

/// A recency-evicting map bounded by entry count and by bytes.
pub(super) struct RecencyCache<K, V> {
    entries: HashMap<K, Slot<V>>,
    /// The current frame number; advanced by [`begin_frame`](Self::begin_frame).
    generation: u64,
    capacity: usize,
    byte_budget: usize,
    /// Sum of the resident entries' `bytes`.
    bytes: usize,
}

impl<K: Eq + Hash, V> RecencyCache<K, V> {
    /// Creates an empty cache that starts evicting stale entries once more
    /// than `capacity` entries or more than `byte_budget` bytes are resident
    /// at the start of a frame.
    pub(super) fn new(capacity: usize, byte_budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            capacity,
            byte_budget,
            bytes: 0,
        }
    }

    /// Opens a frame. When the cache is over its capacity or byte budget,
    /// drops every entry that no frame within [`LINGER_FRAMES`] has read.
    ///
    /// Called once per frame before any lookups, so eviction can never
    /// invalidate an entry the current frame already confirmed.
    pub(super) fn begin_frame(&mut self) {
        self.generation += 1;
        if self.entries.len() <= self.capacity && self.bytes <= self.byte_budget {
            return;
        }
        let oldest_kept = self.generation.saturating_sub(LINGER_FRAMES);
        let mut freed = 0;
        self.entries.retain(|_, slot| {
            let keep = slot.last_used >= oldest_kept;
            if !keep {
                freed += slot.bytes;
            }
            keep
        });
        self.bytes -= freed;
    }

    /// Returns the entry for `key`, stamping it as used by the current frame.
    pub(super) fn get(&mut self, key: &K) -> Option<&V> {
        let generation = self.generation;
        self.entries.get_mut(key).map(|slot| {
            slot.last_used = generation;
            &slot.value
        })
    }

    /// Returns the entry for `key` without stamping it.
    ///
    /// For re-reads within a frame that already confirmed the key with
    /// [`get`](Self::get) or [`touch`](Self::touch): the stamp is in place, so
    /// the second read can skip the write.
    pub(super) fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|slot| &slot.value)
    }

    /// Stamps `key` as used by the current frame; whether it is resident.
    pub(super) fn touch(&mut self, key: &K) -> bool {
        self.get(key).is_some()
    }

    /// Inserts `value`, charged at `bytes`, stamped as used by the current
    /// frame. Replaces and re-charges any entry already under `key`.
    pub(super) fn insert(&mut self, key: K, value: V, bytes: usize) {
        let slot = Slot {
            value,
            bytes,
            last_used: self.generation,
        };
        self.bytes += bytes;
        if let Some(replaced) = self.entries.insert(key, slot) {
            self.bytes -= replaced.bytes;
        }
    }

    /// How many entries are currently resident.
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The summed cost of the resident entries.
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::RecencyCache;

    #[test]
    fn a_hit_returns_what_was_inserted() {
        let mut cache = RecencyCache::new(8, usize::MAX);
        cache.begin_frame();
        cache.insert("a", 1, 10);

        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.peek(&"a"), Some(&1));
        assert!(cache.touch(&"a"));
        assert!(!cache.touch(&"b"));
    }

    #[test]
    fn under_both_bounds_nothing_is_ever_evicted() {
        let mut cache = RecencyCache::new(8, 100);
        cache.begin_frame();
        cache.insert("a", 1, 10);

        for _ in 0..100 {
            cache.begin_frame();
        }

        assert_eq!(cache.peek(&"a"), Some(&1));
        assert_eq!(cache.bytes(), 10);
    }

    #[test]
    fn bytes_follow_inserts_replacements_and_evictions() {
        let mut cache = RecencyCache::new(usize::MAX, 15);
        cache.begin_frame();
        cache.insert("a", 1, 10);
        cache.insert("b", 2, 10);
        assert_eq!(cache.bytes(), 20);

        cache.insert("a", 3, 4);
        assert_eq!(cache.bytes(), 14, "a replacement is charged once");
        assert_eq!(cache.len(), 2);

        // Over budget would need 15; 14 is under, so nothing goes.
        cache.begin_frame();
        cache.begin_frame();
        assert_eq!(cache.len(), 2);

        cache.insert("c", 4, 10);
        cache.begin_frame();
        cache.begin_frame();
        assert_eq!(cache.bytes(), 10, "only the entry inserted last frame stays");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn over_the_byte_budget_only_unread_entries_are_evicted() {
        let mut cache = RecencyCache::new(usize::MAX, 20);
        cache.begin_frame();
        cache.insert("hot", 1, 10);
        cache.insert("cold", 2, 10);
        cache.insert("colder", 3, 10);

        // The next frame reads only "hot".
        cache.begin_frame();
        assert!(cache.touch(&"hot"));

        // Over budget: what no frame within the linger read goes.
        cache.begin_frame();
        assert!(cache.peek(&"hot").is_some());
        assert!(cache.peek(&"cold").is_none());
        assert!(cache.peek(&"colder").is_none());
        assert_eq!(cache.bytes(), 10);
    }

    #[test]
    fn over_the_entry_capacity_only_unread_entries_are_evicted() {
        let mut cache = RecencyCache::new(2, usize::MAX);
        cache.begin_frame();
        cache.insert("a", 1, 1);
        cache.insert("b", 2, 1);
        cache.insert("c", 3, 1);

        cache.begin_frame();
        assert!(cache.touch(&"a"));

        cache.begin_frame();
        assert!(cache.peek(&"a").is_some());
        assert!(cache.peek(&"b").is_none());
        assert!(cache.peek(&"c").is_none());
    }

    // Eviction must never take the previous frame's working set: those are
    // the entries the screen is showing right now.
    #[test]
    fn the_previous_frames_working_set_survives_an_over_budget_frame() {
        let mut cache = RecencyCache::new(usize::MAX, 5);
        cache.begin_frame();
        cache.insert("a", 1, 10);
        cache.insert("b", 2, 10);

        cache.begin_frame();

        assert!(cache.peek(&"a").is_some());
        assert!(cache.peek(&"b").is_some());
    }

    #[test]
    fn a_stream_of_one_use_entries_stays_within_the_budget() {
        const ENTRY: usize = 1_000;
        let mut cache = RecencyCache::new(usize::MAX, 8 * ENTRY);
        let mut largest = 0;
        for frame in 0..1_000_u32 {
            cache.begin_frame();
            cache.insert(frame, frame, ENTRY);
            largest = largest.max(cache.bytes());
        }

        // Over budget by at most the entry that pushed it over.
        assert!(largest <= 8 * ENTRY + ENTRY, "grew to {largest} bytes");
        assert!(cache.bytes() <= 9 * ENTRY);
    }

    #[test]
    fn a_repeatedly_read_entry_outlives_a_flood() {
        const ENTRY: usize = 1_000;
        let mut cache = RecencyCache::new(usize::MAX, 4 * ENTRY);
        cache.begin_frame();
        cache.insert("static", 0_u32, ENTRY);

        for frame in 1..200_u32 {
            cache.begin_frame();
            assert!(cache.touch(&"static"), "evicted at frame {frame}");
            cache.insert("flood", frame, ENTRY);
        }
    }
}
