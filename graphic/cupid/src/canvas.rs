use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use crate::draw_cmd::{DrawList, TextureRegistry};
use crate::font::{FontFamily, FontStyle, FontWeight, TextLanguage};
use crate::frame::RetainedRenderPlan;
use crate::lru_map::LruMap;
use crate::text_pipeline::glyph_rasterizer::{GlyphRasterizer, RunAdvances};
use crate::text_pipeline::text_layout::line_break_opportunities;
use crate::utilities::{Mat3, TextureId};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
    pub line_height: f32,
    pub line_count: usize,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct TextMetricsKey {
    text: String,
    font_size_tenths: u32,
    max_width_tenths: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    /// Measured widths depend on the face the run resolves to, which for
    /// ideographs the language decides — see
    /// [`CupidCanvas::set_text_language`].
    language: Option<TextLanguage>,
    font_revision: u64,
}

#[derive(Hash)]
struct TextMetricsKeyRef<'a> {
    text: &'a str,
    font_size_tenths: u32,
    max_width_tenths: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    language: Option<TextLanguage>,
    font_revision: u64,
}

impl TextMetricsKey {
    #[inline]
    fn matches(&self, lookup: &TextMetricsKeyRef<'_>) -> bool {
        self.text == lookup.text
            && self.font_size_tenths == lookup.font_size_tenths
            && self.max_width_tenths == lookup.max_width_tenths
            && self.font_family == lookup.font_family
            && self.font_style == lookup.font_style
            && self.font_weight == lookup.font_weight
            && self.language == lookup.language
            && self.font_revision == lookup.font_revision
    }
}

#[derive(Clone, Debug)]
struct CachedTextMetrics {
    metrics: TextMetrics,
    line_widths: Vec<f32>,
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct TextShapeKey {
    text: String,
    font_size_bits: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    language: Option<TextLanguage>,
    font_revision: u64,
}

#[derive(Hash)]
struct TextShapeKeyRef<'a> {
    text: &'a str,
    font_size_bits: u32,
    font_family: FontFamily,
    font_style: FontStyle,
    font_weight: u16,
    language: Option<TextLanguage>,
    font_revision: u64,
}

impl TextShapeKey {
    #[inline]
    fn matches(&self, lookup: &TextShapeKeyRef<'_>) -> bool {
        self.text == lookup.text
            && self.font_size_bits == lookup.font_size_bits
            && self.font_family == lookup.font_family
            && self.font_style == lookup.font_style
            && self.font_weight == lookup.font_weight
            && self.language == lookup.language
            && self.font_revision == lookup.font_revision
    }
}

/// How many measured strings the canvas remembers.
///
/// A scrolled list of distinct rows measures a viewport's worth of new strings
/// per frame, so the cache is sized to hold many screens of them and evicts the
/// coldest quarter when it fills. It must never be emptied outright: that would
/// drop the strings the current frame is built from and turn every newly visible
/// row into a permanent miss.
const METRICS_CACHE_CAPACITY: usize = 4096;
/// Whole-run shaping is much larger than a width measurement, but a few hundred
/// paragraphs are a normal document-sized working set. Keeping these shaped
/// runs separate from width-dependent layout lets a resize re-use glyph
/// advances without retaining every possible wrapping width.
const SHAPING_CACHE_CAPACITY: usize = 512;
/// Unwrapped interaction geometry is independent of the paragraph's current
/// wrapping width. Retaining it separately avoids rebuilding the same source
/// clusters when a rich paragraph is re-laid out at a new width.
const UNWRAPPED_LAYOUT_CACHE_CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct CupidCanvas {
    draw_list: Rc<RefCell<DrawList>>,
    retained_render_plan: Rc<RefCell<Option<RetainedRenderPlan>>>,
    texture_registry: Arc<TextureRegistry>,
    rasterizer: Rc<RefCell<GlyphRasterizer>>,
    metrics_cache: Rc<RefCell<LruMap<TextMetricsKey, CachedTextMetrics>>>,
    shaping_cache: Rc<RefCell<LruMap<TextShapeKey, Rc<crate::text_layout::ShapedText>>>>,
    unwrapped_layout_cache:
        Rc<RefCell<LruMap<TextShapeKey, Rc<crate::text_layout::TextInteractionLayout>>>>,
    /// The language subsequent text is written in — see
    /// [`CupidCanvas::set_text_language`].
    ///
    /// Drawing records the state into the draw list, but measuring answers
    /// immediately and never reaches the renderer, so the canvas keeps the
    /// current value here as well: a field that paints its text in a Chinese
    /// face must not place its caret with a Japanese face's advances.
    text_language: Rc<Cell<Option<TextLanguage>>>,
    /// Debug-only counters for the text metrics cache. They are per-frame so
    /// scroll profiling can distinguish cache misses from command recording.
    #[cfg(debug_assertions)]
    metrics_cache_hits: Rc<Cell<u64>>,
    #[cfg(debug_assertions)]
    metrics_cache_misses: Rc<Cell<u64>>,
}

