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
//! scratch. [`LayoutCache`] evicts by recency instead, through
//! [`RecencyCache`]: every hit and insert stamps the entry with the current
//! frame's generation, and when a frame begins over its entry capacity or its
//! byte budget, only the entries no recent frame read are dropped — the
//! working set of the previous frame always survives, so a flood of
//! transient-width entries is shed without ever taking the live screen's
//! layouts with it.

use std::cell::OnceCell;

use super::cache_key::LayoutCacheKey;
use super::recency_cache::RecencyCache;
use super::text_layout::{PositionedGlyph, PositionedTextLayout};
use crate::glyph_rasterizer::GlyphKey;
use hashbrown::HashSet;

/// A cached layout and what is worked out from it once.
///
/// A line of text is a few dozen distinct glyphs drawn some hundreds of times. Whether a glyph
/// is prepared, in the atlas, and how big it is are questions about the glyph and not about
/// where it is drawn, so a frame that asks them of every glyph of every layout it draws asks the
/// same few dozen questions over and over. [`distinct_glyphs`](Self::distinct_glyphs) is that
/// few dozen, found by one pass over the layout the first time they are wanted and kept as long
/// as the layout is.
struct CachedLayout {
    layout: PositionedTextLayout,
    distinct: OnceCell<Box<[(GlyphKey, f32)]>>,
}

impl CachedLayout {
    /// Each glyph key of the layout once, in the order it is first drawn, with the font size
    /// of that first glyph.
    fn distinct_glyphs(&self) -> &[(GlyphKey, f32)] {
        self.distinct.get_or_init(|| {
            let mut seen = HashSet::new();
            self.layout
                .glyphs
                .iter()
                .filter(|glyph| seen.insert(glyph.glyph_key))
                .map(|glyph| (glyph.glyph_key, glyph.font_size))
                .collect()
        })
    }
}

/// A recency-evicting map from [`LayoutCacheKey`] to positioned glyphs.
///
/// The cache never sheds an entry mid-frame: [`begin_frame`] is the only
/// place eviction happens, so a key that [`get`] or [`touch`] confirmed is
/// guaranteed to stay resident until the frame ends, which is what lets
/// `prepare` collect layouts early and read them again when it builds
/// instances.
///
/// The entry capacity and the byte budget are soft bounds — a single frame
/// that genuinely uses more than either keeps all of it, since evicting the
/// working set would only force the next frame to rebuild it.
///
/// [`begin_frame`]: LayoutCache::begin_frame
/// [`get`]: LayoutCache::get
/// [`touch`]: LayoutCache::touch
pub(super) struct LayoutCache {
    cache: RecencyCache<LayoutCacheKey, CachedLayout>,
}

impl LayoutCache {
    /// Creates an empty cache bounded only by entry count.
    #[cfg(test)]
    pub(super) fn new(capacity: usize) -> Self {
        Self::with_byte_budget(capacity, usize::MAX)
    }

    /// Creates an empty cache that starts evicting stale entries once more
    /// than `capacity` entries or `byte_budget` bytes are resident at the
    /// start of a frame.
    pub(super) fn with_byte_budget(capacity: usize, byte_budget: usize) -> Self {
        Self {
            cache: RecencyCache::new(capacity, byte_budget),
        }
    }

    /// Opens a frame: when the cache has grown past its capacity or budget,
    /// drops every entry no recent frame has read.
    ///
    /// Called once per `prepare`, before any lookups, so eviction can never
    /// invalidate a layout the current frame already confirmed.
    pub(super) fn begin_frame(&mut self) {
        self.cache.begin_frame();
    }

    /// Returns the cached glyphs for `key`, stamping the entry as used by the
    /// current frame.
    pub(super) fn get(&mut self, key: &LayoutCacheKey) -> Option<&[PositionedGlyph]> {
        self.cache.get(key).map(|entry| entry.layout.glyphs.as_slice())
    }

    /// Returns the cached glyphs for `key` without stamping it.
    ///
    /// For re-reads within a frame that already confirmed the key with
    /// [`get`](Self::get) or [`touch`](Self::touch) — the stamp is in place,
    /// so the second read can skip the write.
    pub(super) fn peek(&self, key: &LayoutCacheKey) -> Option<&[PositionedGlyph]> {
        self.cache.peek(key).map(|entry| entry.layout.glyphs.as_slice())
    }

