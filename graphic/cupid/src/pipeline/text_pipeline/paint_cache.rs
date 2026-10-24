use std::sync::Arc;

use hashbrown::HashMap;

use super::cache_key::{LayoutCacheKey, SpanLayoutKeys};
use super::glyph_atlas::AtlasRegion;
use super::text_layout::{TextHorizontalAlign, TextWritingMode};
use super::{Rgba8, TextDrawRequest, TextOverflowMode, TextShadowRequest};

const MAX_ENTRIES: usize = 1_024;
const MAX_GLYPH_RECORDS: usize = 262_144;

/// The normalized shadow identity used by a reusable text-paint template.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) struct PaintShadowKey {
    pub offset_x: u32,
    pub offset_y: u32,
    pub blur: u32,
    pub color: Rgba8,
}

impl PaintShadowKey {
    #[inline]
    fn new(shadow: Option<TextShadowRequest>) -> Option<Self> {
        let shadow = sanitize_shadow(shadow)?;
        Some(Self {
            offset_x: shadow.offset_x.to_bits(),
            offset_y: shadow.offset_y.to_bits(),
            blur: shadow.blur.to_bits(),
            color: shadow.color,
        })
    }
}

/// A key for one span's origin-relative glyph paint.
///
/// Position, clip, foreground color, and `draw_glyphs` are deliberately
/// absent: those values are patched while the cached geometry is assembled
/// into the current frame.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct TextPaintCacheKey {
    primary: LayoutCacheKey,
    fallback: Option<LayoutCacheKey>,
    overflow: TextOverflowMode,
    horizontal_align: TextHorizontalAlign,
    writing_mode: TextWritingMode,
    bounds_width: u32,
    bounds_height: u32,
    line_height: Option<u32>,
    font_weight: u16,
    italic: bool,
    shadow: Option<PaintShadowKey>,
}

impl TextPaintCacheKey {
    #[inline]
    pub(super) fn new(
        keys: &SpanLayoutKeys,
        request: &TextDrawRequest,
        font_weight: u16,
        italic: bool,
    ) -> Self {
        // Left-aligned horizontal clipping does not use the request bounds for
        // paint: the layout key already captures wrapping/ellipsis extents,
        // while the renderer may synthesize a changing remaining-surface
        // width for an unbounded DrawText command as it scrolls. Keep that
        // incidental width out of the paint identity. Center/right alignment
        // still needs it because the line offset is patched into the cached
        // glyph positions.
        let bounds_width = if request.writing_mode.is_vertical()
            || request.horizontal_align != TextHorizontalAlign::Left
        {
            request.bounds_width.to_bits()
        } else {
            0
        };
        let bounds_height = if request.writing_mode.is_vertical() {
            request.bounds_height.to_bits()
        } else {
            0
        };
        Self {
            primary: keys.primary.clone(),
            fallback: keys.fallback.clone(),
            overflow: request.overflow,
            horizontal_align: request.horizontal_align,
            writing_mode: request.writing_mode,
            bounds_width,
            bounds_height,
            line_height: request.line_height.map(f32::to_bits),
            font_weight,
            italic,
            shadow: PaintShadowKey::new(request.shadow),
        }
    }
}

/// Whether a cached paint record is a shadow sample or a foreground copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CachedPaintKind {
    Shadow,
    Foreground,
}

/// One origin-relative glyph quad before the request's current translation is
/// applied.
#[derive(Clone, Copy, Debug)]
pub(super) struct CachedPaintGlyph {
    /// The unsnapped origin-relative glyph position before the current request
    /// offset is applied.
    pub local_position: [f32; 2],
    /// Shadow-sample or synthetic-weight offset applied after snapping.
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub region: AtlasRegion,
    pub color_atlas: bool,
    pub skew: f32,
    pub kind: CachedPaintKind,
}

/// Immutable paint geometry for one span.
#[derive(Debug)]
pub(super) struct CachedTextPaint {
    pub glyphs: Vec<CachedPaintGlyph>,
    pub advance_x: f32,
    pub advance_y: f32,
    pub shadow: Option<TextShadowRequest>,
    pub shadow_coverage_exponent: f32,
}

impl CachedTextPaint {
    #[inline]
    pub(super) fn glyph_record_count(&self) -> usize {
        self.glyphs.len()
    }
}

struct PaintEntry {
    paint: Arc<CachedTextPaint>,
    used: u64,
}

/// Bounded LRU storage for origin-relative text paint.
pub(super) struct TextPaintCache {
    entries: HashMap<TextPaintCacheKey, PaintEntry>,
    clock: u64,
    glyph_records: usize,
    atlas_generation: u64,
    color_atlas_generation: u64,
    font_revision: u64,
}

impl Default for TextPaintCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            glyph_records: 0,
            atlas_generation: 0,
            color_atlas_generation: 0,
            font_revision: 0,
        }
    }
}

impl TextPaintCache {
    /// Clears templates derived from an atlas or font state that changed.
    pub(super) fn ensure_generations(
        &mut self,
        atlas_generation: u64,
        color_atlas_generation: u64,
        font_revision: u64,
    ) {
        if self.atlas_generation != atlas_generation
            || self.color_atlas_generation != color_atlas_generation
            || self.font_revision != font_revision
        {
            self.clear();
            self.atlas_generation = atlas_generation;
            self.color_atlas_generation = color_atlas_generation;
            self.font_revision = font_revision;
        }
    }

    #[inline]
    pub(super) fn get(&mut self, key: &TextPaintCacheKey) -> Option<Arc<CachedTextPaint>> {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.used = self.clock;
        Some(entry.paint.clone())
    }