impl CupidCanvas {
    pub fn new() -> Self {
        let texture_registry = Arc::new(TextureRegistry::default());
        Self {
            draw_list: Rc::new(RefCell::new(DrawList::with_texture_registry(
                texture_registry.clone(),
            ))),
            retained_render_plan: Rc::new(RefCell::new(None)),
            texture_registry,
            rasterizer: Rc::new(RefCell::new(GlyphRasterizer::new())),
            metrics_cache: Rc::new(RefCell::new(LruMap::new(METRICS_CACHE_CAPACITY))),
            shaping_cache: Rc::new(RefCell::new(LruMap::new(SHAPING_CACHE_CAPACITY))),
            unwrapped_layout_cache: Rc::new(RefCell::new(LruMap::new(
                UNWRAPPED_LAYOUT_CACHE_CAPACITY,
            ))),
            text_language: Rc::new(Cell::new(None)),
            #[cfg(debug_assertions)]
            metrics_cache_hits: Rc::new(Cell::new(0)),
            #[cfg(debug_assertions)]
            metrics_cache_misses: Rc::new(Cell::new(0)),
        }
    }

    /// Creates a short-lived canvas that shares the parent's text, font,
    /// metrics, and texture state but starts with an identity transform and an
    /// empty draw list. Measurement code uses it so its transform and language
    /// changes cannot leak into the frame's canvas.
    #[inline]
    pub fn fork_for_recording(&self) -> Self {
        Self {
            draw_list: Rc::new(RefCell::new(DrawList::with_texture_registry(
                self.texture_registry.clone(),
            ))),
            retained_render_plan: Rc::new(RefCell::new(None)),
            texture_registry: self.texture_registry.clone(),
            rasterizer: self.rasterizer.clone(),
            metrics_cache: self.metrics_cache.clone(),
            shaping_cache: self.shaping_cache.clone(),
            unwrapped_layout_cache: self.unwrapped_layout_cache.clone(),
            text_language: Rc::new(Cell::new(self.text_language.get())),
            #[cfg(debug_assertions)]
            metrics_cache_hits: self.metrics_cache_hits.clone(),
            #[cfg(debug_assertions)]
            metrics_cache_misses: self.metrics_cache_misses.clone(),
        }
    }

    pub fn begin_frame(&self) {
        self.draw_list.borrow_mut().clear();
        self.retained_render_plan.borrow_mut().take();
        #[cfg(debug_assertions)]
        {
            self.metrics_cache_hits.set(0);
            self.metrics_cache_misses.set(0);
        }
    }

