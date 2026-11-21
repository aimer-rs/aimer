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

    /// Reads the first resident entry among `primary` and `fallback` through `read`, stamping it as
    /// used by the current frame: one question to the table when the primary is resident, which is
    /// what a frame that draws a layout it drew before finds. Stamping with [`touch`](Self::touch)
    /// and reading with [`peek`](Self::peek) asked twice.
    ///
    /// `read` gets the value and not a borrow that outlives the call: the borrow checker does not allow
    /// a reference to be returned from one lookup and the table asked again on the miss.
    pub(super) fn with_first<R>(&mut self, primary: &K, fallback: Option<&K>, read: impl FnOnce(&V) -> R) -> Option<R> {
        let generation = self.generation;
        if let Some(slot) = self.entries.get_mut(primary) {
            slot.last_used = generation;
            return Some(read(&slot.value));
        }
        let slot = self.entries.get_mut(fallback?)?;
        slot.last_used = generation;
        Some(read(&slot.value))
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

    /// A key that counts how often it is hashed, which is how many times the table was asked.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Counted(u32);

    thread_local! {
        static HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    impl std::hash::Hash for Counted {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            HASHES.with(|count| count.set(count.get() + 1));
            self.0.hash(state);
        }
    }

    fn hashes_while<R>(work: impl FnOnce() -> R) -> (R, usize) {
        let before = HASHES.with(std::cell::Cell::get);
        let result = work();
        (result, HASHES.with(std::cell::Cell::get) - before)
    }

    /// A read of a layout that is stamped and read back is one question to the table, not two.
    #[test]
    fn the_first_resident_of_two_keys_is_stamped_and_read_in_one_lookup_each() {
        let mut cache = RecencyCache::new(usize::MAX, usize::MAX);
        cache.begin_frame();
        cache.insert(Counted(1), "one", 1);
        cache.insert(Counted(2), "two", 1);
        let (primary, fallback, absent) = (Counted(1), Counted(2), Counted(9));

        let (read, hashed) = hashes_while(|| cache.with_first(&primary, Some(&fallback), |value| value.len()));
        assert_eq!((read, hashed), (Some(3), 1), "the primary is resident: one lookup");

        let (read, hashed) = hashes_while(|| cache.with_first(&absent, Some(&fallback), |value| *value));
        assert_eq!((read, hashed), (Some("two"), 2), "the fallback answers after the primary missed");

        let (read, hashed) = hashes_while(|| cache.with_first(&absent, None, |value| *value));
        assert_eq!((read, hashed), (None, 1), "neither: one lookup, and no second key to ask");

        let (read, hashed) = hashes_while(|| cache.with_first(&absent, Some(&Counted(8)), |value| *value));
        assert_eq!((read, hashed), (None, 2));
    }

    #[test]
    fn what_is_read_first_is_stamped_as_used_and_the_one_that_missed_is_not_made() {
        let mut cache = RecencyCache::new(1, usize::MAX);
        cache.begin_frame();
        cache.insert(1u32, "kept", 1);
        cache.insert(2u32, "stale", 1);
        for _ in 0..3 {
            cache.begin_frame();
            assert_eq!(cache.with_first(&1, Some(&5), |value| *value), Some("kept"));
        }
        // Over capacity: what no frame read is shed, and what was read first stays.
        assert!(cache.peek(&1).is_some());
        assert!(cache.peek(&2).is_none());
        assert_eq!(cache.len(), 1);
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

    /// What a frame asks of the layout cache for every span of every text it draws: build the key (the text is
    /// the `Arc<str>` a request carries, a new allocation when its text was recorded again) and look it up.
    /// A timing harness, to run by itself in the debug build
    /// (`cargo test -p aimer_text_pipeline layout_key_timing -- --ignored --nocapture`). In the debug build a
    /// lookup is about a fifth building the key, a third hashing it and the rest the probe and the compare
    /// of the text, which is `hashbrown`'s own code unoptimized: a hasher that is cheaper changes none of it.
    #[test]
    #[ignore = "a timing harness: run by itself, in the debug build"]
    fn layout_key_timing() {
        use std::sync::Arc;

        use crate::cache_key::LayoutCacheKey;
        use crate::font::{FontFamily, FontStyle};
        use crate::text_layout::TextWritingMode;

        const SPANS: usize = 4000;
        const FRAMES: usize = 60;
        let texts: Vec<String> = (0..SPANS).map(|n| format!("let value_{n} = compute_something({n}, \"a string\");")).collect();
        let key = |text: Arc<str>| {
            LayoutCacheKey::new_with_writing_mode(
                text,
                14.0,
                0.0,
                0.0,
                FontFamily::SANS_SERIF,
                FontStyle::Normal,
                400,
                None,
                TextWritingMode::HorizontalTb,
            )
        };
        let mut cache = RecencyCache::new(usize::MAX, usize::MAX);
        cache.begin_frame();
        for text in &texts {
            cache.insert(key(Arc::from(text.as_str())), (), 100);
        }
        // The texts of the frames: equal to what the cache holds, and not the same allocations.
        let requests: Vec<Arc<str>> = texts.iter().map(|text| Arc::from(text.as_str())).collect();
        let started = std::time::Instant::now();
        let mut hits = 0usize;
        for _ in 0..FRAMES {
            cache.begin_frame();
            for text in &requests {
                if cache.get(&key(Arc::clone(text))).is_some() {
                    hits += 1;
                }
            }
        }
        let per = started.elapsed() / (FRAMES * SPANS) as u32;
        println!("layout key build + lookup: {per:?} per span ({SPANS} spans a frame) [{hits}]");

        // What a frame does for a layout it draws: stamp it, and read what it holds.
        let keys: Vec<LayoutCacheKey> = requests.iter().map(|text| key(Arc::clone(text))).collect();
        let started = std::time::Instant::now();
        let mut read = 0usize;
        for _ in 0..FRAMES {
            cache.begin_frame();
            for key in &keys {
                if cache.touch(key) {
                    read += usize::from(cache.peek(key).is_some());
                }
            }
        }
        println!("  stamp then read (touch + peek): {:?} per span", started.elapsed() / (FRAMES * SPANS) as u32);
        let started = std::time::Instant::now();
        for _ in 0..FRAMES {
            cache.begin_frame();
            for key in &keys {
                read += usize::from(cache.with_first(key, None, |_| ()).is_some());
            }
        }
        println!("  stamp and read (with_first):    {:?} per span [{read}]", started.elapsed() / (FRAMES * SPANS) as u32);
    }
}