    /// Whether `key` is resident, stamping it as used by the current frame
    /// when it is.
    pub(super) fn touch(&mut self, key: &LayoutCacheKey) -> bool {
        self.cache.touch(key)
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

    /// The distinct glyphs of the first resident entry among `primary` and `fallback`, stamping
    /// it as used by the current frame; see [`CachedLayout::distinct_glyphs`].
    pub(super) fn distinct_glyphs_with_fallback(
        &mut self,
        primary: &LayoutCacheKey,
        fallback: Option<&LayoutCacheKey>,
    ) -> Option<&[(GlyphKey, f32)]> {
        let key = if self.touch(primary) { primary } else { fallback.filter(|key| self.touch(key))? };
        self.cache.peek(key).map(CachedLayout::distinct_glyphs)
    }

    /// [`distinct_glyphs_with_fallback`](Self::distinct_glyphs_with_fallback) without stamping,
    /// for a layout the frame has already confirmed.
    pub(super) fn peek_distinct_glyphs_with_fallback(
        &self,
        primary: &LayoutCacheKey,
        fallback: Option<&LayoutCacheKey>,
    ) -> Option<&[(GlyphKey, f32)]> {
        self.cache
            .peek(primary)
            .or_else(|| fallback.and_then(|key| self.cache.peek(key)))
            .map(CachedLayout::distinct_glyphs)
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
        self.cache
            .peek(primary)
            .or_else(|| fallback.and_then(|key| self.cache.peek(key)))
            .and_then(|entry| entry.layout.interaction.as_ref())
    }

    /// Estimated bytes of every resident layout.
    pub(super) fn bytes(&self) -> usize {
        self.cache.bytes()
    }

    /// How many layouts are currently resident.
    pub(super) fn len(&self) -> usize {
        self.cache.len()
    }

    /// Inserts a layout, stamped as used by the current frame.
    pub(super) fn insert(&mut self, key: LayoutCacheKey, layout: impl Into<PositionedTextLayout>) {
        let layout = layout.into();
        let bytes = layout.heap_bytes();
        self.cache.insert(key, CachedLayout { layout, distinct: OnceCell::new() }, bytes);
    }

    /// Inserts every layout of `layouts`, each stamped as used by the current
    /// frame.
    pub(super) fn extend<L>(&mut self, layouts: impl IntoIterator<Item = (LayoutCacheKey, L)>)
    where
        L: Into<PositionedTextLayout>,
    {
        for (key, layout) in layouts {
            self.insert(key, layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutCache;
    use crate::glyph_rasterizer::GlyphKey;
    use crate::font::{FontFamily, FontStyle, FontWeight};
    use crate::cache_key::LayoutCacheKey;

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

    fn glyph(glyph_id: u16, font_size: f32) -> crate::text_layout::PositionedGlyph {
        crate::text_layout::PositionedGlyph {
            codepoint: 'a',
            glyph_key: GlyphKey::new(1, glyph_id, font_size),
            line_index: 0,
            line_x: 0.0,
            advance: 8.0,
            x: 0.0,
            y: 0.0,
            width: 8,
            height: 12,
            font_size,
        }
    }

    #[test]
    fn a_layout_lists_each_of_its_glyphs_once_in_the_order_they_are_first_drawn() {
        let mut cache = LayoutCache::new(8);
        cache.begin_frame();
        // "abracadabra" and a bigger "a": a repeated glyph, and a glyph of the same face at
        // another size, which is another key.
        let glyphs: Vec<_> = [1, 2, 3, 1, 4, 1, 5, 1, 2, 3, 1].iter().map(|&id| glyph(id, 16.0)).chain([glyph(1, 24.0)]).collect();
        cache.insert(key("abracadabra", 100.0), glyphs);

        let distinct = cache.distinct_glyphs_with_fallback(&key("abracadabra", 100.0), None).unwrap();

        let ids: Vec<(u16, u32)> = distinct.iter().map(|(key, _)| (key.glyph_id, key.size_tenths)).collect();
        assert_eq!(ids, [(1, 160), (2, 160), (3, 160), (4, 160), (5, 160), (1, 240)]);
        assert_eq!(distinct[5].1, 24.0, "with the font size it was drawn at");
    }

    #[test]
    fn the_distinct_glyphs_follow_the_fallback_key_and_a_miss_is_none() {
        let mut cache = LayoutCache::new(8);
        cache.begin_frame();
        cache.insert(key("a", 100.0), vec![glyph(7, 16.0), glyph(7, 16.0)]);

        assert_eq!(cache.distinct_glyphs_with_fallback(&key("a", 999.0), Some(&key("a", 100.0))).unwrap().len(), 1);
        assert!(cache.distinct_glyphs_with_fallback(&key("a", 999.0), None).is_none());
        assert!(cache.distinct_glyphs_with_fallback(&key("a", 999.0), Some(&key("b", 1.0))).is_none());
        assert_eq!(cache.peek_distinct_glyphs_with_fallback(&key("a", 100.0), None).unwrap().len(), 1);
    }

    #[test]
    fn reading_the_distinct_glyphs_stamps_the_layout_as_used() {
        let mut cache = LayoutCache::new(1);
        cache.begin_frame();
        cache.insert(key("kept", 100.0), vec![glyph(1, 16.0)]);
        cache.insert(key("dropped", 100.0), vec![glyph(2, 16.0)]);
        cache.begin_frame();
        assert!(cache.distinct_glyphs_with_fallback(&key("kept", 100.0), None).is_some());

        cache.begin_frame();

        assert!(cache.peek(&key("kept", 100.0)).is_some());
        assert!(cache.peek(&key("dropped", 100.0)).is_none());
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
        use crate::glyph_rasterizer::GlyphRasterizer;
        use crate::text_layout::{layout_shaped_text_result, shape_text_styled};

        // The "ffi" ligature below needs a face that has one. Register the
        // checked-in face instead of relying on the host's default font.
        let family = FontRegistry::register(FontRegistration {
            family: "aimer-layout-cache-test",
            bytes: include_bytes!("../../../vectors/font/fonts/GoogleSans-Regular.ttf"),
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