    /// Moves the frame recorded so far out of the canvas.
    ///
    /// The returned list owns its command buffer, so it can be handed to
    /// another thread for encoding while the canvas keeps serving the next
    /// frame. Ownership should come back through [`recycle_draw_list`] before
    /// the next [`begin_frame`] so command and local metadata allocations are
    /// reused; the shared texture registry preserves image sizes while a frame
    /// is in flight.
    ///
    /// [`recycle_draw_list`]: CupidCanvas::recycle_draw_list
    /// [`begin_frame`]: CupidCanvas::begin_frame
    #[inline]
    pub fn take_draw_list(&self) -> DrawList {
        let mut current = self.draw_list.borrow_mut();
        let mut next = DrawList::with_texture_registry(self.texture_registry.clone());
        std::mem::swap(&mut *current, &mut next);
        next
    }

    /// Gives a list taken by [`take_draw_list`] back to the canvas so its
    /// buffers are reused instead of reallocated every frame.
    ///
    /// [`take_draw_list`]: CupidCanvas::take_draw_list
    #[inline]
    pub fn recycle_draw_list(&self, draw_list: DrawList) {
        *self.draw_list.borrow_mut() = draw_list;
    }

    /// Stores the retained render plan for the frame being assembled.
    #[doc(hidden)]
    pub fn set_retained_render_plan(&self, plan: Option<RetainedRenderPlan>) {
        *self.retained_render_plan.borrow_mut() = plan;
    }

    /// Takes the retained render plan for the frame being assembled.
    #[doc(hidden)]
    pub fn take_retained_render_plan(&self) -> Option<RetainedRenderPlan> {
        self.retained_render_plan.borrow_mut().take()
    }

    pub fn register_font_bytes(&self, bytes: Vec<u8>) -> Option<crate::text_layout::FontId> {
        let font_id = self.rasterizer.borrow_mut().register_font_bytes(bytes)?;
        self.metrics_cache.borrow_mut().clear();
        self.shaping_cache.borrow_mut().clear();
        self.unwrapped_layout_cache.borrow_mut().clear();
        Some(font_id)
    }

    pub fn translate(&self, x: f32, y: f32) {
        self.draw_list.borrow_mut().translate(x, y);
    }

    pub fn scale(&self, sx: f32, sy: f32) {
        self.draw_list.borrow_mut().scale(sx, sy);
    }

    pub fn rotate(&self, radians: f32) {
        self.draw_list.borrow_mut().rotate(radians);
    }

    pub fn save(&self) {
        self.draw_list.borrow_mut().save();
    }

    pub fn restore(&self) {
        self.draw_list.borrow_mut().restore();
    }

    /// Measure text width using the cached text rasterizer.
    pub fn measure_text(&self, text: &str, font_size: f32) -> f32 {
        self.measure_text_styled(
            text,
            font_size,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        )
    }

    pub fn measure_text_styled(
        &self,
        text: &str,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> f32 {
        // Paragraph layout asks for the same unwrapped run width repeatedly
        // while it partitions spans into lines. Route that query through the
        // canvas metrics cache so a rebuild reuses the whole-run result rather
        // than walking every character again. A hard line break is kept on the
        // rasterizer's historical path because the old width API summed runs,
        // whereas cached metrics report the widest line.
        if !text.contains('\n') {
            return self
                .measure_text_metrics_styled(
                    text,
                    font_size,
                    0.0,
                    font_family,
                    font_style,
                    font_weight,
                )
                .width;
        }

        self.rasterizer.borrow_mut().measure_text_for_family(
            text,
            font_size,
            font_family,
            FontWeight::Value(u32::from(font_weight)),
            font_style,
            self.text_language(),
        )
    }

    /// Shapes and lays out a source-aware Aimer paragraph for interaction.
    ///
    /// This CPU-side view is intended for caret, hit-test, and selection
    /// consumers that record text through [`CupidCanvas`]. The renderer keeps
    /// the same result in [`TextPipelineV2`](crate::text_pipeline::TextPipelineV2)
    /// when it prepares the draw list.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> crate::text_layout::TextInteractionLayout {
        (*self.layout_text_styled_shared(
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        ))
        .clone()
    }

