//! Shaped strings kept across frames, and when they are let go.
//!
//! Shaping is width-independent, so its result survives a window resize and
//! is the input to every re-layout. It is also the largest thing the text
//! pipeline caches per string: a cluster, its string and its glyphs for every
//! grapheme. [`ShapingCache`] therefore evicts by recency under both an entry
//! capacity and a byte budget, through [`RecencyCache`], instead of clearing
//! everything when a count is crossed.
//!
//! An entry is stamped whenever a frame lays out or confirms the span it
//! belongs to, which keeps the screen's own strings resident while strings no
//! later frame asks for again (a counter, a clock, streamed text) age out.

use std::sync::Arc;

use super::cache_key::ShapingCacheKey;
use super::recency_cache::RecencyCache;
use super::text_layout::ShapedText;

/// A recency-evicting map from [`ShapingCacheKey`] to shaped text.
///
/// Like [`LayoutCache`](super::layout_cache::LayoutCache), it never sheds an
/// entry mid-frame: [`begin_frame`](Self::begin_frame) is the only place
/// eviction happens.
pub(super) struct ShapingCache {
    cache: RecencyCache<ShapingCacheKey, Arc<ShapedText>>,
}

impl ShapingCache {
    /// Creates an empty cache that starts evicting stale entries once more
    /// than `capacity` entries or `byte_budget` bytes are resident at the
    /// start of a frame.
    pub(super) fn new(capacity: usize, byte_budget: usize) -> Self {
        Self {
            cache: RecencyCache::new(capacity, byte_budget),
        }
    }

    /// Opens a frame: when the cache has grown past its capacity or budget,
    /// drops every entry no recent frame has used.
    pub(super) fn begin_frame(&mut self) {
        self.cache.begin_frame();
    }

    /// Returns the shaped text for `key` without stamping it.
    pub(super) fn get(&self, key: &ShapingCacheKey) -> Option<&Arc<ShapedText>> {
        self.cache.peek(key)
    }

    /// Whether `key` is resident, stamping it as used by the current frame
    /// when it is.
    pub(super) fn touch(&mut self, key: &ShapingCacheKey) -> bool {
        self.cache.touch(key)
    }

    /// Inserts shaped text, stamped as used by the current frame.
    pub(super) fn insert(&mut self, key: ShapingCacheKey, shaped: Arc<ShapedText>) {
        let bytes = shaped.heap_bytes();
        self.cache.insert(key, shaped, bytes);
    }

    /// Inserts every shaped text of `shaped`, each stamped as used by the
    /// current frame.
    pub(super) fn extend(
        &mut self,
        shaped: impl IntoIterator<Item = (ShapingCacheKey, Arc<ShapedText>)>,
    ) {
        for (key, text) in shaped {
            self.insert(key, text);
        }
    }

    /// Estimated bytes of every resident shaped text.
    pub(super) fn bytes(&self) -> usize {
        self.cache.bytes()
    }
}