    pub(super) fn insert(&mut self, key: TextPaintCacheKey, paint: Arc<CachedTextPaint>) {
        let cost = paint.glyph_record_count();
        if cost > MAX_GLYPH_RECORDS {
            return;
        }
        self.clock = self.clock.wrapping_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.glyph_records = self
                .glyph_records
                .saturating_sub(previous.paint.glyph_record_count());
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_ENTRIES
                || self.glyph_records.saturating_add(cost) > MAX_GLYPH_RECORDS)
        {
            self.evict_cold_quarter();
        }
        self.glyph_records += cost;
        self.entries.insert(
            key,
            PaintEntry {
                paint,
                used: self.clock,
            },
        );
    }

    #[cfg(test)]
    #[inline]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.glyph_records = 0;
    }

    fn evict_cold_quarter(&mut self) {
        let target = (self.entries.len() / 4).max(1);
        let mut cold = self
            .entries
            .iter()
            .map(|(key, entry)| (entry.used, key.clone()))
            .collect::<Vec<_>>();
        cold.sort_unstable_by_key(|(used, _)| *used);
        for (_, key) in cold.into_iter().take(target) {
            if let Some(entry) = self.entries.remove(&key) {
                self.glyph_records = self
                    .glyph_records
                    .saturating_sub(entry.paint.glyph_record_count());
            }
        }
    }
}

/// Applies the same finite-value policy as the text assembly path.
#[inline]
pub(super) fn sanitize_shadow(shadow: Option<TextShadowRequest>) -> Option<TextShadowRequest> {
    let shadow = shadow?;
    if shadow.color.0[3] == 0 {
        return None;
    }
    Some(TextShadowRequest {
        offset_x: shadow
            .offset_x
            .is_finite()
            .then_some(shadow.offset_x)
            .unwrap_or(0.0),
        offset_y: shadow
            .offset_y
            .is_finite()
            .then_some(shadow.offset_y)
            .unwrap_or(0.0),
        blur: shadow
            .blur
            .is_finite()
            .then_some(shadow.blur.max(0.0))
            .unwrap_or(0.0),
        color: shadow.color,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hashbrown::HashMap;

    use crate::font::{FontFamily, FontStyle, FontWeight};
    use crate::pipeline::text_pipeline::cache_key::span_layout_keys_with_writing_mode;
    use crate::pipeline::text_pipeline::text_layout::{TextHorizontalAlign, TextWritingMode};
    use crate::utilities::Rgba8;

    use super::{CachedTextPaint, TextPaintCache, TextPaintCacheKey};
    use crate::pipeline::text_pipeline::{TextDrawRequest, TextOverflowMode};

    fn request(x: f32, color: Rgba8, bounds_width: f32) -> TextDrawRequest {
        TextDrawRequest {
            x,
            y: 12.0,
            text: Arc::from("cache"),
            font_size: 16.0,
            color,
            bounds_width,
            bounds_height: 24.0,
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: [0.0, 0.0, 320.0, 40.0],
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        }
    }

    fn key(request: &TextDrawRequest) -> TextPaintCacheKey {
        let shaping_cache = HashMap::new();
        let layout_width = if request.writing_mode.is_vertical() {
            request.bounds_width
        } else {
            match request.overflow {
                TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => request.bounds_width,
                TextOverflowMode::Clip => 0.0,
            }
        };
        let layout_height = if request.writing_mode.is_vertical() {
            match request.overflow {
                TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => request.bounds_height,
                TextOverflowMode::Clip => 0.0,
            }
        } else {
            0.0
        };
        let span_keys = span_layout_keys_with_writing_mode(
            &shaping_cache,
            &request.text,
            request.font_size,
            request.font_family,
            request.font_style,
            FontWeight::Normal.numeric(),
            request.language,
            layout_width,
            layout_height,
            request.writing_mode,
        );
        TextPaintCacheKey::new(&span_keys, request, FontWeight::Normal.numeric(), false)
    }

    fn empty_paint() -> Arc<CachedTextPaint> {
        Arc::new(CachedTextPaint {
            glyphs: Vec::new(),
            advance_x: 0.0,
            advance_y: 0.0,
            shadow: None,
            shadow_coverage_exponent: 1.0,
        })
    }

    #[test]
    fn generation_change_clears_templates() {
        let mut cache = TextPaintCache::default();
        cache.ensure_generations(1, 1, 1);
        let key = key(&request(0.0, Rgba8::new(0, 0, 0, 255), 240.0));
        cache.insert(key.clone(), empty_paint());
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&key).is_some());

        cache.ensure_generations(1, 1, 2);

        assert_eq!(cache.len(), 0);
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn position_color_clip_and_draw_mode_do_not_invalidate_paint_template() {
        let first = request(0.0, Rgba8::new(0, 0, 0, 255), 240.0);
        let mut changed = request(17.5, Rgba8::new(255, 0, 0, 255), 240.0);
        changed.bounds_width = 200.0;
        changed.clip_rect = [10.0, 4.0, 100.0, 20.0];
        changed.draw_glyphs = false;

        assert_eq!(key(&first), key(&changed));
    }

    #[test]
    fn layout_extent_changes_paint_template_identity() {
        let mut first = request(0.0, Rgba8::new(0, 0, 0, 255), 240.0);
        first.horizontal_align = TextHorizontalAlign::Center;
        let mut changed = request(0.0, Rgba8::new(0, 0, 0, 255), 200.0);
        changed.horizontal_align = TextHorizontalAlign::Center;

        assert_ne!(key(&first), key(&changed));
    }
}