    /// Returns shared source-aware geometry for a styled text run.
    ///
    /// Unwrapped runs (`max_width <= 0`) are cached because their clusters and
    /// advances do not depend on the containing paragraph's wrapping width.
    /// Wrapped results remain caller-owned: each width produces different line
    /// positions and retaining every resize width would turn a cache into a
    /// growing history of the window drag.
    #[allow(clippy::too_many_arguments)]
    pub fn layout_text_styled_shared(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Rc<crate::text_layout::TextInteractionLayout> {
        let language = self.text_language();
        let font_revision = crate::font::FontRegistry::revision();
        let lookup = TextShapeKeyRef {
            text,
            font_size_bits: font_size.to_bits(),
            font_family,
            font_style,
            font_weight,
            language,
            font_revision,
        };
        let is_unwrapped = max_width <= 0.0;
        if is_unwrapped
            && let Some(cached) = self
                .unwrapped_layout_cache
                .borrow_mut()
                .get_by(&lookup, TextShapeKey::matches)
        {
            return Rc::clone(cached);
        }

        let shaped = self.shape_text_styled_cached(
            text,
            font_size,
            font_family,
            font_style,
            font_weight,
            language,
        );
        let layout = Rc::new(crate::text_layout::layout_shaped_text_with_interaction(
            &shaped,
            0.0,
            shaped.ascent + shaped.line_gap * 0.5,
            max_width,
        ));
        if is_unwrapped {
            let key = TextShapeKey {
                text: text.to_owned(),
                font_size_bits: lookup.font_size_bits,
                font_family: lookup.font_family,
                font_style: lookup.font_style,
                font_weight: lookup.font_weight,
                language: lookup.language,
                font_revision: lookup.font_revision,
            };
            self.unwrapped_layout_cache
                .borrow_mut()
                .insert(key, Rc::clone(&layout));
        }
        layout
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_text_styled_cached(
        &self,
        text: &str,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
        language: Option<TextLanguage>,
    ) -> Rc<crate::text_layout::ShapedText> {
        let lookup = TextShapeKeyRef {
            text,
            font_size_bits: font_size.to_bits(),
            font_family,
            font_style,
            font_weight,
            language,
            font_revision: crate::font::FontRegistry::revision(),
        };
        if let Some(cached) = self
            .shaping_cache
            .borrow_mut()
            .get_by(&lookup, TextShapeKey::matches)
        {
            return Rc::clone(cached);
        }

        let shaped = {
            let mut rasterizer = self.rasterizer.borrow_mut();
            crate::text_layout::shape_text_styled(
                &mut rasterizer,
                text,
                font_size,
                font_family,
                FontWeight::Value(u32::from(font_weight)),
                font_style,
                language,
            )
        };
        let shaped = Rc::new(shaped);
        let key = TextShapeKey {
            text: text.to_owned(),
            font_size_bits: lookup.font_size_bits,
            font_family: lookup.font_family,
            font_style: lookup.font_style,
            font_weight: lookup.font_weight,
            language: lookup.language,
            font_revision: lookup.font_revision,
        };
        self.shaping_cache
            .borrow_mut()
            .insert(key, Rc::clone(&shaped));
        shaped
    }

    pub fn measure_text_metrics(&self, text: &str, font_size: f32, max_width: f32) -> TextMetrics {
        self.measure_text_metrics_styled(
            text,
            font_size,
            max_width,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        )
    }

    pub fn measure_text_metrics_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> TextMetrics {
        let language = self.text_language();
        let font_size_tenths = (font_size * 10.0) as u32;
        let max_width_tenths = (max_width.max(0.0) * 10.0) as u32;
        let font_revision = crate::font::FontRegistry::revision();
        let lookup = TextMetricsKeyRef {
            text,
            font_size_tenths,
            max_width_tenths,
            font_family,
            font_style,
            font_weight,
            language,
            font_revision,
        };
        if let Some(cached) = self
            .metrics_cache
            .borrow_mut()
            .get_by(&lookup, TextMetricsKey::matches)
        {
            #[cfg(debug_assertions)]
            self.metrics_cache_hits
                .set(self.metrics_cache_hits.get().saturating_add(1));
            return cached.metrics;
        }

        #[cfg(debug_assertions)]
        self.metrics_cache_misses
            .set(self.metrics_cache_misses.get().saturating_add(1));

        let mut rasterizer = self.rasterizer.borrow_mut();
        // Measuring must choose the same faces the shaping pass will, or the
        // line that gets painted can wrap at a different boundary.
        rasterizer.begin_script_run(text, language);
        let weight = FontWeight::Value(u32::from(font_weight));
        let (ascent, descent, line_gap) =
            rasterizer.line_metrics_for_family(font_size, font_family, weight, font_style);
        let line_height = ascent - descent + line_gap;
        let mut width = 0.0_f32;
        let mut current_width = 0.0_f32;
        let mut line_count = 1_usize;
        let mut line_widths = Vec::new();
        // Width of the current line up to its last UAX #14 break opportunity.
        // `None` means no break opportunity is available on the current line
        // yet. This mirrors the word-wrapping performed by `layout_shaped_text`
        // so the measured line count matches the rendered one (otherwise the
        // last line would be clipped).
        let mut last_break_end: Option<f32> = None;
        let can_break_before = line_break_opportunities(text);
        // One run, one style: each distinct ASCII character is resolved once.
        let mut advances = RunAdvances::new();

        for (offset, c) in text.char_indices() {
            if c == '\n' {
                width = width.max(current_width);
                line_widths.push(current_width);
                current_width = 0.0;
                line_count += 1;
                last_break_end = None;
                continue;
            }

            let glyph_width =
                advances.advance(&mut rasterizer, c, font_size, font_family, weight, font_style);

            // Track where this line may be broken, measured before the
            // character is added so a trailing space stays on its own line.
            if can_break_before[offset] {
                last_break_end = Some(current_width);
            }

            if max_width > 0.0 && current_width > 0.0 && current_width + glyph_width > max_width {
                if let Some(break_end) = last_break_end {
                    // Word-wrap: the text after the break opportunity moves to
                    // the next line, so the current line ends at the break.
                    let moved_width = (current_width - break_end).max(0.0);
                    width = width.max(break_end);
                    line_widths.push(break_end);
                    current_width = moved_width;
                    line_count += 1;
                    last_break_end = None;
                } else {
                    // No break opportunity — fall back to character wrapping.
                    width = width.max(current_width);
                    line_widths.push(current_width);
                    current_width = 0.0;
                    line_count += 1;
                }
            }
            current_width += glyph_width;
        }

        width = width.max(current_width);
        line_widths.push(current_width);
        rasterizer.end_script_run();

        // Subtract one line_gap: it only appears *between* lines, not after
        // the last one.  This matches the corrected layout_paragraph height.
        let metrics = TextMetrics {
            width,
            height: line_count as f32 * line_height - line_gap,
            ascent,
            descent,
            line_gap,
            line_height,
            line_count,
        };

        let key = TextMetricsKey {
            text: text.to_string(),
            font_size_tenths,
            max_width_tenths,
            font_family,
            font_style,
            font_weight,
            language,
            font_revision,
        };
        self.metrics_cache.borrow_mut().insert(
            key,
            CachedTextMetrics {
                metrics,
                line_widths,
            },
        );
        metrics
    }

    /// Measures the rendered width of each line after applying the same
    /// wrapping rules as drawing.
    #[allow(clippy::too_many_arguments)]
    pub fn measure_text_line_widths_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Vec<f32> {
        let lookup = TextMetricsKeyRef {
            text,
            font_size_tenths: (font_size * 10.0) as u32,
            max_width_tenths: (max_width.max(0.0) * 10.0) as u32,
            font_family,
            font_style,
            font_weight,
            language: self.text_language(),
            font_revision: crate::font::FontRegistry::revision(),
        };
        self.measure_text_metrics_styled(
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        );
        self.metrics_cache
            .borrow_mut()
            .get_by(&lookup, TextMetricsKey::matches)
            .map(|cached| cached.line_widths.clone())
            .unwrap_or_default()
    }

    pub fn get_transform_translation(&self) -> (f32, f32) {
        let transform = self.draw_list.borrow();
        let t = transform.current_transform();
        (t.cols[2][0], t.cols[2][1])
    }

    /// Returns the current local-to-physical canvas transform.
    #[inline]
    pub fn get_transform(&self) -> Mat3 {
        *self.draw_list.borrow().current_transform()
    }

    /// Declares the language subsequent text is written in.
    ///
    /// Han is unified: `你好` is covered by a Japanese face as readily as by a
    /// Chinese one, so a run of ideographs alone cannot say which face it
    /// wants, and it keeps whichever the platform's cascade prefers until a
    /// character only one language writes joins it — at which point the whole
    /// word changes typeface. A producer that knows the language says so once
    /// here, and every text drawn *and measured* afterwards is resolved in it.
    ///
    /// The setting is canvas state, like [`set_italic`](Self::set_italic):
    /// pass `None` to restore the default, where a run is judged on its own
    /// characters.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use aimer_cupid::canvas::CupidCanvas;
    /// # use aimer_cupid::font::TextLanguage;
    /// # let canvas = CupidCanvas::new();
    /// canvas.set_text_language(Some(TextLanguage::Chinese));
    /// let width = canvas.measure_text("你好", 16.0);
    /// canvas.set_text_language(None);
    /// # let _ = width;
    /// ```
    pub fn set_text_language(&self, language: Option<TextLanguage>) {
        self.text_language.set(language);
        self.draw_list.borrow_mut().set_text_language(language);
    }

    /// The language declared by [`set_text_language`](Self::set_text_language).
    #[inline]
    pub fn text_language(&self) -> Option<TextLanguage> {
        self.text_language.get()
    }

    pub fn load_image(&self, bytes: &[u8], width: u32, height: u32) -> TextureId {
        self.draw_list.borrow_mut().load_image(bytes, width, height)
    }

    pub fn load_image_with_id(&self, texture_id: TextureId, bytes: &[u8], width: u32, height: u32) {
        self.draw_list
            .borrow_mut()
            .load_image_with_id(texture_id, bytes, width, height)
    }

    pub fn remove_texture(&self, texture_id: TextureId) {
        self.draw_list.borrow_mut().remove_texture(texture_id);
    }

    pub fn set_texture_size(&self, texture_id: TextureId, width: u32, height: u32) {
        self.draw_list
            .borrow_mut()
            .set_texture_size(texture_id, width, height);
    }

    pub fn draw_list(&self) -> Ref<'_, DrawList> {
        self.draw_list.borrow()
    }

    /// Returns this frame's text metrics cache hits and misses.
    ///
    /// The counters are reset by [`Self::begin_frame`]. Release builds return
    /// zeroes so profiling support does not add state or synchronization to
    /// the production canvas.
    #[inline]
    pub fn text_cache_stats(&self) -> (u64, u64) {
        #[cfg(debug_assertions)]
        {
            return (
                self.metrics_cache_hits.get(),
                self.metrics_cache_misses.get(),
            );
        }
        #[cfg(not(debug_assertions))]
        (0, 0)
    }

    pub fn get_image_size(&self, texture_id: TextureId) -> Option<(u32, u32)> {
        self.draw_list.borrow().get_texture_size(texture_id)
    }

    /// Returns the generation of renderer-side image-cache changes.
    #[inline]
    pub fn texture_cache_epoch(&self) -> u64 {
        self.draw_list.borrow().texture_cache_epoch()
    }

    /// Returns whether an image ID still refers to available canvas image
    /// metadata. The renderer marks automatically evicted textures unavailable
    /// while retaining their dimensions so source providers can reload them.
    #[inline]
    pub fn is_texture_available(&self, texture_id: TextureId) -> bool {
        self.draw_list.borrow().is_texture_available(texture_id)
    }
}

impl Default for CupidCanvas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod family_metrics_tests {
    use std::rc::Rc;

