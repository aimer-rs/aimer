//! A bounded map that discards the entries it has gone longest without being
//! asked for.
//!
//! The caches in front of text measurement are keyed by the string itself, so a
//! scrolled list of a hundred thousand distinct rows will always ask for keys the
//! cache has never seen. A cache that reacts to being full by emptying itself
//! then throws away the handful of entries the current frame is built from, and
//! every newly visible row becomes a guaranteed miss for the rest of the
//! session. Evicting the coldest entries instead keeps what is on screen.
//!
//! Eviction happens in batches: when the map is full, a quarter of it is dropped
//! at once. A batch costs `O(len)`, but it frees `capacity / 4` slots, so the
//! amortized cost per insertion stays constant — and unlike an intrusive
//! linked-list LRU it needs no extra allocation per entry and no `unsafe`.

use std::hash::{BuildHasher, Hash, Hasher};

use hashbrown::HashMap;
use hashbrown::hash_map::RawEntryMut;

/// Fraction of the map dropped by one eviction, as a divisor.
///
/// Dropping a quarter keeps the amortized cost of an insertion constant while
/// leaving three quarters of the working set untouched.
const EVICT_DIVISOR: usize = 4;

/// A stored value together with the moment it was last touched.
struct Entry<V> {
    value: V,
    /// Value of the map's clock when this entry was last inserted or read.
    used: u64,
}

/// A `HashMap` with an upper bound on how many entries it keeps.
pub(crate) struct LruMap<K, V> {
    entries: HashMap<K, Entry<V>>,
    capacity: usize,
    /// Monotonic counter standing in for time; incremented on every access.
    clock: u64,
}

impl<K: Eq + Hash, V> LruMap<K, V> {
    /// Creates a map holding at most `capacity` entries.
    ///
    /// A capacity of zero is raised to one, so an insertion is never silently
    /// dropped.
    #[inline]
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            capacity: capacity.max(1),
            clock: 0,
        }
    }

    /// Number of entries currently held.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Discards every entry.
    ///
    /// Needed when what the values were derived from changed — registering a
    /// font invalidates every measurement taken before it.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Borrows the value stored under `key`, marking it as recently used.
    ///
    /// Takes `&mut self` because a read is what "recently used" is defined by;
    /// an entry that is never read is the first to be evicted.
    #[inline]
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        let entry = self.entries.get_mut(key)?;
        entry.used = clock;
        Some(&entry.value)
    }

    /// Borrows a value with a lookup key that does not have to own the stored
    /// key's data. The matcher is used only after the map's hasher selects a
    /// bucket, so callers can compare a borrowed view of a composite key
    /// without allocating a temporary `K` on every hit.
    #[inline]
    pub(crate) fn get_by<Q, F>(&mut self, key: &Q, mut is_match: F) -> Option<&V>
    where
        Q: Hash + ?Sized,
        F: FnMut(&K, &Q) -> bool,
    {
        let mut hasher = self.entries.hasher().build_hasher();
        key.hash(&mut hasher);
        let hash = hasher.finish();
        self.clock += 1;
        let clock = self.clock;
        match self
            .entries
            .raw_entry_mut()
            .from_hash(hash, |stored| is_match(stored, key))
        {
            RawEntryMut::Occupied(entry) => {
                let entry = entry.into_mut();
                entry.used = clock;
                Some(&entry.value)
            }
            RawEntryMut::Vacant(_) => None,
        }
    }

    /// Stores `value` under `key`, evicting the coldest entries when full.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.clock += 1;
        if self.len() >= self.capacity && !self.entries.contains_key(&key) {
            self.evict();
        }
        self.entries.insert(
            key,
            Entry {
                value,
                used: self.clock,
            },
        );
    }

    /// Drops the coldest quarter of the entries.
    ///
    /// The cut-off is found by partial selection rather than by sorting, so the
    /// pass is linear in the number of entries.
    fn evict(&mut self) {
        let target = (self.len() / EVICT_DIVISOR).max(1);
        let mut stamps: Vec<u64> = self.entries.values().map(|entry| entry.used).collect();
        let (_, cutoff, _) = stamps.select_nth_unstable(target - 1);
        let cutoff = *cutoff;
        self.entries.retain(|_, entry| entry.used > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Filling the map must not empty it: the entries that are still being read
    /// are exactly the ones a frame is built from.
    #[test]
    fn an_overflowing_map_keeps_most_of_what_it_held() {
        let mut map: LruMap<usize, usize> = LruMap::new(100);

        for key in 0..400 {
            map.insert(key, key);
        }

        assert!(
            map.len() >= 100 / EVICT_DIVISOR,
            "held only {} entries",
            map.len()
        );
        assert!(map.len() <= 100, "held {} entries", map.len());
    }

    /// The point of the whole structure: a key that keeps being read survives
    /// any number of one-shot keys arriving after it.
    #[test]
    fn a_repeatedly_read_entry_is_never_evicted() {
        let mut map: LruMap<usize, usize> = LruMap::new(16);
        map.insert(usize::MAX, 7);

        for key in 0..1_000 {
            map.insert(key, key);
            assert_eq!(map.get(&usize::MAX), Some(&7), "the hot entry was dropped");
        }
    }

    /// An entry that was never read has to go before one that was.
    #[test]
    fn a_cold_entry_is_evicted_before_a_warm_one() {
        let mut map: LruMap<usize, usize> = LruMap::new(4);
        map.insert(0, 0);
        map.insert(1, 1);
        map.get(&1);

        // Overflow by one, which evicts a quarter of four entries: one.
        map.insert(2, 2);
        map.insert(3, 3);
        map.insert(4, 4);

        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&0), None);
    }

    /// Overwriting an existing key must not count against the capacity, or a map
    /// used as a memo of one hot key would evict on every write.
    #[test]
    fn overwriting_a_key_replaces_it() {
        let mut map: LruMap<&str, usize> = LruMap::new(1);

        map.insert("a", 1);
        map.insert("a", 2);

        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&"a"), Some(&2));
    }

    #[test]
    fn clearing_drops_everything() {
        let mut map: LruMap<usize, usize> = LruMap::new(8);
        map.insert(1, 1);

        map.clear();

        assert_eq!(map.len(), 0);
        assert_eq!(map.get(&1), None);
    }

    #[test]
    fn borrowed_lookup_matches_an_owned_key_without_allocating_one() {
        let mut map: LruMap<String, usize> = LruMap::new(8);
        map.insert("cached".to_owned(), 7);

        assert_eq!(
            map.get_by("cached", |stored, lookup| stored == lookup),
            Some(&7)
        );
    }
}
