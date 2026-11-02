//! Process-wide cache of rasterized glyph metrics.
//!
//! Positioning shaped text needs the pixel box of every glyph — its bitmap
//! size and the offsets from the pen position and the baseline — but not the
//! coverage bitmap itself. Those numbers are a pure function of the
//! [`GlyphKey`], which already encodes the font, the glyph id, the size in
//! tenths of a point and the subpixel phase, so they can be computed once and
//! reused for the lifetime of the process.
//!
//! Without such a cache the only way to obtain them is to rasterize the glyph.
//! Layout runs on short-lived worker contexts whose bitmap caches start empty,
//! and re-layout happens on every frame in which the wrapping width changes —
//! exactly what a window resize does. A page of mixed-script text therefore
//! re-rasterized every visible glyph on the CPU on every resize frame.
//!
//! Entries are ~32 bytes each and are dropped wholesale once
//! [`CAPACITY`] is exceeded, mirroring the eviction strategy of the other text
//! caches: the working set is the glyphs actually on screen, so the cap is only
//! reached by long-running sessions cycling through many fonts and sizes.

use std::sync::{LazyLock, RwLock};

use hashbrown::HashMap;

use super::glyph_rasterizer::{GlyphKey, RasterizedGlyph};
use super::text_layout::FontId;

/// Size and placement of one rasterized glyph, without its bitmap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GlyphMetrics {
    pub(super) width: u32,
    pub(super) height: u32,
    /// Horizontal offset from the pen position to the left edge of the bitmap.
    pub(super) offset_x: f32,
    /// Vertical offset from the baseline to the bottom edge of the bitmap.
    pub(super) offset_y: f32,
    /// Horizontal advance width.
    pub(super) advance_width: f32,
    /// Whether the glyph rasterizes into the color atlas.
    pub(super) is_color: bool,
}

impl From<&RasterizedGlyph> for GlyphMetrics {
    fn from(glyph: &RasterizedGlyph) -> Self {
        Self {
            width: glyph.width,
            height: glyph.height,
            offset_x: glyph.offset_x,
            offset_y: glyph.offset_y,
            advance_width: glyph.advance_width,
            is_color: glyph.is_color,
        }
    }
}

/// Upper bound on retained metric entries before the cache is flushed.
const CAPACITY: usize = 16_384;

static METRICS: LazyLock<RwLock<HashMap<GlyphKey, GlyphMetrics>>> =
    LazyLock::new(|| RwLock::new(HashMap::default()));

/// Returns the metrics recorded for `key`, if the glyph was ever rasterized.
pub(super) fn cached(key: GlyphKey) -> Option<GlyphMetrics> {
    METRICS.read().ok()?.get(&key).copied()
}

/// Copies the metrics for `keys` in input order while taking one shared-cache
/// read lock for the complete batch. Missing entries remain `None`.
pub(super) fn cached_many(keys: &[GlyphKey], output: &mut Vec<Option<GlyphMetrics>>) {
    output.clear();
    let Ok(metrics) = METRICS.read() else {
        output.resize(keys.len(), None);
        return;
    };
    output.extend(keys.iter().map(|key| metrics.get(key).copied()));
}

/// Publishes the metrics of a freshly rasterized glyph for every worker and
/// every later frame to reuse.
pub(super) fn store(key: GlyphKey, glyph: &RasterizedGlyph) {
    let Ok(mut metrics) = METRICS.write() else {
        return;
    };
    if metrics.len() >= CAPACITY {
        metrics.clear();
    }
    metrics.insert(key, GlyphMetrics::from(glyph));
}

/// Drops every metric recorded for `font_id`.
///
/// Registering font bytes at runtime hands out the next free font id, which a
/// previously dropped font may already have used. Metrics are only valid for
/// the face they were measured from, so the entries of a reassigned id must go
/// the same way as the rasterizer's own caches.
pub(super) fn forget_font(font_id: FontId) {
    let Ok(mut metrics) = METRICS.write() else {
        return;
    };
    metrics.retain(|key, _| key.font_id != font_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(width: u32) -> RasterizedGlyph {
        RasterizedGlyph {
            bitmap: Vec::new(),
            width,
            height: 4,
            offset_x: 1.0,
            offset_y: -2.0,
            advance_width: 9.0,
            is_color: false,
        }
    }

    #[test]
    fn stored_metrics_are_visible_to_later_lookups() {
        let key = GlyphKey {
            font_id: 0xdead_0001,
            glyph_id: 5,
            size_tenths: 180,
            subpixel_x: 0,
            subpixel_y: 0,
            weight: 400,
            variation_id: 0,
        };

        assert_eq!(cached(key), None);
        store(key, &glyph(7));

        assert_eq!(cached(key), Some(GlyphMetrics::from(&glyph(7))));
    }

    #[test]
    fn distinct_keys_do_not_share_metrics() {
        let key = GlyphKey {
            font_id: 0xdead_0002,
            glyph_id: 5,
            size_tenths: 180,
            subpixel_x: 0,
            subpixel_y: 0,
            weight: 400,
            variation_id: 0,
        };
        let other = GlyphKey {
            size_tenths: 240,
            ..key
        };

        store(key, &glyph(3));
        store(other, &glyph(11));

        assert_eq!(cached(key).map(|metrics| metrics.width), Some(3));
        assert_eq!(cached(other).map(|metrics| metrics.width), Some(11));
    }

    #[test]
    fn forgetting_a_font_drops_only_its_own_metrics() {
        let key = GlyphKey {
            font_id: 0xdead_0003,
            glyph_id: 5,
            size_tenths: 180,
            subpixel_x: 0,
            subpixel_y: 0,
            weight: 400,
            variation_id: 0,
        };
        let other = GlyphKey {
            font_id: 0xdead_0004,
            ..key
        };

        store(key, &glyph(3));
        store(other, &glyph(11));
        forget_font(key.font_id);

        assert_eq!(cached(key), None);
        assert_eq!(cached(other).map(|metrics| metrics.width), Some(11));
    }
}