    use super::CupidCanvas;
    use crate::font::{FontFamily, FontStyle, FontWeight};

    #[test]
    fn metrics_cache_isolated_by_selected_family() {
        let canvas = CupidCanvas::new();
        let sans = canvas.measure_text_metrics_styled(
            "Mi",
            20.0,
            0.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );
        let mono = canvas.measure_text_metrics_styled(
            "Mi",
            20.0,
            0.0,
            FontFamily::MONOSPACE,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );

        assert_ne!(sans.width, mono.width);
        assert_eq!(canvas.metrics_cache.borrow().len(), 2);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn metrics_cache_records_hits_and_misses() {
        let canvas = CupidCanvas::new();

        assert_eq!(canvas.text_cache_stats(), (0, 0));
        canvas.measure_text_metrics("cached", 16.0, 200.0);
        assert_eq!(canvas.text_cache_stats(), (0, 1));

        canvas.measure_text_metrics("cached", 16.0, 200.0);
        assert_eq!(canvas.text_cache_stats(), (1, 1));
    }

    #[test]
    #[cfg(debug_assertions)]
    fn unwrapped_width_measurements_reuse_metrics_cache() {
        let canvas = CupidCanvas::new();

        canvas.measure_text_styled(
            "cached width",
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );
        assert_eq!(canvas.text_cache_stats(), (0, 1));

        canvas.measure_text_styled(
            "cached width",
            16.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );
        assert_eq!(canvas.text_cache_stats(), (1, 1));
    }

    #[test]
    fn styled_layout_reuses_shaping_for_a_different_width() {
        let canvas = CupidCanvas::new();
        canvas.rasterizer.borrow_mut().reset_shape_call_count();

        let first = canvas.layout_text_styled(
            "cached paragraph",
            16.0,
            200.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );
        let first_shape_calls = canvas.rasterizer.borrow().shape_call_count();

        let second = canvas.layout_text_styled(
            "cached paragraph",
            16.0,
            120.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );

        assert_eq!(first_shape_calls, 1);
        assert_eq!(
            canvas.rasterizer.borrow().shape_call_count(),
            first_shape_calls
        );
        assert_ne!(first.metrics.line_count, second.metrics.line_count);
    }

    #[test]
    fn unwrapped_styled_layout_reuses_shared_interaction_geometry() {
        let canvas = CupidCanvas::new();
        let first = canvas.layout_text_styled_shared(
            "cached unwrapped run",
            16.0,
            0.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );
        let second = canvas.layout_text_styled_shared(
            "cached unwrapped run",
            16.0,
            0.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );

        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn wrapped_line_widths_preserve_short_final_line() {
        let canvas = CupidCanvas::new();
        let widths = canvas.measure_text_line_widths_styled(
            "MMMM i",
            13.0,
            45.0,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );

        assert_eq!(widths.len(), 2);
        assert!(widths[1] < widths[0]);
    }

    #[test]
    fn measured_cjk_lines_fill_the_available_width() {
        // Chinese carries no spaces, so measuring must break between
        // ideographs.  Rewinding to the last Latin space instead would report
        // a half-empty line — and, being one line too many, an inflated
        // height for the widget that owns the text.
        let canvas = CupidCanvas::new();
        let font_size = 16.0;
        let max_width = 240.0;

        let widths = canvas.measure_text_line_widths_styled(
            "「Hello, World!」（世界你好！）之類字串的電腦程式在大多数通用编程语言中",
            font_size,
            max_width,
            FontFamily::SANS_SERIF,
            FontStyle::Normal,
            FontWeight::Normal.numeric(),
        );

        assert!(widths.len() > 1, "the sample must wrap at {max_width}px");
        for (index, width) in widths.iter().enumerate().take(widths.len() - 1) {
            assert!(
                *width > max_width - font_size * 1.5,
                "line {index} stopped at {width}px of {max_width}px"
            );
        }
    }
}
