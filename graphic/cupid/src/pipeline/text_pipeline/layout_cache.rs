//! Positioned-glyph layouts kept across frames, and when they are let go.
//!
//! Layout is the last per-width stage of the text pipeline: shaping survives
//! a resize, rasterized bitmaps survive everything, but a wrapped layout is
//! keyed by the width it wraps at and must be recomputed when that width
//! changes. The cache therefore fills at very different rates in different
//! situations — a static screen inserts nothing, a scroll inserts a few lines
//! a frame, and a live window resize inserts *the whole visible document
//! every frame*, each frame under a width no later frame will ask for again.
//!
//! Clearing the whole cache when it grows past its capacity punishes exactly
//! the frame that can least afford it: the resize frame that flooded the
//! cache is followed by one that re-lays-out everything on screen from
//! scratch. [`LayoutCache`] evicts by recency instead. Every hit and insert
//! stamps the entry with the current frame's generation, and when a frame
//! begins over capacity, only the entries no recent frame read are dropped —
//! the working set of the previous frame always survives, so a flood of
//! transient-width entries is shed without ever taking the live screen's
//! layouts with it.

use hashbrown::HashMap;

use super::cache_key::LayoutCacheKey;
use super::text_layout::{PositionedGlyph, PositionedTextLayout};

/// How many frames an entry may go unread before an over-capacity frame is
/// allowed to evict it.
///
/// One: eviction runs at the start of a frame, before that frame stamps
/// anything, so the entries the *previous* frame used — the screen's current
/// working set — are exactly the ones a linger of one frame protects.
const LINGER_FRAMES: u64 = 1;

/// One cached layout: the glyphs, and the last frame that read them.
struct CachedLayout {
    layout: PositionedTextLayout,
    last_used: u64,
}

/// A recency-evicting map from [`LayoutCacheKey`] to positioned glyphs.
///
/// The cache never sheds an entry mid-frame: [`begin_frame`] is the only
/// place eviction happens, so a key that [`get`] or [`touch`] confirmed is
/// guaranteed to stay resident until the frame ends, which is what lets
/// `prepare` collect layouts early and read them again when it builds
/// instances.
///
/// `capacity` is a soft bound — a single frame that genuinely uses more
/// entries than the capacity keeps them all, since evicting the working set
/// would only force the next frame to rebuild it.
///
/// [`begin_frame`]: LayoutCache::begin_frame
/// [`get`]: LayoutCache::get
/// [`touch`]: LayoutCache::touch
pub(super) struct LayoutCache {
    entries: HashMap<LayoutCacheKey, CachedLayout>,
    /// The current frame number; advanced by [`begin_frame`](Self::begin_frame).
    generation: u64,
    capacity: usize,
}

impl LayoutCache {
    /// Creates an empty cache that starts evicting stale entries once more
    /// than `capacity` are resident at the start of a frame.
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            generation: 0,
            capacity,
        }
    }

    /// Opens a new frame: advances the generation and, when the cache has
    /// grown past its capacity, drops every entry no frame within
    /// [`LINGER_FRAMES`] has read.
    ///
    /// Called once per `prepare`, before any lookups, so eviction can never
    /// invalidate a layout the current frame already confirmed.
    pub(super) fn begin_frame(&mut self) {
        self.generation += 1;
        if self.entries.len() > self.capacity {
            let oldest_kept = self.generation.saturating_sub(LINGER_FRAMES);
            self.entries
                .retain(|_, entry| entry.last_used >= oldest_kept);
        }
    }

    /// Returns the cached glyphs for `key`, stamping the entry as used by the
    /// current frame.
    pub(super) fn get(&mut self, key: &LayoutCacheKey) -> Option<&[PositionedGlyph]> {
        let generation = self.generation;
        self.entries.get_mut(key).map(|entry| {
            entry.last_used = generation;
            entry.layout.glyphs.as_slice()
        })
    }

    /// Returns the cached glyphs for `key` without stamping it.
    ///
    /// For re-reads within a frame that already confirmed the key with
    /// [`get`](Self::get) or [`touch`](Self::touch) — the stamp is in place,
    /// so the second read can skip the write.
    pub(super) fn peek(&self, key: &LayoutCacheKey) -> Option<&[PositionedGlyph]> {
        self.entries
            .get(key)
            .map(|entry| entry.layout.glyphs.as_slice())
    }

    /// Whether `key` is resident, stamping it as used by the current frame
    /// when it is.
    pub(super) fn touch(&mut self, key: &LayoutCacheKey) -> bool {
        self.get(key).is_some()
    }

    /// Returns the first resident entry among `primary` and `fallback`,
    /// stamping it as used by the current frame.
    ///
    /// The two-key read serves canonicalized layouts: a span whose wrapping
    /// width was canonicalized away may still be cached under the width its
    /// shaping-less first frame keyed it by.
    pub(super) fn get_with_fallback(
        &mut self,
        primary: &LayoutCacheKey,
        fallback: Option<&LayoutCacheKey>,
    ) -> Option<&[PositionedGlyph]> {
        if self.touch(primary) {
            return self.peek(primary);
        }
        let fallback = fallback?;
        if self.touch(fallback) {
            return self.peek(fallback);
        }
        None
    }

    /// Returns the first resident entry among `primary` and `fallback`
    /// without stamping it.
    pub(super) fn peek_with_fallback(
        &self,
        primary: &LayoutCacheKey,
        fallback: Option<&LayoutCacheKey>,
    ) -> Option<&[PositionedGlyph]> {
        self.peek(primary)
            .or_else(|| fallback.and_then(|key| self.peek(key)))
    }

    /// Returns the source-aware interaction view for a resident layout.
    ///
    /// The renderer's glyph slice and this geometry are stored in one cache
    /// entry, so a consumer cannot accidentally query geometry from a
    /// different width or shaping result.
    pub(super) fn interaction_with_fallback(
        &self,
        primary: &LayoutCacheKey,
        fallback: Option<&LayoutCacheKey>,
    ) -> Option<&super::text_layout::TextInteractionLayout> {
        self.entries
            .get(primary)
            .or_else(|| fallback.and_then(|key| self.entries.get(key)))
            .and_then(|entry| entry.layout.interaction.as_ref())
    }

    /// How many layouts are currently resident.
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Inserts a layout, stamped as used by the current frame.
    pub(super) fn insert(&mut self, key: LayoutCacheKey, layout: impl Into<PositionedTextLayout>) {
        let generation = self.generation;
        self.entries.insert(
            key,
            CachedLayout {
                layout: layout.into(),
                last_used: generation,
            },
        );
    }

    /// Inserts every layout of `layouts`, each stamped as used by the current
    /// frame.
    pub(super) fn extend<L>(&mut self, layouts: impl IntoIterator<Item = (LayoutCacheKey, L)>)
    where
        L: Into<PositionedTextLayout>,
    {
        let generation = self.generation;
        self.entries
            .extend(layouts.into_iter().map(|(key, layout)| {
                (
                    key,
                    CachedLayout {
                        layout: layout.into(),
                        last_used: generation,
                    },
                )
            }));
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutCache;
    use crate::font::{FontFamily, FontStyle, FontWeight};
    use crate::text_pipeline::cache_key::LayoutCacheKey;

    /// A key standing in for one span laid out at `width`.
    fn key(text: &str, width: f32) -> LayoutCacheKey {
        LayoutCacheKey::new(
            text,
            16.0,
            width,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
            None,
        )
    }

    #[test]
    fn a_hit_returns_what_was_inserted() {
        let mut cache = LayoutCache::new(8);
        cache.begin_frame();

        cache.insert(key("a", 100.0), Vec::new());

        assert!(cache.get(&key("a", 100.0)).is_some());
        assert!(cache.peek(&key("a", 100.0)).is_some());
        assert!(cache.touch(&key("a", 100.0)));
        assert!(cache.get(&key("a", 200.0)).is_none());
        assert!(!cache.touch(&key("b", 100.0)));
    }

    #[test]
    fn under_capacity_nothing_is_ever_evicted() {
        let mut cache = LayoutCache::new(8);
        cache.begin_frame();
        cache.insert(key("a", 100.0), Vec::new());

        for _ in 0..100 {
            cache.begin_frame();
        }

        assert!(cache.peek(&key("a", 100.0)).is_some());
    }

    // The resize scenario: a flood of entries under widths nothing will ask
    // for again must be shed, while what the screen read last frame stays.
    #[test]
    fn over_capacity_only_the_entries_no_recent_frame_read_are_evicted() {
        let mut cache = LayoutCache::new(2);
        cache.begin_frame();
        cache.insert(key("a", 100.0), Vec::new());
        cache.insert(key("b", 100.0), Vec::new());
        cache.insert(key("c", 100.0), Vec::new());

        // The next frame reads only "a"; "b" and "c" go unread.
        cache.begin_frame();
        assert!(cache.touch(&key("a", 100.0)));

        // Over capacity: the entries no frame within the linger read go.
        cache.begin_frame();
        assert!(cache.peek(&key("a", 100.0)).is_some());
        assert!(cache.peek(&key("b", 100.0)).is_none());
        assert!(cache.peek(&key("c", 100.0)).is_none());
    }

    // Eviction must never take the previous frame's working set: those are
    // the layouts the screen is showing right now.
    #[test]
    fn the_previous_frames_working_set_survives_an_over_capacity_frame() {
        let mut cache = LayoutCache::new(2);
        cache.begin_frame();
        cache.insert(key("a", 100.0), Vec::new());
        cache.insert(key("b", 100.0), Vec::new());
        cache.insert(key("c", 100.0), Vec::new());

        cache.begin_frame();

        assert!(cache.peek(&key("a", 100.0)).is_some());
        assert!(cache.peek(&key("b", 100.0)).is_some());
        assert!(cache.peek(&key("c", 100.0)).is_some());
    }

    #[test]
    fn extended_layouts_are_stamped_like_inserts() {
        let mut cache = LayoutCache::new(2);
        cache.begin_frame();
        cache.insert(key("stale", 100.0), Vec::new());
        cache.insert(key("stale", 200.0), Vec::new());
        cache.insert(key("stale", 300.0), Vec::new());

        cache.begin_frame();
        cache.extend([(key("fresh", 100.0), Vec::new())]);

        cache.begin_frame();
        assert!(cache.peek(&key("fresh", 100.0)).is_some());
        assert!(cache.peek(&key("stale", 100.0)).is_none());
    }

    #[test]
    fn cached_aimer_layout_keeps_render_and_interaction_views_together() {
        use crate::font::{FontRegistration, FontRegistry, FontStyle, FontWeight};
        use crate::text_pipeline::glyph_rasterizer::GlyphRasterizer;
        use crate::text_pipeline::text_layout::{layout_shaped_text_result, shape_text_styled};

        // The "ffi" ligature below needs a face that has one. Register the
        // checked-in face instead of relying on the host's default font.
        let family = FontRegistry::register(FontRegistration {
            family: "aimer-layout-cache-test",
            bytes: include_bytes!("../../../../cupid_font/fonts/GoogleSans-Regular.ttf"),
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        })
        .expect("the checked-in face must register");
        let mut rasterizer = GlyphRasterizer::new();
        let shaped = shape_text_styled(
            &mut rasterizer,
            "office",
            16.0,
            family,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
        );
        let layout = layout_shaped_text_result(&shaped, 0.0, 16.0, 0.0);
        let mut cache = LayoutCache::new(8);
        let cache_key = key("office", 0.0);
        cache.begin_frame();
        cache.insert(cache_key.clone(), layout);

        assert!(!cache.peek(&cache_key).unwrap().is_empty());
        let interaction = cache
            .interaction_with_fallback(&cache_key, None)
            .expect("Aimer layout entries retain interaction geometry");
        assert!(interaction.caret_geometry(1).is_some());
        assert!(interaction.caret_geometry(2).is_none());
    }
}
